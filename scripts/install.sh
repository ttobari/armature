#!/bin/sh
# Builds Armature from this checkout, puts it in Applications and opens it.
#
#   scripts/install.sh
#
# Needs the Xcode Command Line Tools and Rust; it says what is missing and stops. The first run
# also fetches the fonts and builds tmux (a few minutes). Run it again after changing the code.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
APP_NAME="${APP_NAME:-Armature}"
DIST="${DIST:-$ROOT/dist}"

missing=""
xcode-select -p >/dev/null 2>&1 || missing="$missing
  Xcode Command Line Tools: xcode-select --install"
if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"
        export PATH
    else
        missing="$missing
  Rust: https://rustup.rs"
    fi
fi
if [ -n "$missing" ]; then
    echo "Install these first, then run scripts/install.sh again:$missing" >&2
    exit 1
fi
if ! command -v claude >/dev/null 2>&1 && [ ! -x "$HOME/.local/bin/claude" ]; then
    echo "Note: Claude Code is not installed (https://claude.com/claude-code). Armature opens without it, but the center needs it." >&2
fi

DIST="$DIST" APP_NAME="$APP_NAME" "$HERE/make-app.sh"

DEST=/Applications
[ -w "$DEST" ] || { DEST="$HOME/Applications"; mkdir -p "$DEST"; }
# 動いている版を閉じてから入れ替える(セッションは tmux の中で続く)。
osascript -e "tell application \"$APP_NAME\" to quit" >/dev/null 2>&1 || true
ditto "$DIST/$APP_NAME.app" "$DEST/$APP_NAME.app"
open "$DEST/$APP_NAME.app"
echo "$DEST/$APP_NAME.app"
