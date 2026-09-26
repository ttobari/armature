#!/bin/sh
# Runs the tests the way make-app.sh builds the app (release, the same flags), so the tests and the
# app share one build of the dependencies instead of compiling them twice.
#
#   scripts/test.sh                      → every crate in the workspace
#   scripts/test.sh -p armature weather  → the arguments go to cargo test
#   MANIFEST=~/dev/my-armature/Cargo.toml scripts/test.sh   → a crate of your own
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
MANIFEST="${MANIFEST:-$ROOT/rust/Cargo.toml}"
. "$HERE/cargo-env.sh"
[ $# -gt 0 ] || set -- --workspace
exec "$CARGO_BIN" test --release --manifest-path "$MANIFEST" "$@"
