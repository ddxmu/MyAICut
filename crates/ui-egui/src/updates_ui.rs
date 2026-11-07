//! MyAICut updates from GitHub: check, verified incremental package, install and restart.
//!
//! The release feed is `ddxmu/MyAICut` on GitHub. Each release carries an update ZIP
//! (`myaicut-<version>-macos-<arch>-update.zip`) and `SHA256SUMS.txt`, so the app downloads only
//! the application bundle and never needs the source or the full disk image.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, TryRecvError, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use craft_update::{Http, Release};

use crate::FilmcraftApp;

/// Public GitHub repository that owns the release feed.
pub const REPO: &str = "ddxmu/MyAICut";
/// The running version, also shown by Help ▸ About.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const BUNDLE_ID: &str = "com.myaicut.app";

pub(crate) struct Runtime {
    #[cfg(not(target_arch = "wasm32"))]
    native: Native,
    pub(crate) manual_window_open: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Native {
    startup_check: bool,
    started: bool,
    check_requested: bool,
    check_is_manual: bool,
    check: Option<Receiver<Result<Option<Release>, String>>>,
    release: Option<Release>,
    install: Option<Receiver<InstallEvent>>,
    prepared: Option<Prepared>,
    progress: Option<(u64, u64)>,
    error: Option<String>,
    up_to_date: bool,
    installing: bool,
    open_window: bool,
    repaint: Option<egui::Context>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            native: Native::default(),
            manual_window_open: false,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Prepared {
    staged: PathBuf,
    staging_root: PathBuf,
    archive: PathBuf,
    version: String,
}

#[cfg(not(target_arch = "wasm32"))]
enum InstallEvent {
    Progress(u64, u64),
    Prepared(Prepared),
    Failed(String),
}

impl FilmcraftApp {
    /// Whether the app asks GitHub for a newer release when it starts.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_startup_update_check(&mut self, on: bool) {
        self.updates_rt.native.startup_check = on;
    }
}

pub(crate) fn check(app: &mut FilmcraftApp) -> Result<serde_json::Value, String> {
    app.updates_rt.manual_window_open = true;
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.updates_rt.native.check_requested = true;
        if app.updates_rt.native.check.is_some() {
            app.updates_rt.native.check_is_manual = true;
        }
        if let Some(ctx) = &app.updates_rt.native.repaint {
            ctx.request_repaint();
        }
    }
    Ok(serde_json::Value::Null)
}

