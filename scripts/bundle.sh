#!/bin/sh
# Builds target/Crow.app (release) with icon. Usage: ./scripts/bundle.sh [binary_path]
set -e
cd "$(dirname "$0")/.."

BIN="$1"
if [ -z "$BIN" ]; then
  if [ -f "target/release/CrowBrowser-universal" ]; then
    BIN="target/release/CrowBrowser-universal"
  else
    export PATH="$HOME/.cargo/bin:$PATH"
    MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release
    BIN=target/release/crow-browser
  fi
fi

APP=target/Crow.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Crow"
cp assets/Crow.icns "$APP/Contents/Resources/Crow.icns"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>CrowBrowser</string>
  <key>CFBundleDisplayName</key><string>CrowBrowser</string>
  <key>CFBundleIdentifier</key><string>com.crow.browser</string>
  <key>CFBundleExecutable</key><string>Crow</string>
  <key>CFBundleIconFile</key><string>Crow</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleSupportedPlatforms</key>
  <array>
    <string>MacOSX</string>
  </array>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSArchitecturePriority</key>
  <array>
    <string>arm64</string>
    <string>x86_64</string>
  </array>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>Web site URL</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>http</string>
        <string>https</string>
      </array>
    </dict>
  </array>
  <key>NSCameraUsageDescription</key>
  <string>CrowBrowser requires access to the camera for WebRTC video conferencing.</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>CrowBrowser requires access to the microphone for WebRTC audio calls.</string>
</dict>
</plist>
EOF

# Code signature with Hardened Runtime and Entitlements (ad-hoc or developer cert)
ENTITLEMENTS="assets/Crow.entitlements"
if [ -f "$ENTITLEMENTS" ]; then
  codesign --force --deep --options runtime --entitlements "$ENTITLEMENTS" --sign - "$APP"
else
  codesign --force --deep --sign - "$APP"
fi

echo "Hotovo: $APP  (presuň do /Applications alebo spusti: open $APP)"
