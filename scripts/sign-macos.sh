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
# The app gets the hardened runtime, needed for notarization (a bare dev binary does not, so a
# debugger can still attach). No entitlements: WebKit runs its JIT in its own processes, and
# nothing here loads other code. A Developer ID signature also gets a secure timestamp, which
# notarization needs too; a disk image takes only the timestamp. A release (NIMBRA_RELEASE=1)
# has checked above that the identity is a Developer ID.
OPTIONS=()
case "$TARGET" in *.app) OPTIONS+=(--options runtime) ;; esac
case "$IDENTITY" in "Developer ID Application"*) OPTIONS+=(--timestamp) ;; esac
codesign --force ${OPTIONS[@]+"${OPTIONS[@]}"} --sign "$IDENTITY" --identifier com.riangle.nimbra "$TARGET"
echo "Signed $TARGET as: $IDENTITY"
