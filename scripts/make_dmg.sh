#!/bin/sh
# Builds dist/Crow-<version>.dmg: background banner, drag-to-Applications layout, volume + file icon.
# Usage: ./scripts/make_dmg.sh [--skip-build]
set -e
cd "$(dirname "$0")/.."

[ "$1" = "--skip-build" ] || ./scripts/bundle.sh

NAME=Crow
APP=target/$NAME.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DMG=dist/$NAME-$VERSION.dmg
MNT=/Volumes/$NAME
[ -d "$APP" ] || { echo "Chýba $APP – spusti bez --skip-build"; exit 1; }

STAGE=$(mktemp -d)
TMPDMG=$(mktemp -u).dmg
trap 'rm -rf "$STAGE" "$TMPDMG"' EXIT

cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
mkdir "$STAGE/.background"
cp assets/dmg-background.tiff "$STAGE/.background/background.tiff"

SIZE=$(( $(du -sm "$STAGE" | cut -f1) + 20 ))
[ -d "$MNT" ] && hdiutil detach "$MNT" -force -quiet
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -fs HFS+ -format UDRW -size "${SIZE}m" -ov "$TMPDMG" -quiet
DEV=$(hdiutil attach -readwrite -noverify -noautoopen "$TMPDMG" | awk '/Apple_HFS/ {print $1}')

# Window layout (stored in the volume's .DS_Store). Content area = 660x400 = banner size.
osascript <<EOF || echo "Upozornenie: rozloženie okna sa nenastavilo (povoľ ovládanie Findera v Nastavenia > Súkromie > Automatizácia)."
tell application "Finder"
  tell disk "$NAME"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {200, 120, 860, 542}
    set opts to the icon view options of container window
    set arrangement of opts to not arranged
    set icon size of opts to 112
    set text size of opts to 13
    set background picture of opts to file ".background:background.tiff"
    set position of item "$NAME.app" of container window to {170, 215}
    set position of item "Applications" of container window to {490, 215}
    update without registering applications
    delay 2
    close
  end tell
end tell
EOF

# After Finder: it drops the volume icon when it rewrites .DS_Store
cp assets/Crow.icns "$MNT/.VolumeIcon.icns"
SetFile -a C "$MNT"

sync
rm -rf "$MNT/.fseventsd"
hdiutil detach "$DEV" -quiet || hdiutil detach "$DEV" -force -quiet

mkdir -p dist
rm -f "$DMG"
hdiutil convert "$TMPDMG" -format UDZO -imagekey zlib-level=9 -o "$DMG" -quiet

# Icon of the .dmg file itself (local Finder only; not preserved by downloads)
osascript -l JavaScript -e "ObjC.import('AppKit'); \$.NSWorkspace.sharedWorkspace.setIconForFileOptions(\$.NSImage.alloc.initWithContentsOfFile('$PWD/assets/Crow.icns'), '$PWD/$DMG', 0)" >/dev/null

echo "Hotovo: $DMG ($(du -h "$DMG" | cut -f1))"
