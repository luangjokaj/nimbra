#!/usr/bin/env bash
# Signs a binary or .app so its identity stays the same across rebuilds.
# Uses NIMBRA_SIGN_IDENTITY if set, else the first Developer ID or Apple Development
# certificate in the keychain. Without one it leaves the build unsigned and says so.
set -euo pipefail
TARGET="$1"
IDENTITY="${NIMBRA_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  CERTS="$(security find-identity -v -p codesigning)"
  # Prefer Developer ID (the one used for distribution), else a development certificate.
  for KIND in "Developer ID Application" "Apple Development"; do
    IDENTITY="$(printf '%s\n' "$CERTS" | sed -n "s/.*\"\($KIND: [^\"]*\)\".*/\1/p" | head -1)"
    [ -n "$IDENTITY" ] && break
  done
fi
if [ -z "$IDENTITY" ]; then
  echo "No signing certificate found; leaving $TARGET unsigned (Keychain will ask on every rebuild)." >&2
  exit 0
fi
# The hardened runtime stops other programs from loading code into Nimbra, which would
# then read its keychain items as Nimbra. A secure timestamp (which needs the network) is
# what notarization asks of a Developer ID signature.
case "$IDENTITY" in
  "Developer ID Application:"*) TIMESTAMP="--timestamp" ;;
  *) TIMESTAMP="--timestamp=none" ;;
esac
codesign --force --options runtime "$TIMESTAMP" --sign "$IDENTITY" \
  --identifier com.riangle.nimbra "$TARGET"
echo "Signed $TARGET as: $IDENTITY"
