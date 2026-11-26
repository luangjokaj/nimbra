#!/usr/bin/env bash
# Packs Nimbra.app into a disk image that opens on the Inbox zero sky, with the app and an
# Applications shortcut to drag it onto (assets/dmg/background.png and @2x).
# Usage: scripts/dmg-macos.sh <Nimbra.app> <out.dmg>
# Uses dmgbuild, kept in its own Python environment in ~/.cache/nimbra (made on first use),
# which writes the Finder layout itself, so nothing has to drive Finder.
set -euo pipefail
cd "$(dirname "$0")/.."
APP="$1"
OUT="$2"

VENV="${HOME}/.cache/nimbra/dmgbuild-venv"
if [ ! -x "$VENV/bin/dmgbuild" ]; then
  echo "Setting up dmgbuild in $VENV (once)…"
  python3 -m venv "$VENV"
  "$VENV/bin/pip" install --quiet "dmgbuild>=1.6,<2"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
# One image with both sizes, so Retina screens get the sharp one.
tiffutil -cathidpicheck assets/dmg/background.png assets/dmg/background@2x.png \
  -out "$WORK/background.tiff" >/dev/null 2>&1

ICON="$APP/Contents/Resources/AppIcon.icns"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
"$VENV/bin/dmgbuild" -s scripts/dmg-settings.py \
  -D app="$APP" -D background="$WORK/background.tiff" \
  ${ICON:+-D volume_icon="$ICON"} \
  "Nimbra" "$OUT" >/dev/null
echo "Packed $OUT"
