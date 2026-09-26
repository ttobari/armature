#!/bin/sh
# 同梱する書体を公式の配布から取り寄せて dist/fonts に置く(組む機体に入っていなくても組めるように)。
#
#   scripts/fetch-fonts.sh            → dist/fonts/*.ttf
#
# どちらも SIL Open Font License 1.1(許諾文は licenses/)。
set -eu

# 版を上げるときは、下の SHA-256 も配布元の値に替える(GitHub の Release の digest)。
MORALERSPACE_VERSION=v2.0.0
MORALERSPACE_SHA256=56175ee16373ba1a3d2fd5ec46f3b0b6bf0412be7db1481ec7dee757f2e3d557
NERD_FONTS_VERSION=v3.5.1
NERD_FONTS_SHA256=04d5e8f903693f9dd13e16f867e994834e681eb3c72c0d337a770dcda09010cf

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT="$ROOT/dist/fonts"
if [ -n "${TMPDIR:-}" ]; then
    WORK=${TMPDIR%/}/armature-fonts
else
    WORK=$(mktemp -d /tmp/armature-fonts.XXXXXX)
fi
mkdir -p "$OUT" "$WORK"

# 版が替わったら、置いてある書体を捨てて取り寄せ直す。
STAMP="$OUT/VERSIONS"
WANT="moralerspace=$MORALERSPACE_VERSION nerd-fonts=$NERD_FONTS_VERSION"
if [ "$(cat "$STAMP" 2>/dev/null)" != "$WANT" ]; then
    rm -f "$OUT"/*.ttf
fi

# 落とした物は必ず SHA-256 で確かめる。
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

need() {
    for f in "$@"; do
        [ -f "$OUT/$f" ] || return 0
    done
    return 1
}

if need MoralerspaceArgon-Regular.ttf MoralerspaceArgon-Bold.ttf \
    MoralerspaceArgon-Italic.ttf MoralerspaceArgon-BoldItalic.ttf; then
    zip="$WORK/Moralerspace_$MORALERSPACE_VERSION.zip"
    fetch "https://github.com/yuru7/moralerspace/releases/download/$MORALERSPACE_VERSION/Moralerspace_$MORALERSPACE_VERSION.zip" \
        "$zip" "$MORALERSPACE_SHA256"
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
    fetch "https://github.com/ryanoasis/nerd-fonts/releases/download/$NERD_FONTS_VERSION/JetBrainsMono.tar.xz" \
        "$tarball" "$NERD_FONTS_SHA256"
    rm -rf "$WORK/nerd" && mkdir -p "$WORK/nerd"
    tar -xJf "$tarball" -C "$WORK/nerd" JetBrainsMonoNerdFontMono-Regular.ttf
    cp "$WORK/nerd/JetBrainsMonoNerdFontMono-Regular.ttf" "$OUT/"
fi

echo "$WANT" > "$STAMP"
ls "$OUT"
