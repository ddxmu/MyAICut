use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use craft_update::http::sha256_file;
use serde::Deserialize;
use zip::result::ZipError;

use super::{path_str, run};

const EXECUTABLE: &str = "Contents/MacOS/MyAICut";
const MAX_BUNDLE_SIZE: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST_SIZE: u64 = 128 * 1024;

#[derive(Deserialize)]
struct Manifest {
    base_sha256: String,
    files: Vec<String>,
}

pub(super) fn base_hash(bundle: &Path) -> Result<String, String> {
    sha256_file(&bundle.join(EXECUTABLE))
}

fn valid_path(path: &str) -> bool {
    path.len() <= 240
        && path.starts_with("Contents/")
        && path != EXECUTABLE
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|component| !matches!(component, "" | "." | ".."))
}

fn patch_size(path: &Path) -> Result<u64, String> {
    let mut header = [0; 32];
    File::open(path).and_then(|mut file| file.read_exact(&mut header)).map_err(|e| e.to_string())?;
    let [magic, _, _, size] = header.as_chunks::<8>().0 else {
        return Err("Invalid delta patch header.".into());
    };
    let size = u64::from_le_bytes(*size);
    if magic != b"BSDIFF40" || !(1..=MAX_BUNDLE_SIZE).contains(&size) {
        return Err("Invalid delta patch size.".into());
    }
    Ok(size)
}

fn write_file(input: impl Read, destination: &Path, size: u64, mode: u32, remaining: &mut u64) -> Result<(), String> {
    *remaining = remaining.checked_sub(size).ok_or("Delta bundle is too large.")?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(destination).map_err(|e| e.to_string())?;
    let copied = io::copy(&mut input.take(size + 1), &mut output).map_err(|e| e.to_string())?;
    if copied != size {
        return Err("A delta file has the wrong size.".into());
    }
    output.set_permissions(fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())
}

pub(super) fn apply(archive: &Path, base: &Path, stage: &Path, base_sha256: &str) -> Result<(), String> {
    let archive = File::open(archive).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(archive).map_err(|e| e.to_string())?;
    if archive.len() > 514 {
        return Err("Delta archive has too many entries.".into());
    }
    let manifest: Manifest = {
        let manifest = archive.by_name("manifest.json").map_err(|e| e.to_string())?;
        if manifest.size() > MAX_MANIFEST_SIZE {
            return Err("Delta manifest is too large.".into());
        }
        serde_json::from_reader(manifest.take(MAX_MANIFEST_SIZE + 1)).map_err(|e| e.to_string())?
    };
    if manifest.base_sha256 != base_sha256 || manifest.files.len() > 512 {
        return Err("Delta does not match this MyAICut installation.".into());
    }
    let patch = stage.with_extension("bsdiff");
    {
        let entry = archive.by_name("executable.bsdiff").map_err(|e| e.to_string())?;
        if !entry.is_file() {
            return Err("Invalid executable patch entry.".into());
        }
        let size = entry.size();
        let mut limit = MAX_BUNDLE_SIZE;
        write_file(entry, &patch, size, 0o600, &mut limit)?;
    }
    let expected = patch_size(&patch)?;
    let mut remaining = MAX_BUNDLE_SIZE - expected;
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    for path in &manifest.files {
        if !valid_path(path) {
            return Err("Invalid file path in delta manifest.".into());
        }
        let destination = stage.join(path);
        let parent = destination.parent().ok_or("Invalid delta file path.")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        match archive.by_name(&format!("files/{path}")) {
            Ok(entry) if entry.is_file() => {
                let size = entry.size();
                let mode = 0o644 | (entry.unix_mode().unwrap_or(0) & 0o111);
                write_file(entry, &destination, size, mode, &mut remaining)?;
            }
            Err(ZipError::FileNotFound) => {
                let source = base.join(path).canonicalize().map_err(|e| e.to_string())?;
                if !source.starts_with(&base) {
                    return Err("A delta source file is outside the app.".into());
                }
                let source = File::open(source).map_err(|e| e.to_string())?;
                let meta = source.metadata().map_err(|e| e.to_string())?;
                if !meta.is_file() {
                    return Err("A delta source is not a regular file.".into());
                }
                write_file(source, &destination, meta.len(), 0o644 | (meta.permissions().mode() & 0o111), &mut remaining)?;
            }
            _ => return Err("Invalid file in delta archive.".into()),
        }
    }
    let executable = stage.join(EXECUTABLE);
    fs::create_dir_all(executable.parent().ok_or("Missing executable folder.")?).map_err(|e| e.to_string())?;
    run(Path::new("/usr/bin/bspatch"), &[path_str(&base.join(EXECUTABLE))?, path_str(&executable)?, path_str(&patch)?])?;
    if fs::metadata(&executable).map_err(|e| e.to_string())?.len() != expected {
        return Err("The patched executable has the wrong size.".into());
    }
    fs::set_permissions(executable, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    Ok(())
}
