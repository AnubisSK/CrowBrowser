#!/bin/sh
# Builds (release) and installs Crow for the current user: binary, .desktop entry, icon.
# Usage: ./scripts/install_linux.sh [--skip-build]   Uninstall: ./scripts/install_linux.sh --uninstall
set -e
cd "$(dirname "$0")/.."
ID=com.crow.browser
BIN="$HOME/.local/bin/crow-browser"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
DESKTOP="$DATA/applications/$ID.desktop"
ICON="$DATA/icons/hicolor/256x256/apps/$ID.png"

if [ "$1" = "--uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    echo "Odinštalované (trezor v $DATA/crow zostal)."
    exit 0
fi
[ "$1" = "--skip-build" ] || cargo build --release

install -Dm755 target/release/crow-browser "$BIN"
install -Dm644 assets/icon-256.png "$ICON"
mkdir -p "$(dirname "$DESKTOP")"
cat > "$DESKTOP" <<EOF
[Desktop Entry]
Type=Application
Name=Crow
GenericName=Webový prehliadač
Comment=Ľahký, súkromný a šifrovaný webový prehliadač
Exec=$BIN
Icon=$ID
Terminal=false
Categories=Network;WebBrowser;
StartupWMClass=$ID
EOF
command -v update-desktop-database >/dev/null && update-desktop-database "$DATA/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$DATA/icons/hicolor" 2>/dev/null || true
echo "Nainštalované: $BIN"
