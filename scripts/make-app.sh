#!/bin/sh
# Builds the app bundle into dist/.
#
#   scripts/make-app.sh                 → dist/Armature.app (ad-hoc signature)
#   SIGN_ID="Developer ID Application: …" scripts/make-app.sh
#                                       → signed for distribution (hardened runtime)
#
# A build with panels of your own (a crate that depends on armature, see CLAUDE.md) uses the same
# script, pointed at that crate:
#
#   MANIFEST=~/dev/my-armature/Cargo.toml PACKAGE=my-armature BINARY_NAME=my-armature \
#   APP_NAME="My Armature" BUNDLE_ID=com.example.my-armature DIST=~/dev/my-armature/dist \
#       ~/dev/armature/scripts/make-app.sh
#
# The name and the icon are yours to change: APP_NAME is the name in the Dock, the menu bar and
# the window title; ICON is a 1024×1024 PNG, or an Icon Composer bundle (*.icon, macOS 26).
#
#   APP_NAME="Workbench" ICON=~/Pictures/workbench.png scripts/make-app.sh
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
RUST="$ROOT/rust"
CRATE="$RUST/crates/armature"

APP_NAME="${APP_NAME:-Armature}"
BUNDLE_ID="${BUNDLE_ID:-blog.tobari.armature}"
PACKAGE="${PACKAGE:-armature}"
BINARY_NAME="${BINARY_NAME:-armature}"
MANIFEST="${MANIFEST:-$RUST/Cargo.toml}"
TARGET_DIR="${TARGET_DIR:-$(dirname "$MANIFEST")/target}"
VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$RUST/Cargo.toml" | head -1)}"
DIST="${DIST:-$ROOT/dist}"
APP="$DIST/$APP_NAME.app"
SIGN_ID="${SIGN_ID:--}"
# 組み立て済みの Armature の束。そこの tmux と書体が同じ版なら、組まず・取り寄せずに写す
# (ダウンロードした版を、その同梱ソースから組み直すとき)。
REUSE_FROM="${REUSE_FROM:-}"
# 配布用の束にはソースを同梱する(7b)。コミットしていない変更があると、同梱するソースと
# 組んだ中身が食い違うので、組む前に止める。
if [ "$SIGN_ID" != "-" ] && ! git -C "$ROOT" diff --quiet HEAD --; then
    echo "コミットしていない変更がある: 配布用の束に入れるソースと組む中身が食い違う" >&2
    exit 1
fi
# 同じ置き場(DIST)へ組む make-app.sh は一度に一本。cargo の錠が守るのは 1. の組み立てだけで、
# 束を組む 3. から先は錠の外にあり、二本が同じ束を消し合う(Claude が裏で走らせた初回の
# 組み立てと、直してから走らせる組み立てが重なりうる。src/notes.rs)。後から来た方は待つ。
mkdir -p "$DIST"
MAKE_APP_LOCK="$DIST/.make-app.lock"
if [ "${ARMATURE_MAKE_APP_LOCK:-}" != "$MAKE_APP_LOCK" ]; then
    ARMATURE_MAKE_APP_LOCK="$MAKE_APP_LOCK"
    export ARMATURE_MAKE_APP_LOCK
    if ! /usr/bin/lockf -k -s -t 0 "$MAKE_APP_LOCK" true; then
        echo "$DIST へ組む make-app.sh がほかに走っている。終わるのを待ってから組む" >&2
    fi
    exec /usr/bin/lockf -k "$MAKE_APP_LOCK" /bin/sh "$0" "$@"
fi
ICON="${ICON:-$RUST/crates/armature/assets/Cockpit.icon}"
CARGO_BIN="${CARGO_BIN:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"

# 1. 本体を組む。最低 OS は 15.0。組んだ機体のパス(利用者名が入る)を実体に残さない。
#    (rustc は後に書いた置き換えを優先する。ホーム全体を先に、細かいものを後に。)
#    旗は CARGO_ENCODED_RUSTFLAGS(区切りは 0x1f)で渡す。RUSTFLAGS は空白で切るので、
#    空白を含む置き場(`~/My Projects/…`)で壊れる。
#    旗は scripts/test.sh と同じ(cargo-env.sh)。違うと依存を丸ごと組み直す。
. "$HERE/cargo-env.sh"
LANG=ja_JP.UTF-8 "$CARGO_BIN" build --release \
    --manifest-path "$MANIFEST" -p "$PACKAGE" --bin "$BINARY_NAME"
