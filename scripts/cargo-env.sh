# make-app.sh と test.sh が同じ旗で cargo を呼ぶための共通部分(`.` で読む。ROOT を先に決めておく)。
# 旗が一つでも違うと cargo は依存を丸ごと組み直す——テストとアプリで一度組んだ依存を使い回せなくなる。
#
# 組んだ機体のパス(利用者名が入る)を実体に残さない。rustc は後に書いた置き換えを優先するので、
# ホーム全体を先に、細かいものを後に。旗は CARGO_ENCODED_RUSTFLAGS(区切りは 0x1f)で渡す。
# RUSTFLAGS は空白で切るので、空白を含む置き場(`~/My Projects/…`)で壊れる。
SEP=$(printf '\037')
CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$HOME=/home$SEP--remap-path-prefix=$HOME/.cargo/registry/src=/cargo$SEP--remap-path-prefix=$HOME/.rustup=/rustup$SEP--remap-path-prefix=$ROOT=/src"
# proc-macro だけ strip を切る。Rust 1.97 までの strip は proc-macro の dylib を macOS 27 が
# 読めない形に削る(rust-lang/rust#157750。1.98.0 で直った)。束の中身は変わらない。
CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=false
MACOSX_DEPLOYMENT_TARGET=15.0
export CARGO_ENCODED_RUSTFLAGS CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP MACOSX_DEPLOYMENT_TARGET
CARGO_BIN="${CARGO_BIN:-$(command -v cargo || echo "$HOME/.cargo/bin/cargo")}"
