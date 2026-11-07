# MyAICut

MyAICut is a macOS non-linear video editor built from the open-source
[FilmCraft](https://github.com/storytold/filmcraft) project. It keeps FilmCraft's
pure-Rust editing engine, codecs, GPU compositor and UI, with a new product name,
bundle identifier and icon.

The project is published at [ddxmu/MyAICut](https://github.com/ddxmu/MyAICut).

The interface follows the operating system language by default:

- English system language → English UI
- Chinese system language (Simplified or Traditional) → Simplified Chinese UI
- Japanese system language → Japanese UI
- other languages → the matching FilmCraft catalog, or English when there is none

## Updates

MyAICut checks the latest stable GitHub release at launch. **MyAICut → Check for
Updates…** checks manually. **Update and Restart** downloads the app-only update
ZIP, verifies its SHA-256 checksum, bundle identifier, version and code signature,
then replaces `MyAICut.app` in Applications and restarts the app. Unsaved work
blocks installation so an update cannot discard changes.

Each release carries:

- `myaicut-<version>-macos-aarch64-update.zip` — the incremental app-only package
- `myaicut-<version>-macos-aarch64.dmg` — the full installer
- `SHA256SUMS.txt` — checksums for the app update and installer

## Build (macOS, Apple silicon)

Requirements: Xcode command-line tools and Rust 1.95 or newer.

```sh
cargo build --release --locked -p filmcraft -p filmcraft-cli
```

The app binary is `target/release/filmcraft`; the packaging script assembles it as
`MyAICut.app` and writes the DMG, update ZIP and checksum file:

```sh
bash packaging/macos/package-local.sh
```

Publish the three files from `dist/` as a new GitHub release whose tag is
`v<version>`; the in-app updater discovers it through `releases/latest`.

## Licence and attribution

MyAICut is a modified fork of FilmCraft. FilmCraft is Copyright (c) 2026 ArtCraft
Team and the FilmCraft contributors, dual-licensed under MIT or Apache-2.0. The
original licence texts are in `LICENSE-MIT` and `LICENSE-APACHE`; required notices
are in `NOTICE` and `ATTRIBUTION.md`.

This fork is not affiliated with or endorsed by the ArtCraft Team. The ArtCraft and
FilmCraft names and logos are trademarks of the ArtCraft Team; this fork uses a
different name and original icon artwork. The upstream project's community links
remain in the About window for attribution.
