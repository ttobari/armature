#!/bin/sh
# 束に入れる Mach-O が、最低 OS(15.0)に無い C の関数を弱リンクで呼んでいないかを確かめる。
#
# 新しい SDK で組むと、最低 OS より後に入った関数は弱リンクになり、その関数の無い macOS では
# NULL を呼んで落ちる(2026-09-26: SDK 27 で組んだ tmux が、libevent の pipe2 で macOS 26 では
# 起動直後に落ちた)。次の弱リンクは除く:
# - Swift の記号(`_$s…`・`__swift_FORCE_LOAD_$_…`)。Swift は @available / #available で守らないと組ませない。
# - Objective-C のクラス(`_OBJC_CLASS_$_…`・`_OBJC_METACLASS_$_…`)。無い版では nil になり、呼んでも落ちない。
# 強リンクの記号は見ない(無ければ dyld が起動の時点で名前を出して止める)。
#
#   scripts/check-min-os.sh <Mach-O または .app>...   → 見つかれば記号を出して exit 1
#   .app を渡すと、Resources 以外にある Mach-O を全部見る(1 つも無ければ exit 1)。
set -eu

[ $# -gt 0 ] || { echo "check-min-os.sh: 確かめる物が無い" >&2; exit 1; }

list=$(mktemp "${TMPDIR:-/tmp}/check-min-os.XXXXXX")
trap 'rm -f "$list"' EXIT
for target in "$@"; do
    if [ -d "$target" ]; then
        # Resources は資源の置き場(書体・頁・許諾文)。コードは署名と公証が資源として扱うので置かない。
        before=$(wc -l < "$list")
        find "$target/Contents" -path "$target/Contents/Resources" -prune -o -type f -print |
            while read -r file; do
                case "$(file -b "$file")" in
                    Mach-O*) printf '%s\n' "$file" ;;
                esac
            done >> "$list"
        [ "$(wc -l < "$list")" -gt "$before" ] || { echo "$target に Mach-O が無い" >&2; exit 1; }
    else
        printf '%s\n' "$target" >> "$list"
    fi
done

status=0
while read -r file; do
    # nm が読めなければ止める(確かめられなかった物を通さない)。
    symbols=$(nm -m "$file") || { echo "$file を nm で読めない" >&2; exit 1; }
    weak=$(printf '%s\n' "$symbols" |
        awk '$1 == "(undefined)" && $2 == "weak" && $3 == "external" {print $4}' |
        grep -v -E '^_\$s|^__swift_FORCE_LOAD_\$_|^_OBJC_(META)?CLASS_\$_' || true)
    if [ -n "$weak" ]; then
        echo "$file が最低 OS に無いかもしれない関数を弱リンクで呼んでいる:" $weak >&2
        status=1
    fi
done < "$list"
exit $status
