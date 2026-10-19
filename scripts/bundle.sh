#!/bin/sh
# Builds target/Crow.app (release) with icon. Usage: ./scripts/bundle.sh
set -e
cd "$(dirname "$0")/.."
MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release

APP=target/Crow.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/crow-browser "$APP/Contents/MacOS/Crow"
cp assets/Crow.icns "$APP/Contents/Resources/Crow.icns"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Crow</string>
  <key>CFBundleDisplayName</key><string>Crow</string>
  <key>CFBundleIdentifier</key><string>com.crow.browser</string>
  <key>CFBundleExecutable</key><string>Crow</string>
  <key>CFBundleIconFile</key><string>Crow</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Ad-hoc signature: required for a stable identity (Keychain, WebKit) on the local machine.
codesign --force --deep --sign - "$APP"

echo "Hotovo: $APP  (presuň do /Applications alebo spusti: open $APP)"
