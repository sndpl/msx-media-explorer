#!/bin/sh
# Run the dev build as a minimal macOS .app bundle.
#
# A plain `cargo run` is not a bundle, so it gets its Dock icon from
# `NSApplication.setApplicationIconImage` (a raw bitmap) and none of the
# bundle-derived behavior: no icon.icns for the system to mask and theme, no
# Info.plist on disk, no LaunchServices registration. This assembles a minimal
# .app around the release binary, ad-hoc signs it, and opens it, so a dev build
# behaves like the packaged one. Optionally pass a disk/tape image path to open
# on launch.
#
# This is a dev convenience only; real releases are packaged by cargo-packager
# (see README "Packaging a macOS app").
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p mediaexplorer-gui

APP="target/dev-app/MSX Media Explorer.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/mediaexplorer "$APP/Contents/MacOS/"
cp crates/mediaexplorer-gui/assets/icons/icon.icns "$APP/Contents/Resources/"
# Keep the name/identifier in sync with [package.metadata.packager] in
# crates/mediaexplorer-gui/Cargo.toml and INFO_PLIST_XML in src/main.rs.
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>MSX Media Explorer</string>
    <key>CFBundleDisplayName</key><string>MSX Media Explorer</string>
    <key>CFBundleIdentifier</key><string>io.github.sandy.mediaexplorer</string>
    <key>CFBundleExecutable</key><string>mediaexplorer</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"

if [ "$#" -gt 0 ]; then
    open "$APP" --args "$@"
else
    open "$APP"
fi
