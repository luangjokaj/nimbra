#!/usr/bin/env bash
# Signs a binary or .app so its identity stays the same across rebuilds.
# Uses NIMBRA_SIGN_IDENTITY if set, else the first Developer ID or Apple Development
# certificate in the keychain. Without one it leaves the build unsigned and says so; a
# release (NIMBRA_RELEASE=1) needs a Developer ID and stops instead.
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
if [ "${NIMBRA_RELEASE:-}" = 1 ] && [[ "$IDENTITY" != "Developer ID Application:"* ]]; then
  echo "A release needs a Developer ID Application certificate (found: ${IDENTITY:-none})." >&2
  exit 1
fi
if [ -z "$IDENTITY" ]; then
  echo "No signing certificate found; leaving $TARGET unsigned (Keychain will ask on every rebuild)." >&2
  exit 0
fi
# NIMBRA_RELEASE=1 signs for distribution: Apple's notary service needs the hardened
# runtime and a secure timestamp. A disk image takes only the timestamp.
OPTIONS=()
if [ "${NIMBRA_RELEASE:-}" = 1 ]; then
  case "$TARGET" in
    *.dmg) OPTIONS=(--timestamp) ;;
    *) OPTIONS=(--options runtime --timestamp) ;;
  esac
fi
codesign --force ${OPTIONS[@]+"${OPTIONS[@]}"} --sign "$IDENTITY" --identifier com.riangle.nimbra "$TARGET"
echo "Signed $TARGET as: $IDENTITY"
