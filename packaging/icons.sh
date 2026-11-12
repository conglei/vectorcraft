#!/usr/bin/env bash
# Regenerate every app icon from assets/app-icon/vectorcraft.svg (the canonical artwork), or
# StitchCraft's from assets/app-icon/stitchcraft.svg.
#
# Needs: resvg (brew install resvg / cargo install resvg). On macOS, iconutil also writes the
# .icns. The outputs are committed, so building and packaging never need these tools.
#
#   packaging/icons.sh               # VectorCraft
#   packaging/icons.sh stitchcraft   # StitchCraft: its runtime icons and its .icns only
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/app-icon"
APP="${1:-vectorcraft}"
case "$APP" in
  vectorcraft | stitchcraft) ;;
  *) echo "error: unknown app $APP (vectorcraft or stitchcraft)" >&2; exit 1 ;;
esac
SVG="$DIR/$APP.svg"
ID="ai.storyteller.$APP"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v resvg >/dev/null || { echo "error: resvg not found (brew install resvg)" >&2; exit 1; }

# The SVG is a full-bleed 512 tile, used as-is on Windows and Linux. macOS icons follow Apple's grid:
# the tile is 824/1024 of the canvas with a transparent margin, so widen the viewBox around it.
MAC="$TMP/mac.svg"
sed 's/viewBox="0 0 512 512"/viewBox="-62.136 -62.136 636.272 636.272"/' "$SVG" >"$MAC"
grep -q 'viewBox="-62.136' "$MAC" || { echo "error: $SVG has no 512 viewBox" >&2; exit 1; }

render() { resvg -w "$2" -h "$2" "$1" "$3" </dev/null; }

render "$SVG" 1024 "$DIR/$APP-1024.png"
# Runtime window/Dock icon on macOS (embedded by apps/vectorcraft/src/titlebar.rs; StitchCraft's
# by apps/stitchcraft/src/main.rs).
render "$MAC" 512 "$DIR/$APP-macos-512.png"

# StitchCraft is packaged for macOS only so far: its runtime icons (the 256 px window icon on
# Windows and Linux, the 128 px brand mark in the app bar and About box) and its .icns.
if [ "$APP" = stitchcraft ]; then
  for s in 128 256; do
    mkdir -p "$DIR/hicolor/${s}x${s}/apps"
    render "$SVG" "$s" "$DIR/hicolor/${s}x${s}/apps/$ID.png"
  done
  command -v iconutil >/dev/null || { echo "warning: iconutil not found (macOS only); $APP.icns not regenerated" >&2; exit 0; }
  SET="$TMP/$APP.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$MAC" "$s" "$SET/icon_${s}x${s}.png"
    render "$MAC" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/$APP.icns" "$SET"
  echo "icons written to $DIR"
  exit 0
fi

# Linux hicolor theme (the 256 one is also the runtime icon on Windows and Linux).
for s in 16 24 32 48 64 128 256 512; do
  mkdir -p "$DIR/hicolor/${s}x${s}/apps"
  render "$SVG" "$s" "$DIR/hicolor/${s}x${s}/apps/$ID.png"
done
mkdir -p "$DIR/hicolor/scalable/apps"
cp "$DIR/vectorcraft-small.svg" "$DIR/hicolor/scalable/apps/$ID.svg"

# Windows .ico.
ICO_PNGS=()
for s in 16 20 24 32 40 48 64 128 256; do
  render "$SVG" "$s" "$TMP/ico-$s.png"
  ICO_PNGS+=("$TMP/ico-$s.png")
done
(cd "$ROOT" && cargo run -q -p xtask -- ico "$DIR/vectorcraft.ico" "${ICO_PNGS[@]}")

# macOS .icns.
if command -v iconutil >/dev/null; then
  SET="$TMP/vectorcraft.iconset"
  mkdir -p "$SET"
  for s in 16 32 128 256 512; do
    render "$MAC" "$s" "$SET/icon_${s}x${s}.png"
    render "$MAC" $((s * 2)) "$SET/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns -o "$DIR/vectorcraft.icns" "$SET"
else
  echo "warning: iconutil not found (macOS only); vectorcraft.icns not regenerated" >&2
fi
echo "icons written to $DIR"
