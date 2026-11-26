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
# The app gets the hardened runtime, needed for notarization (a bare dev binary does not, so a
# debugger can still attach). No entitlements: WebKit runs its JIT in its own processes, and
# nothing here loads other code. A Developer ID build also gets a secure timestamp.
FLAGS=""
case "$TARGET" in *.app) FLAGS="--options runtime" ;; esac
case "$IDENTITY" in "Developer ID Application"*) FLAGS="$FLAGS --timestamp" ;; esac
# shellcheck disable=SC2086 # FLAGS is a list of flags
codesign --force $FLAGS --sign "$IDENTITY" --identifier com.riangle.nimbra "$TARGET"
echo "Signed $TARGET as: $IDENTITY"
