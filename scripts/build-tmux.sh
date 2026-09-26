#!/bin/sh
# アプリ束に同梱する tmux を組む。
#
# 利用者の Mac に Homebrew も tmux も無くても動くように、libevent と utf8proc は
# 静的に取り込み、ncurses は macOS 同梱(/usr/lib)を使う。最低 OS は束と同じ 15.0。
#
#   scripts/build-tmux.sh            → dist/helpers/tmux
#   scripts/build-tmux.sh <出力先>
set -eu

# 版を上げるときは、下の SHA-256 も配布元の値に替える(GitHub の Release の digest)。
TMUX_VERSION=3.7c
TMUX_SHA256=7c60cae9a0e25288e2e24750aafc9e8800fc7fd4555e447e1b29ee4201cfb3bf
LIBEVENT_VERSION=2.1.13-stable
LIBEVENT_SHA256=f7e9383b8c0baa81b687e5b5eecc01beefaf1b19b64151d95ed61647fe7a315c
UTF8PROC_VERSION=2.11.3
UTF8PROC_SHA256=abfed50b6d4da51345713661370290f4f4747263ee73dc90356299dfc7990c78

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT=${1:-$ROOT/dist/helpers}
# 途中の物は利用者ごとの一時置き場へ。TMPDIR が無いときは共有の /tmp を使い回さず、
# その回だけの置き場を作る(先に置かれた物を信じないため)。
if [ -n "${TMPDIR:-}" ]; then
    WORK=${TMPDIR%/}/armature-tmux
else
    WORK=$(mktemp -d /tmp/armature-tmux.XXXXXX)
fi
# 組んだ libevent と utf8proc は版と組み方ごとの置き場に置く。版か RECIPE を上げると組み直す。
# RECIPE は下の configure の旗を変えたら上げる(前の組み方の物を使い回さないため)。
RECIPE=2
PREFIX=$WORK/prefix-libevent-$LIBEVENT_VERSION-utf8proc-$UTF8PROC_VERSION-r$RECIPE
JOBS=$(sysctl -n hw.ncpu)

export MACOSX_DEPLOYMENT_TARGET=15.0
# 最低 OS より後に入った関数を、確かめずに呼ぶ所があれば組むのを止める。組む機体の SDK が新しいと、
# そうした関数は弱リンクになり、古い macOS では NULL を呼んで落ちる。
export CFLAGS="-O2 -mmacosx-version-min=15.0 -Werror=unguarded-availability-new -I$PREFIX/include"
export LDFLAGS="-mmacosx-version-min=15.0 -L$PREFIX/lib"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig"

mkdir -p "$WORK" "$PREFIX" "$OUT"
cd "$WORK"

# 落とした物は必ず SHA-256 で確かめる。手元に残っている物も、合わなければ落とし直す。
fetch() {
    if [ -s "$2" ] && echo "$3  $2" | shasum -a 256 -c - >/dev/null 2>&1; then
        return 0
    fi
    curl -fsSL "$1" -o "$2"
    echo "$3  $2" | shasum -a 256 -c - >/dev/null 2>&1 || {
        echo "$2 の SHA-256 が合わない(配布元が替わった?)" >&2
        rm -f "$2"
        exit 1
    }
}

# configure は関数の有無をリンクできるかで見るので、SDK にあれば最低 OS に無くても「有る」と判断する。
# 最低 OS 15.0 より後に入った関数は無いものとして組ませる(pipe2 は macOS 27 から)。
NEWER_THAN_15="ac_cv_func_pipe2=no"

LIBEVENT_TAR=libevent-$LIBEVENT_VERSION.tar.gz
UTF8PROC_TAR=utf8proc-$UTF8PROC_VERSION.tar.gz
TMUX_TAR=tmux-$TMUX_VERSION.tar.gz
fetch "https://github.com/libevent/libevent/releases/download/release-$LIBEVENT_VERSION/libevent-$LIBEVENT_VERSION.tar.gz" "$LIBEVENT_TAR" "$LIBEVENT_SHA256"
fetch "https://github.com/JuliaStrings/utf8proc/archive/refs/tags/v$UTF8PROC_VERSION.tar.gz" "$UTF8PROC_TAR" "$UTF8PROC_SHA256"
fetch "https://github.com/tmux/tmux/releases/download/$TMUX_VERSION/tmux-$TMUX_VERSION.tar.gz" "$TMUX_TAR" "$TMUX_SHA256"

