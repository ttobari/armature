#!/bin/sh
# 同梱する書体を公式の配布から取り寄せて dist/fonts に置く(組む機体に入っていなくても組めるように)。
#
#   scripts/fetch-fonts.sh            → dist/fonts/*.ttf
#
# どちらも SIL Open Font License 1.1(許諾文は licenses/)。
set -eu

MORALERSPACE_VERSION=v2.0.0
NERD_FONTS_VERSION=v3.5.1

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT="$ROOT/dist/fonts"
WORK=${TMPDIR:-/tmp}
WORK=${WORK%/}/armature-fonts
mkdir -p "$OUT" "$WORK"

need() {
    for f in "$@"; do
        [ -f "$OUT/$f" ] || return 0
    done
    return 1
}

if need MoralerspaceArgon-Regular.ttf MoralerspaceArgon-Bold.ttf \
    MoralerspaceArgon-Italic.ttf MoralerspaceArgon-BoldItalic.ttf; then
    zip="$WORK/Moralerspace_$MORALERSPACE_VERSION.zip"
    [ -s "$zip" ] || curl -fsSL \
        "https://github.com/yuru7/moralerspace/releases/download/$MORALERSPACE_VERSION/Moralerspace_$MORALERSPACE_VERSION.zip" \
        -o "$zip"
    rm -rf "$WORK/moralerspace" && mkdir -p "$WORK/moralerspace"
    unzip -q -o "$zip" '*MoralerspaceArgon-*.ttf' -d "$WORK/moralerspace"
    for style in Regular Bold Italic BoldItalic; do
        found=$(find "$WORK/moralerspace" -name "MoralerspaceArgon-$style.ttf" | head -1)
        [ -n "$found" ] || { echo "Moralerspace の $style が配布に無い" >&2; exit 1; }
        cp "$found" "$OUT/"
    done
fi

if need JetBrainsMonoNerdFontMono-Regular.ttf; then
    tarball="$WORK/JetBrainsMono_$NERD_FONTS_VERSION.tar.xz"
    [ -s "$tarball" ] || curl -fsSL \
        "https://github.com/ryanoasis/nerd-fonts/releases/download/$NERD_FONTS_VERSION/JetBrainsMono.tar.xz" \
        -o "$tarball"
    rm -rf "$WORK/nerd" && mkdir -p "$WORK/nerd"
    tar -xJf "$tarball" -C "$WORK/nerd" JetBrainsMonoNerdFontMono-Regular.ttf
    cp "$WORK/nerd/JetBrainsMonoNerdFontMono-Regular.ttf" "$OUT/"
fi

ls "$OUT"
