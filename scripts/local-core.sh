#!/usr/bin/env bash
# Switches the mail engine between its release and a local checkout.
#
#   scripts/local-core.sh on      build against ../core (the nimbra-core repo, cloned next to this one)
#   scripts/local-core.sh off     back to the release tag in Cargo.toml
#   scripts/local-core.sh status  which one is in use
#
# `on` writes .cargo/config.toml (gitignored) with a [patch] that points the git
# dependency at ../core/crates/nimbra-core. Cargo.toml is never touched. While it is on,
# Cargo.lock loses nimbra-core's git source: don't commit it like that (CI refuses it).
# `off` removes the patch and puts the git source back in Cargo.lock.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="$ROOT/.cargo/config.toml"
CORE="$ROOT/../core"
CRATE="$CORE/crates/nimbra-core"
MARKER="# Written by scripts/local-core.sh"
GIT_SOURCE='source = "git+https://github.com/nimbra-mail/core?'

ours() { [[ -f "$CONFIG" && "$(head -n 1 "$CONFIG")" == "$MARKER" ]]; }

# The source line under nimbra-core's entry in Cargo.lock, or nothing for a path dependency.
lock_source() {
  awk '/^name = "nimbra-core"$/ { found = 1; next }
       found && /^source = / { print; exit }
       found && /^(dependencies|checksum) = / { exit }
       found && /^$/ { exit }' "$ROOT/Cargo.lock"
}

# Resolves the dependency graph again so Cargo.lock matches the current setup.
resolve() {
  (cd "$ROOT" && cargo metadata --format-version 1 >/dev/null)
}

case "${1:-}" in
  on)
    if [[ ! -f "$CRATE/Cargo.toml" ]]; then
      echo "No nimbra-core at $CRATE." >&2
      echo "Clone it first: git clone https://github.com/nimbra-mail/core ../core" >&2
      exit 1
    fi
    if [[ -f "$CONFIG" ]] && ! ours; then
      echo "$CONFIG exists and was not written by this script; leaving it alone." >&2
      echo "Add this to it by hand instead:" >&2
      echo '  [patch."https://github.com/nimbra-mail/core"]' >&2
      echo '  nimbra-core = { path = "../core/crates/nimbra-core" }' >&2
      exit 1
    fi
    mkdir -p "$ROOT/.cargo"
    cat > "$CONFIG" <<EOF
$MARKER
# Builds nimbra-core from the local checkout in ../core instead of the release tag.
# Remove with scripts/local-core.sh off. This file is gitignored.
[patch."https://github.com/nimbra-mail/core"]
nimbra-core = { path = "../core/crates/nimbra-core" }
EOF
    resolve
    echo "nimbra-core now builds from ../core."
    echo "Cargo.lock now has a local nimbra-core: run scripts/local-core.sh off before committing."
    ;;
  off)
    if [[ -f "$CONFIG" ]] && ! ours; then
      echo "$CONFIG was not written by this script; leaving it alone." >&2
      exit 1
    fi
    if [[ -f "$CONFIG" ]]; then
      rm "$CONFIG"
      rmdir "$ROOT/.cargo" 2>/dev/null || true
    fi
    if [[ -z "$(lock_source)" ]]; then
      resolve
    fi
    echo "nimbra-core now builds from the release in Cargo.toml."
    ;;
  status)
    if ours; then
      echo "local: ../core/crates/nimbra-core (scripts/local-core.sh off to go back)"
    else
      echo "release: $(sed -n 's/^nimbra-core = .*tag = "\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n 1) from github.com/nimbra-mail/core"
    fi
    source_line="$(lock_source)"
    if [[ "$source_line" == "$GIT_SOURCE"* ]]; then
      echo "Cargo.lock: ${source_line#source = }"
    else
      echo "Cargo.lock: local path (not committable)"
    fi
    ;;
  *)
    echo "usage: $0 on|off|status" >&2
    exit 2
    ;;
esac
