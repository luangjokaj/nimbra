#!/usr/bin/env bash
# Notarizes a signed Nimbra.app or disk image with Apple and staples the ticket to it,
# so it opens on any Mac without a warning, even offline.
# Uses the notarytool keychain profile in NIMBRA_NOTARY_PROFILE (default: nimbra); make one
# with `xcrun notarytool store-credentials nimbra --apple-id <id> --team-id <team>`.
set -euo pipefail
TARGET="$1"
PROFILE="${NIMBRA_NOTARY_PROFILE:-nimbra}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# An app goes up zipped; a disk image as it is.
case "$TARGET" in
  *.app)
    UPLOAD="$WORK/$(basename "$TARGET" .app).zip"
    ditto -c -k --keepParent "$TARGET" "$UPLOAD"
    ;;
  *) UPLOAD="$TARGET" ;;
esac

echo "Notarizing $(basename "$TARGET") (usually a minute or two)…"
RESULT="$(xcrun notarytool submit "$UPLOAD" --keychain-profile "$PROFILE" --wait --timeout 30m --output-format json)"
STATUS="$(printf '%s' "$RESULT" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("status",""))')"
ID="$(printf '%s' "$RESULT" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id",""))')"
if [ "$STATUS" != "Accepted" ]; then
  echo "Apple did not accept $(basename "$TARGET"): ${STATUS:-no answer}. Its log:" >&2
  [ -n "$ID" ] && xcrun notarytool log "$ID" --keychain-profile "$PROFILE" >&2 || true
  exit 1
fi
xcrun stapler staple "$TARGET" >/dev/null
echo "Notarized and stapled $(basename "$TARGET")"
