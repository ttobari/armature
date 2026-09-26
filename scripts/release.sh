#!/bin/sh
# 配布用の DMG を組む: Developer ID で署名 → DMG → 公証 → 貼り付け(staple)→ 検証。
#
#   SIGN_ID="Developer ID Application: <名前> (<TEAM>)" scripts/release.sh
#
# 公証は App Store Connect の API キーで通す。鍵の在り処は環境変数か
# ~/.appstoreconnect/config.env(ASC_KEY_ID・ASC_ISSUER_ID)と
# ~/.appstoreconnect/private_keys/AuthKey_<ASC_KEY_ID>.p8。
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
DIST="$ROOT/dist"
APP_NAME="Armature"
APP="$DIST/$APP_NAME.app"

[ -n "${SIGN_ID:-}" ] && [ "$SIGN_ID" != "-" ] || {
    echo "SIGN_ID に Developer ID Application の名前を入れて呼ぶ" >&2
    echo "  security find-identity -v -p codesigning | grep 'Developer ID Application'" >&2
    exit 2
}

CONF="$HOME/.appstoreconnect/config.env"
[ -f "$CONF" ] && . "$CONF"
: "${ASC_KEY_ID:?ASC_KEY_ID が無い}"
: "${ASC_ISSUER_ID:?ASC_ISSUER_ID が無い}"
ASC_KEY_PATH="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8}"
[ -f "$ASC_KEY_PATH" ] || { echo "API キーが無い: $ASC_KEY_PATH" >&2; exit 1; }

# 1. 署名つきの束(hardened runtime)。
SIGN_ID="$SIGN_ID" "$HERE/make-app.sh"
VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist")
DMG="$DIST/$APP_NAME-$VERSION.dmg"

# 2. DMG(アプリと、ドラッグ先の Applications)。
STAGE="$DIST/dmg-stage"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
codesign --force --timestamp --sign "$SIGN_ID" "$DMG"

# 3. 公証(数分かかる)→ 貼り付け。
xcrun notarytool submit "$DMG" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" \
    --issuer "$ASC_ISSUER_ID" --wait
xcrun stapler staple "$DMG"

# 4. 検証: Gatekeeper がダウンロードしたものとして通すか。
spctl --assess --type open --context context:primary-signature --verbose "$DMG"
echo "$DMG"