pub(crate) fn tick(app: &mut FilmcraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let updates = &mut app.updates_rt;
        let rt = &mut updates.native;
        rt.repaint = Some(ctx.clone());

        if rt.check_requested {
            rt.check_requested = false;
            updates.manual_window_open = true;
            rt.check_is_manual = true;
            if rt.check.is_none() {
                begin_check(rt, ctx);
            }
        } else if rt.startup_check && !rt.started {
            rt.startup_check = false;
            rt.started = true;
            begin_check(rt, ctx);
        }

        poll_check(rt, ctx);
        poll_install(rt, ctx);
        if std::mem::take(&mut rt.open_window) {
            updates.manual_window_open = true;
        }

        // The package is already downloaded and checked. It replaces the app only when there is
        // no unsaved work, so closing for the restart can never discard an edit.
        if !app.session.is_dirty() {
            if let Some(prepared) = rt.prepared.take() {
                match activate(&prepared) {
                    Ok(()) => {
                        updates.manual_window_open = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(error) => {
                        rt.error = Some(crate::i18n::fmt(crate::i18n::t("The update could not be installed:\n{error}"), &[("error", &error)]));
                        let _ = std::fs::remove_dir_all(&prepared.staging_root);
                    }
                }
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

pub(crate) fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    #[cfg(target_arch = "wasm32")]
    {
        if app.updates_rt.manual_window_open {
            let mut open = true;
            egui::Window::new(crate::i18n::t("MyAICut Updates"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    ui.label(crate::i18n::t("Update check is not supported in the web browser."));
                    ui.add_space(8.0);
                    if ui.button(crate::i18n::t("OK")).clicked() {
                        app.updates_rt.manual_window_open = false;
                    }
                });
            if !open {
                app.updates_rt.manual_window_open = false;
            }
        }
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        if !app.updates_rt.manual_window_open {
            return;
        }

        let clean = !app.session.is_dirty();
        let can_update = current_bundle().is_ok();
        let mut open = true;
        let mut close = false;
        let mut start = false;
        let mut check_again = false;
        let mut open_release = None;
        let mut elems: Vec<(&str, egui::Rect, String)> = Vec::new();

        egui::Window::new(crate::i18n::t("MyAICut Updates"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let rt = &app.updates_rt.native;
                ui.set_max_width(440.0);
                ui.label(egui::RichText::new(crate::i18n::fmt(crate::i18n::t("Current version: {version}"), &[("version", CURRENT_VERSION)])).small().weak());
                ui.add_space(6.0);

                if let Some(error) = &rt.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let again = ui.button(crate::i18n::t("Check Again"));
                        elems.push(("updates.checkAgain", again.rect, crate::i18n::t("Check Again").to_string()));
                        check_again = again.clicked();
                        let close_button = ui.button(crate::i18n::t("Close"));
                        elems.push(("updates.close", close_button.rect, crate::i18n::t("Close").to_string()));
                        if close_button.clicked() {
                            close = true;
                        }
                    });
                } else if let Some(release) = &rt.release {
                    ui.label(
                        egui::RichText::new(crate::i18n::fmt(crate::i18n::t("MyAICut {version} is available."), &[("version", &release.version_label())]))
                            .strong()
                            .size(15.0),
                    );
                    ui.add_space(6.0);

                    if rt.installing {
                        let (done, total) = rt.progress.unwrap_or((0, 0));
                        let fraction = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                        ui.add(egui::ProgressBar::new(fraction).show_percentage());
                        ui.label(crate::i18n::t("Downloading update…"));
                    } else if rt.prepared.is_some() {
                        ui.spinner();
                        ui.label(crate::i18n::t("The update is ready. Save or close your open work to finish."));
                    } else if !can_update {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            crate::i18n::t("This copy can't update itself. Move MyAICut.app to Applications and try again."),
                        );
                    } else if !clean {
                        ui.colored_label(ui.visuals().warn_fg_color, crate::i18n::t("Save or close your open work first; the update restarts the app."));
                    }

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let later = ui.button(crate::i18n::t("Later"));
                        elems.push(("updates.later", later.rect, crate::i18n::t("Later").to_string()));
                        if later.clicked() {
                            close = true;
                        }
                        let download = ui.button(crate::i18n::t("Download from GitHub"));
                        elems.push(("updates.download", download.rect, crate::i18n::t("Download from GitHub").to_string()));
                        if download.clicked() {
                            open_release = Some(release.html_url.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let enabled = clean && can_update && !rt.installing && rt.prepared.is_none();
                            let install = ui.add_enabled(enabled, egui::Button::new(crate::i18n::t("Update and Restart")));
                            elems.push(("updates.install", install.rect, crate::i18n::t("Update and Restart").to_string()));
                            if install.clicked() {
                                start = true;
                            }
                        });
                    });
                } else if rt.up_to_date {
                    ui.label(crate::i18n::fmt(crate::i18n::t("MyAICut {version} is currently the newest version available."), &[("version", CURRENT_VERSION)]));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let again = ui.button(crate::i18n::t("Check Again"));
                        elems.push(("updates.checkAgain", again.rect, crate::i18n::t("Check Again").to_string()));
                        check_again = again.clicked();
                        let close_button = ui.button(crate::i18n::t("Close"));
                        elems.push(("updates.close", close_button.rect, crate::i18n::t("Close").to_string()));
                        if close_button.clicked() {
                            close = true;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(crate::i18n::t("Checking for updates…"));
                    });
                }
            });

        for (id, rect, label) in elems {
            app.auto.add(id, rect, &label);
        }
        if let Some(url) = open_release {
            crate::links::open(app, ctx, &url);
        }
        if check_again {
            app.updates_rt.native.up_to_date = false;
            app.updates_rt.native.error = None;
            app.updates_rt.native.release = None;
            app.updates_rt.native.check_requested = true;
            ctx.request_repaint();
        }
        if start {
            start_install(app, ctx);
        }
        if !open || close {
            app.updates_rt.manual_window_open = false;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn begin_check(rt: &mut Native, ctx: &egui::Context) {
    rt.error = None;
    rt.up_to_date = false;
    rt.release = None;
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("myaicut-update-check".into()).spawn(move || {
        let answer = std::panic::catch_unwind(fetch_latest).unwrap_or_else(|_| Err("The update check failed unexpectedly.".into()));
        let _ = tx.send(answer);
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => rt.check = Some(rx),
        Err(error) => rt.error = Some(format!("Could not start the update check: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn poll_check(rt: &mut Native, ctx: &egui::Context) {
    let outcome = rt.check.as_ref().and_then(|rx| match rx.try_recv() {
        Ok(answer) => Some(Some(answer)),
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(Duration::from_millis(100));
            None
        }
        Err(TryRecvError::Disconnected) => Some(Some(Err("The update check stopped unexpectedly.".into()))),
    });
    let Some(outcome) = outcome else { return };
    rt.check = None;
    let manual = rt.check_is_manual;
    rt.check_is_manual = false;
    match outcome {
        Some(Ok(Some(release))) => {
            rt.release = Some(release);
            rt.open_window = true;
            rt.error = None;
            rt.up_to_date = false;
        }
        Some(Ok(None)) => {
            if manual {
                rt.up_to_date = true;
            }
        }
        Some(Err(error)) => {
            if manual {
                rt.error = Some(crate::i18n::fmt(crate::i18n::t("Could not check for updates:\n{error}"), &[("error", &error)]));
            } else {
                log::warn!("update check failed: {error}");
            }
        }
        None => {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn poll_install(rt: &mut Native, ctx: &egui::Context) {
    loop {
        let event = rt.install.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(Ok(event)),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                None
            }
            Err(TryRecvError::Disconnected) => Some(Err(())),
        });
        let Some(event) = event else { return };
        match event {
            Ok(InstallEvent::Progress(done, total)) => rt.progress = Some((done, total)),
            Ok(InstallEvent::Prepared(prepared)) => {
                rt.installing = false;
                rt.prepared = Some(prepared);
                rt.progress = None;
            }
            Ok(InstallEvent::Failed(error)) => {
                rt.installing = false;
                rt.progress = None;
                rt.error = Some(crate::i18n::fmt(crate::i18n::t("The update could not be installed:\n{error}"), &[("error", &error)]));
            }
            Err(()) => {
                rt.install = None;
                if rt.installing {
                    rt.installing = false;
                    rt.progress = None;
                    rt.error = Some("The update stopped unexpectedly.".into());
                }
                return;
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_install(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(release) = app.updates_rt.native.release.clone() else { return };
    let rt = &mut app.updates_rt.native;
    if rt.installing || rt.prepared.is_some() {
        return;
    }
    if let Err(error) = current_bundle() {
        rt.error = Some(error);
        return;
    }
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name("myaicut-update-install".into()).spawn(move || {
        let result = std::panic::catch_unwind(|| prepare(&release, &tx)).unwrap_or_else(|_| Err("The update failed unexpectedly.".into()));
        if let Err(error) = result {
            let _ = tx.send(InstallEvent::Failed(error));
        }
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => {
            rt.install = Some(rx);
            rt.installing = true;
            rt.progress = Some((0, 0));
            rt.error = None;
        }
        Err(error) => rt.error = Some(format!("Could not start the update: {error}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn fetch_latest() -> Result<Option<Release>, String> {
    let http = Http::new(&format!("MyAICut/{CURRENT_VERSION}"))?;
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let release = match http.get_api(&url, None)? {
        craft_update::http::Fetched::Body { text, .. } => Some(craft_update::parse_github_release(&text, REPO)?),
        craft_update::http::Fetched::NotFound | craft_update::http::Fetched::NotModified => None,
        craft_update::http::Fetched::RateLimited { .. } => return Err("GitHub's request limit was reached. Try again later.".into()),
    };
    let Some(release) = release else { return Ok(None) };
    let running = craft_update::parse_version(CURRENT_VERSION).ok_or_else(|| "This build has no readable version.".to_string())?;
    let newer = release.version.as_ref().is_some_and(|version| craft_update::version::offers_update(version, &running));
    Ok(newer.then_some(release))
}

#[cfg(not(target_arch = "wasm32"))]
fn prepare(release: &Release, events: &std::sync::mpsc::Sender<InstallEvent>) -> Result<(), String> {
    let asset = update_asset(release)?;
    let http = Http::new(&format!("MyAICut/{CURRENT_VERSION}"))?;
    let sha256 = checksum(release, asset, &http)?;
    let archive = update_cache().join(&asset.name);
    let stop = craft_update::Stop::default();
    http.download(&asset.url, &archive, asset.size, &sha256, &stop, &mut |transfer| {
        let done = match transfer {
            craft_update::http::Transfer::Bytes { done, .. } | craft_update::http::Transfer::Reconnecting { done, .. } => done,
        };
        let _ = events.send(InstallEvent::Progress(done, asset.size));
    })?;

    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let staging_root = std::env::temp_dir().join(format!("myaicut-update-{}-{stamp}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging_root);
    std::fs::create_dir_all(&staging_root).map_err(|e| format!("Could not create {} ({e}).", staging_root.display()))?;
    let extracted = run(Path::new("/usr/bin/ditto"), &["-x", "-k", path_str(&archive)?, path_str(&staging_root)?]);
    if let Err(error) = extracted {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(format!("Could not unpack the update ({error})."));
    }
    let staged = staging_root.join("MyAICut.app");
    if let Err(error) = verify_bundle(&staged, &release.version_label()) {
        let _ = std::fs::remove_dir_all(&staging_root);
        return Err(error);
    }
    if events.send(InstallEvent::Prepared(Prepared { staged, staging_root, archive, version: release.version_label() })).is_err() {
        return Err("The update window closed before the package was ready.".into());
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn update_asset(release: &Release) -> Result<&craft_update::Asset, String> {
    let version = release.version.as_ref().ok_or("The release has no version.")?.to_string();
    let prefix = format!("myaicut-{version}-");
    let tokens: &[&str] = match std::env::consts::ARCH {
        "aarch64" => &["macos-universal", "macos-aarch64", "macos-arm64"],
        "x86_64" => &["macos-universal", "macos-x86_64", "macos-x64"],
        _ => &[],
    };
    release
        .assets
        .iter()
        .find(|asset| asset.name.starts_with(&prefix) && tokens.iter().any(|token| asset.name == format!("{prefix}{token}-update.zip")))
        .ok_or_else(|| "This release has no update package for this Mac.".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn checksum(release: &Release, asset: &craft_update::Asset, http: &Http) -> Result<String, String> {
    if let Some(sha256) = &asset.sha256 {
        return Ok(sha256.clone());
    }
    let url = release
        .sums_asset()
        .map(|sums| sums.url.clone())
        .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/download/{}/SHA256SUMS.txt", release.tag));
    let text = http.get_text(&url)?.ok_or("The release has no SHA256SUMS.txt.")?;
    craft_update::release::parse_sha256sums(&text)
        .into_iter()
        .find(|(_, name)| name == &asset.name)
        .map(|(hash, _)| hash)
        .ok_or_else(|| "The release has no checksum for its update package.".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn current_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Could not locate MyAICut ({e})."))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    exe.ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| "This copy of MyAICut can't update itself. Move MyAICut.app to Applications and try again.".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn verify_bundle(bundle: &Path, version: &str) -> Result<(), String> {
    let plist = bundle.join("Contents/Info.plist");
    let id = plist_value(&plist, "CFBundleIdentifier")?;
    if id != BUNDLE_ID {
        return Err(format!("The update is not MyAICut ({id})."));
    }
    let found = plist_value(&plist, "CFBundleShortVersionString")?;
    if found != version {
        return Err(format!("The update reports version {found}, not {version}."));
    }
    run(Path::new("/usr/bin/codesign"), &["--verify", "--strict", "--deep", path_str(bundle)?])?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn plist_value(path: &Path, key: &str) -> Result<String, String> {
    let output = std::process::Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(path)
        .output()
        .map_err(|e| format!("Could not read the update's Info.plist ({e})."))?;
    if !output.status.success() {
        return Err("The update has no readable Info.plist.".into());
    }
    String::from_utf8(output.stdout).map(|text| text.trim().to_string()).map_err(|_| "The update's Info.plist is not text.".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn activate(prepared: &Prepared) -> Result<(), String> {
    let bundle = current_bundle()?;
    let parent = bundle.parent().ok_or("MyAICut.app has no parent folder.")?;
    let pid = std::process::id();
    let next = parent.join(format!(".MyAICut.app.new-{pid}"));
    let old = parent.join(format!(".MyAICut.app.old-{pid}"));
    let _ = std::fs::remove_dir_all(&next);
    let _ = std::fs::remove_dir_all(&old);

    run(Path::new("/usr/bin/ditto"), &["--norsrc", "--noextattr", "--noacl", path_str(&prepared.staged)?, path_str(&next)?])
        .map_err(|e| format!("Could not stage MyAICut.app ({e})."))?;
    verify_bundle(&next, &prepared.version).map_err(|e| {
        let _ = std::fs::remove_dir_all(&next);
        e
    })?;

    std::fs::rename(&bundle, &old).map_err(|e| format!("Could not move the current MyAICut.app aside ({e})."))?;
    if let Err(error) = std::fs::rename(&next, &bundle) {
        let _ = std::fs::rename(&old, &bundle);
        let _ = std::fs::remove_dir_all(&next);
        return Err(format!("Could not put the update in place ({error})."));
    }
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&prepared.staging_root);
    let _ = std::fs::remove_file(&prepared.archive);
    relaunch_after_exit(pid, &bundle)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn relaunch_after_exit(pid: u32, bundle: &Path) -> Result<(), String> {
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open -n \"$2\"")
        .arg("myaicut-update")
        .arg(pid.to_string())
        .arg(bundle)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().map(|_| ()).map_err(|e| format!("The update is installed, but MyAICut could not schedule its restart ({e}). Quit and reopen it."))
}

#[cfg(not(target_arch = "wasm32"))]
fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str().ok_or_else(|| format!("Unusual characters in {}.", path.display()))
}

#[cfg(not(target_arch = "wasm32"))]
fn run(program: &Path, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new(program).args(args).output().map_err(|e| format!("Could not run {} ({e}).", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    Err(format!("{} failed: {}", program.display(), detail.trim()))
}

#[cfg(not(target_arch = "wasm32"))]
fn update_cache() -> PathBuf {
    std::env::temp_dir().join("MyAICut").join("Updates")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_runtime_is_idle() {
        let rt = Runtime::default();
        assert!(!rt.manual_window_open);
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!(!rt.native.startup_check);
            assert!(rt.native.release.is_none());
            assert!(rt.native.install.is_none());
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn picks_the_native_update_package() {
        let release = Release {
            tag: "v9.9.9".into(),
            version: craft_update::parse_version("9.9.9"),
            assets: ["myaicut-9.9.9-macos-x86_64-update.zip", "myaicut-9.9.9-macos-aarch64-update.zip", "myaicut-9.9.9-macos-universal.dmg"]
                .into_iter()
                .map(|name| craft_update::Asset {
                    name: name.into(),
                    size: 1,
                    url: format!("https://github.com/{REPO}/releases/download/v9.9.9/x"),
                    sha256: None,
                })
                .collect(),
            ..Release::default()
        };
        let asset = update_asset(&release).unwrap();
        #[cfg(target_arch = "aarch64")]
        assert!(asset.name.contains("aarch64"));
        #[cfg(target_arch = "x86_64")]
        assert!(asset.name.contains("x86_64"));
    }
}