BIN="$TARGET_DIR/release/$BINARY_NAME"
BRIDGE="$TARGET_DIR/release/libCockpitTranslation.dylib"
mkdir -p "$DIST"

# 2. 同梱の tmux。Armature 本体の dist に同じ版を組んだものがあれば使い回し、無ければ組む。
TMUX_VERSION=$(sed -n 's/^TMUX_VERSION=//p' "$HERE/build-tmux.sh")
same_tmux() {
    [ -x "$1" ] && [ "$("$1" -V 2>/dev/null)" = "tmux $TMUX_VERSION" ]
}
# 版が同じでも、最低 OS に無い関数を弱リンクで呼ぶ物(0.1.1 までの束の tmux)は使い回さない。
current_tmux() {
    same_tmux "$1" && "$HERE/check-min-os.sh" "$1" 2>/dev/null
}
TMUX_HELPER="$DIST/helpers/tmux"
if ! current_tmux "$TMUX_HELPER"; then
    if current_tmux "$ROOT/dist/helpers/tmux"; then
        TMUX_HELPER="$ROOT/dist/helpers/tmux"
    elif [ -n "$REUSE_FROM" ] && current_tmux "$REUSE_FROM/Contents/Helpers/tmux"; then
        # 動いているアプリが同じ版の tmux を持っている(ダウンロードした版の組み直し)。
        mkdir -p "$DIST/helpers"
        cp "$REUSE_FROM/Contents/Helpers/tmux" "$TMUX_HELPER"
    else
        for old in "$ROOT/dist/helpers/tmux" ${REUSE_FROM:+"$REUSE_FROM/Contents/Helpers/tmux"}; do
            if same_tmux "$old"; then
                echo "$old は最低 OS に無い関数を弱リンクで呼んでいて古い macOS で落ちるので使わず、tmux を組み直す(ソースを取り寄せる)" >&2
            fi
        done
        "$HERE/build-tmux.sh" "$DIST/helpers"
    fi
fi

# 3. 束を組み直す(dist の中だけ。毎回まっさらから)。脇の置き場で組み、全部済んでから
#    $APP と入れ替える——途中で落ちても、組みかけの束が $APP に残らない。
STAGE="$DIST/.building/$APP_NAME.app"
rm -rf "$STAGE"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Frameworks" "$STAGE/Contents/Helpers" \
    "$STAGE/Contents/Resources/fonts" "$STAGE/Contents/Resources/licenses"
cp "$BIN" "$STAGE/Contents/MacOS/$BINARY_NAME"
# 組む途中の置き場を指す rpath を外す(束の中の Frameworks だけを見る)。
otool -l "$STAGE/Contents/MacOS/$BINARY_NAME" | awk '/LC_RPATH/{getline; getline; print $2}' |
    grep -v '^@' | while read -r stale; do
        install_name_tool -delete_rpath "$stale" "$STAGE/Contents/MacOS/$BINARY_NAME"
    done
cp "$BRIDGE" "$STAGE/Contents/Frameworks/"
cp "$TMUX_HELPER" "$STAGE/Contents/Helpers/tmux"

# 4. 書体(どれも SIL OFL 1.1)。決まった版を公式の配布から取り寄せて照合したもの
#    (dist/fonts)だけを入れる。組む機体に入っている別の版は使わない。
FONTS="MoralerspaceArgon-Regular.ttf MoralerspaceArgon-Bold.ttf MoralerspaceArgon-Italic.ttf MoralerspaceArgon-BoldItalic.ttf JetBrainsMonoNerdFontMono-Regular.ttf"
# 動いているアプリの書体を先に写しておく(版の印 VERSIONS が合えば fetch-fonts.sh は取り寄せない。
# 合わなければ捨てて取り寄せ直す)。
if [ -n "$REUSE_FROM" ] && [ -f "$REUSE_FROM/Contents/Resources/fonts/VERSIONS" ] &&
    [ ! -f "$ROOT/dist/fonts/VERSIONS" ]; then
    mkdir -p "$ROOT/dist/fonts"
    cp "$REUSE_FROM/Contents/Resources/fonts/"*.ttf "$REUSE_FROM/Contents/Resources/fonts/VERSIONS" \
        "$ROOT/dist/fonts/"
