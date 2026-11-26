#!/usr/bin/env bash
# Packs Nimbra.app into a disk image with an Applications shortcut to drag it onto.
# Usage: scripts/dmg-macos.sh <Nimbra.app> <out.dmg>
set -euo pipefail
APP="$1"
OUT="$2"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/Nimbra.app"
ln -s /Applications "$STAGE/Applications"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
hdiutil create -volname "Nimbra" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$OUT" >/dev/null
echo "Packed $OUT"
