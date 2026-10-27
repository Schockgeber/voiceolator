#!/bin/sh
# Install Voiceolator for the current user (no root needed).
#   ./install.sh            install into ~/.local
#   ./install.sh --uninstall
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
DEST="${XDG_DATA_HOME:-$HOME/.local/share}/voiceolator"
BIN="$HOME/.local/bin"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"

if [ "$1" = "--uninstall" ]; then
    rm -rf "$DEST" "$BIN/voiceolator" "$BIN/voiceolator-cli" "$APPS/voiceolator.desktop" "$ICONS/voiceolator.png"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
    echo "Voiceolator removed. (Settings in ~/.config/Voiceolator were kept.)"
    exit 0
fi

mkdir -p "$DEST" "$BIN" "$APPS" "$ICONS"
cp -r "$HERE"/voiceolator "$HERE"/libonnxruntime.so "$HERE"/models "$HERE"/README.html "$HERE"/LICENSE.txt "$HERE"/THIRD_PARTY_LICENSES.txt "$DEST"/
[ -f "$HERE/voiceolator-cli" ] && cp "$HERE/voiceolator-cli" "$DEST/"
chmod +x "$DEST/voiceolator" "$DEST"/voiceolator-cli 2>/dev/null || true
ln -sf "$DEST/voiceolator" "$BIN/voiceolator"
[ -f "$DEST/voiceolator-cli" ] && ln -sf "$DEST/voiceolator-cli" "$BIN/voiceolator-cli"
cp "$HERE/voiceolator.png" "$ICONS/voiceolator.png"
sed "s|^Exec=voiceolator|Exec=$DEST/voiceolator|" "$HERE/voiceolator.desktop" > "$APPS/voiceolator.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APPS" >/dev/null 2>&1 || true
echo "Voiceolator installed. Start it from your application menu or run: voiceolator"
