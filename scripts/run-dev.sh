#!/usr/bin/env bash
# Development run: build, sign with the same identity as the app bundle, then launch.
# Signing keeps the Keychain from asking for permission after every rebuild.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build
./scripts/sign-macos.sh target/debug/nimbra
exec target/debug/nimbra "$@"