if [ ! -f "$PREFIX/lib/libevent_core.a" ]; then
    rm -rf "libevent-$LIBEVENT_VERSION"
    tar xzf "$LIBEVENT_TAR"
    (cd "libevent-$LIBEVENT_VERSION" &&
        ./configure --prefix="$PREFIX" --disable-shared --enable-static \
            --disable-openssl --disable-samples --disable-libevent-regress \
            $NEWER_THAN_15 >/dev/null &&
        make -j"$JOBS" >/dev/null &&
        make install >/dev/null)
fi

if [ ! -f "$PREFIX/lib/libutf8proc.a" ]; then
    rm -rf "utf8proc-$UTF8PROC_VERSION"
    tar xzf "$UTF8PROC_TAR"
    (cd "utf8proc-$UTF8PROC_VERSION" &&
        make -j"$JOBS" libutf8proc.a CFLAGS="$CFLAGS" >/dev/null &&
        mkdir -p "$PREFIX/include" "$PREFIX/lib" &&
        cp utf8proc.h "$PREFIX/include/" &&
        cp libutf8proc.a "$PREFIX/lib/")
fi

rm -rf "tmux-$TMUX_VERSION"
tar xzf "$TMUX_TAR"
(cd "tmux-$TMUX_VERSION" &&
    ./configure --prefix="$PREFIX" --enable-utf8proc --disable-jemalloc $NEWER_THAN_15 \
        LIBEVENT_CORE_CFLAGS="-I$PREFIX/include" \
        LIBEVENT_CORE_LIBS="$PREFIX/lib/libevent_core.a" \
        LIBEVENT_CFLAGS="-I$PREFIX/include" \
        LIBEVENT_LIBS="$PREFIX/lib/libevent_core.a" \
        LIBUTF8PROC_CFLAGS="-I$PREFIX/include" \
        LIBUTF8PROC_LIBS="$PREFIX/lib/libutf8proc.a" >/dev/null &&
    make -j"$JOBS" >/dev/null)

# 最低 OS に無い関数を弱リンクで呼んでいないこと(出力先へ写す前に。落ちる物を置かない)。
"$HERE/check-min-os.sh" "tmux-$TMUX_VERSION/tmux"
cp "tmux-$TMUX_VERSION/tmux" "$OUT/tmux"
chmod +x "$OUT/tmux"

# 同梱できる形かを確かめる: /usr/lib と /System の外を指していないこと。
if otool -L "$OUT/tmux" | tail -n +2 | awk '{print $1}' | grep -v -E '^(/usr/lib/|/System/)'; then
    echo "tmux が同梱できない依存を持っている(上の行)" >&2
    exit 1
fi
# バイナリで配るとき表示の再掲を求める BSD の compat が、許諾の表示に全部載っているか。
# 版を上げて組み込まれる compat が替わったら、licenses/tmux-compat-notices.txt に足す。
NOTICES="$ROOT/licenses/tmux-compat-notices.txt"
missing=""
for object in "tmux-$TMUX_VERSION"/compat/*.o; do
    source="${object%.o}.c"
    [ -f "$source" ] && grep -q "Redistributions in binary form" "$source" || continue
    name="compat/$(basename "$source")"
    grep -qx "$name" "$NOTICES" || missing="$missing $name"
done
# tree.h と queue.h は、macOS に無いので compat の版が読み込まれる。
for header in tree queue; do
    grep -qx "compat/$header.h" "$NOTICES" || missing="$missing compat/$header.h"
done
if [ -n "$missing" ]; then
    echo "tmux に組み込まれた BSD の compat の表示が $NOTICES に無い:$missing" >&2
    exit 1
fi

echo "$OUT/tmux"
"$OUT/tmux" -V
