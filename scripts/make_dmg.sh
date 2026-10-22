#!/bin/sh
# Builds dist/Crow-<version>-<arch>.dmg: background banner, drag-to-Applications layout, volume + file icon.
# Usage: ./scripts/make_dmg.sh [arm64|x86_64|universal|all] [--skip-build]
#   arm64      Apple Silicon
#   x86_64     Intel
#   universal  one binary for both (lipo), default
#   all        all three DMGs from one pair of builds
#   --skip-build  reuse binaries already in target/<triple>/release
set -e
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

ARCH=universal
SKIP=
for a in "$@"; do
  case "$a" in
    arm64|x86_64|universal|all) ARCH=$a ;;
    --skip-build) SKIP=1 ;;
    *) echo "Neznámy argument: $a"; exit 1 ;;
  esac
done

NAME=Crow
APP=target/$NAME.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
MNT=/Volumes/$NAME

triple() {
  case "$1" in
    arm64) echo aarch64-apple-darwin ;;
    x86_64) echo x86_64-apple-darwin ;;
  esac
}

# Release binary for one architecture (built unless --skip-build).
bin_for() {
  t=$(triple "$1")
  if [ -z "$SKIP" ]; then
    rustup target list --installed | grep -qx "$t" || rustup target add "$t" >&2
    MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --target "$t" >&2
  fi
  b=target/$t/release/crow-browser
  [ -f "$b" ] || { echo "Chýba $b – spusti bez --skip-build" >&2; exit 1; }
  echo "$b"
}

# Fat binary from both architectures.
universal_bin() {
  arm=$(bin_for arm64) || exit 1
  x86=$(bin_for x86_64) || exit 1
  out=target/universal-apple-darwin/release/crow-browser
  mkdir -p "$(dirname "$out")"
  lipo -create "$arm" "$x86" -output "$out"
  echo "$out"
}

# Bundles `bin` into Crow.app and packs it as dist/Crow-<version>-<arch>.dmg.
make_dmg() (
  arch=$1
  bin=$2
  DMG=dist/$NAME-$VERSION-$arch.dmg
  echo "== $DMG ($(lipo -archs "$bin"))"
  ./scripts/bundle.sh "$bin"

  STAGE=$(mktemp -d)
  TMPDMG=$(mktemp -u).dmg
  trap 'rm -rf "$STAGE" "$TMPDMG"' EXIT

  cp -R "$APP" "$STAGE/"
  ln -s /Applications "$STAGE/Applications"
  mkdir "$STAGE/.background"
  cp assets/dmg-background.tiff "$STAGE/.background/background.tiff"

  SIZE=$(( $(du -sm "$STAGE" | cut -f1) + 20 ))
  if [ -d "$MNT" ]; then hdiutil detach "$MNT" -force -quiet; fi
  hdiutil create -volname "$NAME" -srcfolder "$STAGE" -fs HFS+ -format UDRW -size "${SIZE}m" -ov "$TMPDMG" -quiet
  DEV=$(hdiutil attach -readwrite -noverify -noautoopen "$TMPDMG" | awk '/Apple_HFS/ {print $1}')

  # Finder registers a freshly attached volume with a delay; wait up to ~10 s.
  i=0
  until osascript -e "tell application \"Finder\" to exists disk \"$NAME\"" 2>/dev/null | grep -q true || [ $i -ge 20 ]; do
    sleep 0.5
    i=$((i + 1))
  done

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
)

# Binaries go into variables first: a failed `$(...)` used directly as an argument would not stop `set -e`.
case "$ARCH" in
  arm64|x86_64)
    b=$(bin_for "$ARCH")
    make_dmg "$ARCH" "$b"
    ;;
  universal)
    b=$(universal_bin)
    make_dmg universal "$b"
    ;;
  all)
    arm=$(bin_for arm64)
    x86=$(bin_for x86_64)
    SKIP=1 # both architectures are built by now
    uni=$(universal_bin)
    make_dmg arm64 "$arm"
    make_dmg x86_64 "$x86"
    make_dmg universal "$uni"
    ;;
esac