fi
"$HERE/fetch-fonts.sh" >/dev/null
for font in $FONTS; do
    [ -f "$ROOT/dist/fonts/$font" ] || { echo "書体が無い: $font" >&2; exit 1; }
    cp "$ROOT/dist/fonts/$font" "$STAGE/Contents/Resources/fonts/"
done
cp "$ROOT/dist/fonts/VERSIONS" "$STAGE/Contents/Resources/fonts/"

# 4.5 初回に開く「はじめに」の頁。
cp "$CRATE/assets/welcome.html" "$STAGE/Contents/Resources/welcome.html"

# 5. 第三者の許諾文。Rust の標準ライブラリの分は、組んだ toolchain に付いてくるものを写す。
cp "$ROOT/licenses/"* "$STAGE/Contents/Resources/licenses/"
RUSTC_BIN="$(dirname "$CARGO_BIN")/rustc"
[ -x "$RUSTC_BIN" ] || RUSTC_BIN=rustc
STD_LICENSE="$("$RUSTC_BIN" --print sysroot)/share/doc/rust/COPYRIGHT-library.html"
if [ -f "$STD_LICENSE" ]; then
    cp "$STD_LICENSE" "$STAGE/Contents/Resources/licenses/rust-std-COPYRIGHT.html"
else
    echo "Rust の標準ライブラリの許諾文が無い: $STD_LICENSE" >&2
    exit 1
fi

# 6. アイコン(新形式 .icon → Assets.car。PNG を渡されたか actool が無ければ PNG から icns)。
ICON_PLIST=""
ICON_PNG="$CRATE/assets/cockpit-icon.png"
case "$ICON" in
    *.icon) ICON_BUNDLE="${ICON%/}" ;;
    *) ICON_BUNDLE=""; ICON_PNG="$ICON" ;;
esac
if [ -n "$ICON_BUNDLE" ] && [ -d "$ICON_BUNDLE" ] && xcrun --find actool >/dev/null 2>&1; then
    ICON_ASSET=$(basename "$ICON_BUNDLE" .icon)
    ICON_OUT="${TMPDIR:-/tmp}"
    ICON_OUT="${ICON_OUT%/}/armature-icon.out"
    rm -rf "$ICON_OUT" && mkdir -p "$ICON_OUT"
    xcrun actool "$ICON_BUNDLE" --compile "$ICON_OUT" --platform macosx \
        --minimum-deployment-target 15.0 --app-icon "$ICON_ASSET" \
        --output-partial-info-plist "$ICON_OUT/partial.plist" >/dev/null
    cp "$ICON_OUT/Assets.car" "$STAGE/Contents/Resources/Assets.car"
    cp "$ICON_OUT/$ICON_ASSET.icns" "$STAGE/Contents/Resources/cockpit.icns"
    ICON_PLIST="<key>CFBundleIconName</key><string>$ICON_ASSET</string>"
else
    ICONSET="${TMPDIR:-/tmp}"
    ICONSET="${ICONSET%/}/armature.iconset"
    rm -rf "$ICONSET" && mkdir -p "$ICONSET"
    for size in 16 32 128 256 512; do
        sips -z $size $size "$ICON_PNG" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
        sips -z $((size * 2)) $((size * 2)) "$ICON_PNG" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$STAGE/Contents/Resources/cockpit.icns"
fi

# 7. Info.plist。カメラ・マイクの説明は WebKit が mediaDevices を生やす条件。
cat > "$STAGE/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleExecutable</key><string>$BINARY_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleIconFile</key><string>cockpit</string>
    $ICON_PLIST
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>15.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleLocalizations</key><array><string>en</string><string>ja</string></array>
    <key>NSAppleEventsUsageDescription</key><string>Used by the music panel (Apple Music) and when Claude Code controls an app you allow.</string>
    <key>NSDesktopFolderUsageDescription</key><string>Used when Claude Code works with files you ask it to.</string>
    <key>NSDocumentsFolderUsageDescription</key><string>Used when Claude Code works with files you ask it to.</string>
    <key>NSDownloadsFolderUsageDescription</key><string>Used when Claude Code works with files you ask it to.</string>
    <key>NSCameraUsageDescription</key><string>Used by web pages for video calls.</string>
    <key>NSMicrophoneUsageDescription</key><string>Used by web pages for calls.</string>
