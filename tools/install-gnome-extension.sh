#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UUID="devflow-recorder@local"
SOURCE_DIR="$ROOT_DIR/gnome-extension/$UUID"
TARGET_DIR="$HOME/.local/share/gnome-shell/extensions/$UUID"

mkdir -p "$TARGET_DIR"
cp "$SOURCE_DIR/metadata.json" "$SOURCE_DIR/extension.js" "$SOURCE_DIR/stylesheet.css" "$TARGET_DIR/"

echo "Installed $UUID to $TARGET_DIR"
echo "Enable it with:"
echo "  gnome-extensions enable $UUID"
echo ""
echo "On GNOME Shell 42, if enabling fails before a relogin, press Alt+F2, type r, then Enter on X11; on Wayland, log out and back in."
echo "If you updated extension.js while logged in and the app still shows no active window, log out and back in so GNOME reloads the extension code."
