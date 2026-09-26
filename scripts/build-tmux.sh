#!/bin/sh
# アプリ束に同梱する tmux を組む。
#
# 利用者の Mac に Homebrew も tmux も無くても動くように、libevent と utf8proc は
# 静的に取り込み、ncurses は macOS 同梱(/usr/lib)を使う。最低 OS は束と同じ 15.0。
#
#   scripts/build-tmux.sh            → dist/helpers/tmux
#   scripts/build-tmux.sh <出力先>
set -eu

TMUX_VERSION=3.7c
LIBEVENT_VERSION=2.1.13-stable
UTF8PROC_VERSION=2.11.3

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT=${1:-$ROOT/dist/helpers}
WORK=${TMPDIR:-/tmp}
WORK=${WORK%/}/armature-tmux
PREFIX=$WORK/prefix
JOBS=$(sysctl -n hw.ncpu)

export MACOSX_DEPLOYMENT_TARGET=15.0
export CFLAGS="-O2 -mmacosx-version-min=15.0 -I$PREFIX/include"
export LDFLAGS="-mmacosx-version-min=15.0 -L$PREFIX/lib"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"

mkdir -p "$WORK" "$PREFIX" "$OUT"
cd "$WORK"

fetch() {
    [ -s "$2" ] || curl -fsSL "$1" -o "$2"
}

fetch "https://github.com/libevent/libevent/releases/download/release-$LIBEVENT_VERSION/libevent-$LIBEVENT_VERSION.tar.gz" libevent.tar.gz
fetch "https://github.com/JuliaStrings/utf8proc/archive/refs/tags/v$UTF8PROC_VERSION.tar.gz" utf8proc.tar.gz
fetch "https://github.com/tmux/tmux/releases/download/$TMUX_VERSION/tmux-$TMUX_VERSION.tar.gz" tmux.tar.gz

if [ ! -f "$PREFIX/lib/libevent_core.a" ]; then
    rm -rf "libevent-$LIBEVENT_VERSION"
    tar xzf libevent.tar.gz
    (cd "libevent-$LIBEVENT_VERSION" &&
        ./configure --prefix="$PREFIX" --disable-shared --enable-static \
            --disable-openssl --disable-samples --disable-libevent-regress >/dev/null &&
        make -j"$JOBS" >/dev/null &&
        make install >/dev/null)
fi

if [ ! -f "$PREFIX/lib/libutf8proc.a" ]; then
    rm -rf "utf8proc-$UTF8PROC_VERSION"
    tar xzf utf8proc.tar.gz
    (cd "utf8proc-$UTF8PROC_VERSION" &&
        make -j"$JOBS" libutf8proc.a CFLAGS="$CFLAGS" >/dev/null &&
        mkdir -p "$PREFIX/include" "$PREFIX/lib" &&
        cp utf8proc.h "$PREFIX/include/" &&
        cp libutf8proc.a "$PREFIX/lib/")
fi

rm -rf "tmux-$TMUX_VERSION"
tar xzf tmux.tar.gz
(cd "tmux-$TMUX_VERSION" &&
    ./configure --prefix="$PREFIX" --enable-utf8proc --disable-jemalloc \
        LIBEVENT_CORE_CFLAGS="-I$PREFIX/include" \
        LIBEVENT_CORE_LIBS="$PREFIX/lib/libevent_core.a" \
        LIBEVENT_CFLAGS="-I$PREFIX/include" \
        LIBEVENT_LIBS="$PREFIX/lib/libevent_core.a" \
        LIBUTF8PROC_CFLAGS="-I$PREFIX/include" \
        LIBUTF8PROC_LIBS="$PREFIX/lib/libutf8proc.a" >/dev/null &&
    make -j"$JOBS" >/dev/null)

cp "tmux-$TMUX_VERSION/tmux" "$OUT/tmux"
chmod +x "$OUT/tmux"

# 同梱できる形かを確かめる: /usr/lib と /System の外を指していないこと。
if otool -L "$OUT/tmux" | tail -n +2 | awk '{print $1}' | grep -v -E '^(/usr/lib/|/System/)'; then
    echo "tmux が同梱できない依存を持っている(上の行)" >&2
    exit 1
fi
echo "$OUT/tmux"
"$OUT/tmux" -V