</dict>
</plist>
PLIST
# 許可を求める窓の説明文。既定は上の英語、日本語の Mac では ja.lproj の文が出る。
mkdir -p "$STAGE/Contents/Resources/ja.lproj"
cat > "$STAGE/Contents/Resources/ja.lproj/InfoPlist.strings" <<'STRINGS'
"NSAppleEventsUsageDescription" = "音楽のパネル(Apple Music)と、Claude Code が許可されたアプリを操作するときに使います。";
"NSDesktopFolderUsageDescription" = "Claude Code が依頼されたファイルを扱うときに使います。";
"NSDocumentsFolderUsageDescription" = "Claude Code が依頼されたファイルを扱うときに使います。";
"NSDownloadsFolderUsageDescription" = "Claude Code が依頼されたファイルを扱うときに使います。";
"NSCameraUsageDescription" = "Web ページのビデオ通話でカメラを使います。";
"NSMicrophoneUsageDescription" = "Web ページの通話でマイクを使います。";
STRINGS
/usr/bin/plutil -lint "$STAGE/Contents/Resources/ja.lproj/InfoPlist.strings" >/dev/null
/usr/bin/plutil -lint "$STAGE/Contents/Info.plist" >/dev/null
printf 'APPL????' > "$STAGE/Contents/PkgInfo"

# 7b. ソースの在り処と組み直しの命令。窓の中の Claude がこの窓を作り替えられるように
#     (`src/notes.rs` が読んで約束の文に書き足す)。**配布用の束には書かない**——組んだ
#     機体のパス(利用者名が入る)を外へ出さない。
if [ "$SIGN_ID" = "-" ]; then
    quote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
    SOURCE_DIR=$(cd "$(dirname "$MANIFEST")" && pwd)
    DIST_DIR=$(cd "$DIST" && pwd)
    {
        printf 'source=%s\n' "$SOURCE_DIR"
        printf 'repo=%s\n' "$ROOT"
        printf 'app=%s\n' "$DIST_DIR/$APP_NAME.app"
        printf 'build=MANIFEST=%s PACKAGE=%s BINARY_NAME=%s APP_NAME=%s BUNDLE_ID=%s DIST=%s ICON=%s %s\n' \
            "$(quote "$MANIFEST")" "$(quote "$PACKAGE")" "$(quote "$BINARY_NAME")" \
            "$(quote "$APP_NAME")" "$(quote "$BUNDLE_ID")" "$(quote "$DIST")" "$(quote "$ICON")" \
            "$(quote "$ROOT/scripts/make-app.sh")"
    } > "$STAGE/Contents/Resources/source.txt"
else
    # 配布用の束は、その版のソースを丸ごと持つ(初回に ~/Armature/source へ書き出す・
    # `src/notes.rs`)。中身は組んだ木と同じ(コミットしていない変更は冒頭で止めてある)。
    git -C "$ROOT" archive --format=tar.gz HEAD > "$STAGE/Contents/Resources/source.tar.gz"
fi

# 7c. 束の中の Mach-O が、最低 OS(15.0)に無い C の関数を弱リンクで呼んでいないこと。
#     組む機体の SDK が新しいと、古い macOS で起動直後に落ちる物ができる(scripts/check-min-os.sh)。
"$HERE/check-min-os.sh" "$STAGE"

# 8. 署名。中身から外へ。配布用(Developer ID)のときは hardened runtime と時刻印を付ける。
sign() {
    if [ "$SIGN_ID" = "-" ]; then
        codesign --force --sign - "$@"
    else
        codesign --force --timestamp --options runtime --sign "$SIGN_ID" "$@"
    fi
}
sign "$STAGE/Contents/Frameworks/libCockpitTranslation.dylib"
sign "$STAGE/Contents/Helpers/tmux"
if [ "$SIGN_ID" = "-" ]; then
    sign --identifier "$BUNDLE_ID" "$STAGE"
else
    # 配布用は hardened runtime なので、Music の操作・カメラ・マイクの権利を明示する。
    sign --entitlements "$HERE/entitlements.plist" --identifier "$BUNDLE_ID" "$STAGE"
fi
codesign --verify --strict "$STAGE"

rm -rf "$APP"
mv "$STAGE" "$APP"
rmdir "$DIST/.building" 2>/dev/null || true
echo "$APP"
