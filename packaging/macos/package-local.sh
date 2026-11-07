#!/usr/bin/env bash
# Build MyAICut.app and MyAICut.DMG from an already-built native binary.
# Ad-hoc signed for local use; not notarized.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
SHORT_VERSION="${VERSION%%-*}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
ARCH="${MYAICUT_ARCH:-aarch64}"
WORK="$TARGET_DIR/macos-package"
APP="$WORK/MyAICut.app"
DIST="${1:-$ROOT/dist}"
DMG="$DIST/MyAICut.DMG"
VERSIONED_DMG="$DIST/myaicut-$VERSION-macos-$ARCH.dmg"
UPDATE_ZIP="$DIST/myaicut-$VERSION-macos-$ARCH-update.zip"
SUMS="$DIST/SHA256SUMS.txt"

rm -rf "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DIST"

cp "$TARGET_DIR/release/filmcraft" "$APP/Contents/MacOS/MyAICut"
cp "$ROOT/assets/app-icon/filmcraft.icns" "$APP/Contents/Resources/MyAICut.icns"
cp "$ROOT/NOTICE" "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$ROOT/FORK_NOTICE.md" \
  "$ROOT/ATTRIBUTION.md" "$APP/Contents/Resources/"
cp "$ROOT/docs/brand/LICENSE-brand.txt" "$APP/Contents/Resources/LICENSE-brand.txt"

sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHORT_VERSION@/$SHORT_VERSION/g" \
  -e "s/@BUILD_SHA@/${MYAICUT_BUILD_SHA:-local}/g" \
  "$ROOT/packaging/macos/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

sign() {
  codesign --force --sign - --timestamp=none --options runtime \
    --entitlements "$ROOT/packaging/macos/entitlements.plist" "$@"
}
sign "$APP/Contents/MacOS/MyAICut"
sign "$APP"
codesign --verify --strict --deep --verbose=2 "$APP"

STAGE="$WORK/dmg"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/MyAICut.app"
ln -s /Applications "$STAGE/Applications"

hdiutil makehybrid -hfs -hfs-volume-name "MyAICut" -hfs-openfolder "$STAGE" \
  -o "$WORK/raw.dmg" "$STAGE" >/dev/null
hdiutil convert "$WORK/raw.dmg" -format UDZO -imagekey zlib-level=9 \
  -o "$WORK/MyAICut.dmg" >"$WORK/convert.log" 2>&1
python3 - "$WORK/MyAICut.dmg" <<'PY'
import sys
b = open(sys.argv[1], "rb").read()
assert b[-512:-508] == b"koly", "DMG has no koly trailer"
print("UDIF OK:", len(b), "bytes")
PY
mv -f "$WORK/MyAICut.dmg" "$VERSIONED_DMG"
codesign --force --sign - --timestamp=none "$VERSIONED_DMG"
codesign --verify --strict --verbose=2 "$VERSIONED_DMG"
cp "$VERSIONED_DMG" "$DMG"

ditto -c -k --keepParent "$APP" "$UPDATE_ZIP"
(
  cd "$DIST"
  shasum -a 256 "$(basename "$VERSIONED_DMG")" "$(basename "$UPDATE_ZIP")" >"$(basename "$SUMS")"
)

echo "APP=$APP"
echo "DMG=$DMG"
echo "VERSIONED_DMG=$VERSIONED_DMG"
echo "UPDATE_ZIP=$UPDATE_ZIP"
echo "SHA256SUMS=$SUMS"
ls -lh "$DMG" "$VERSIONED_DMG" "$UPDATE_ZIP" "$SUMS"
