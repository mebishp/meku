#!/usr/bin/env bash
# Build Meku.app (Apple silicon) and a .dmg and .zip from target/release/meku.
# Run on a Mac after `cargo build --release -p meku`. Output goes to dist/.
set -euo pipefail

cd "$(dirname "$0")/.."
VER=$(grep -m1 '^version' crates/meku/Cargo.toml | cut -d'"' -f2)
BIN=target/release/meku
OUT=dist
APP="$OUT/Meku.app"

[ -x "$BIN" ] || { echo "missing $BIN" >&2; exit 1; }
rm -rf "$APP" "$OUT"/Meku-"$VER"-macos-arm64.*
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/meku"

# Icon: a .icns from the 1024px PNG.
ICONSET=$(mktemp -d)/Meku.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z $s $s packaging/meku-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) packaging/meku-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Meku.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Meku</string>
  <key>CFBundleDisplayName</key><string>Meku</string>
  <key>CFBundleIdentifier</key><string>io.github.mebishp.meku</string>
  <key>CFBundleExecutable</key><string>meku</string>
  <key>CFBundleIconFile</key><string>Meku</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature (no Apple Developer ID yet): Apple silicon refuses unsigned code.
codesign --force --deep --sign - "$APP"

ditto -c -k --keepParent "$APP" "$OUT/Meku-$VER-macos-arm64.zip"
DMG_DIR=$(mktemp -d)
cp -R "$APP" "$DMG_DIR/"
ln -s /Applications "$DMG_DIR/Applications"
hdiutil create -volname "Meku $VER" -srcfolder "$DMG_DIR" -ov -format UDZO "$OUT/Meku-$VER-macos-arm64.dmg"
ls -1 "$OUT" | grep macos
