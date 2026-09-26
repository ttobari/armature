//! What every Claude tab in the window is told: where it runs, where the panel layout is, and
//! what each shown panel says about itself ([`crate::Panel::note`]).
//!
//! Nothing is installed into Claude Code. The notes are written to one file in the state folder
//! and passed with `--append-system-prompt-file` to each Claude tab Armature starts; nothing is
//! written to the user's `~/.claude`, and it only holds inside those sessions.

use std::path::PathBuf;

use armature_core::tr;

/// The part about Armature itself. `{name}` and `{panels}` are filled in.
const HEADER_EN: &str = "\
# This app

You are running inside {name}, a Mac app: this terminal sits in the middle of a window, with panels in columns on either side.

- The panel layout lives in `{panels}`. Edit it when the user asks to move, add or hide panels; the window follows at once.
";

const HEADER_JA: &str = "\
# このアプリ

ここは {name} という Mac のアプリの中。この端末は窓の真ん中にあり、左右の列にパネルが並んでいる。

- パネルの並びは `{panels}` にある。パネルを動かす・足す・外すと頼まれたらこれを書き換える。窓はその場で追従する。
";

/// How to change this window, when it was built on this Mac (`scripts/make-app.sh` writes
/// `Resources/source.txt` into local builds, never into signed ones). `{source}`, `{repo}`,
/// `{build}` are filled in; `{copy}` is the line about a copy that runs from elsewhere.
const CHANGE_EN: &str = "\
## Changing this app

This window was built on this Mac from `{source}` (Armature's code is at `{repo}`; its `CLAUDE.md` is the map for adding or changing a panel). When the user asks to add a panel or change how the window works, edit the code, run the tests with `MANIFEST='{source}/Cargo.toml' '{repo}/scripts/test.sh'`, and rebuild with:

```sh
{build}
```
{own_crate}
When the new build is in place, a green restart icon (circling arrows) appears at the bottom of the right column; the user clicks it (or presses ⇧⌘R) to switch to it.{copy}
";

const CHANGE_JA: &str = "\
## この窓を作り替える

この窓は、この Mac で `{source}` から組んだもの(Armature のコードは `{repo}`。パネルの足し方・直し方の地図はそこの `CLAUDE.md`)。パネルを足したい・窓の動きを変えたいと頼まれたら、コードを直し、`MANIFEST='{source}/Cargo.toml' '{repo}/scripts/test.sh'` で試験を通してから、次の命令で組み直す。

```sh
{build}
```
{own_crate}
新しい束ができると、右の列の下に緑の再起動のアイコン(円を描く矢印)が出る。利用者がそれ(または ⇧⌘R)を押すと切り替わる。{copy}
";

const OWN_CRATE_EN: &str = "\nThis is plain Armature's own checkout. For changes the user wants to keep, prefer a crate of their own that depends on it (`CLAUDE.md`, \"Your own crate\"): their code stays out of Armature's repository and updating Armature stays a `git pull`.\n";
const OWN_CRATE_JA: &str = "\nこれは素の Armature の checkout。利用者が持ち続けたい変更なら、これに依存する利用者自身の crate を作るほうを勧める(`CLAUDE.md` の「Your own crate」)——利用者のコードが Armature のリポに混ざらず、Armature の更新は `git pull` のまま済む。\n";

const COPY_EN: &str = " This window runs from `{running}`, not from where the build lands (`{built}`): after building, copy it over with `ditto '{built}' '{running}'` so the restart icon appears.";
const COPY_JA: &str = " この窓は組んだ先(`{built}`)ではなく `{running}` から動いている。組み直したら `ditto '{built}' '{running}'` で上書きすると再起動のアイコンが出る。";

/// Where Armature's code lives, for a copy that was downloaded rather than built on this Mac.
/// A fork that hands out its own signed build sets `ARMATURE_REPOSITORY` when building it.
const REPOSITORY: &str = match option_env!("ARMATURE_REPOSITORY") {
    Some(url) => url,
    None => "https://github.com/ttobari/armature",
};

/// How to change a downloaded copy: there is no code on this Mac, so start from the repository.
const PREBUILT_EN: &str = "\
## Changing this app

This copy of Armature was downloaded ready-made, so its code is not on this Mac. When the user asks to add a panel or change how the window works, clone {repo} (or their own fork of it), read its `CLAUDE.md` — the map for adding or changing a panel — and prefer a crate of the user's own that depends on it. Build with `scripts/make-app.sh`; the user then opens the app it builds instead of this one, and from there you can change and rebuild it in place.
";

const PREBUILT_JA: &str = "\
## この窓を作り替える

この Armature は組み上がったものをダウンロードした版で、この Mac にコードは無い。パネルを足したい・窓の動きを変えたいと頼まれたら、{repo}(または利用者のフォーク)を clone し、そこの `CLAUDE.md`(パネルの足し方・直し方の地図)を読み、それに依存する利用者自身の crate を作るほうを勧める。`scripts/make-app.sh` で組み、利用者はこの版の代わりに組んだアプリを開く。そこから先は、その窓の中で直して組み直せる。
";

/// How to change a downloaded copy that carries its own source (unpacked to `{source}` on the
/// first launch). `{name}`, `{version}`, `{source}`, `{running}` and `{repo}` are filled in.
const UNPACKED_EN: &str = "\
## Changing this app

This copy of {name} was downloaded ready-made, and it carries its own code: the source of this version ({version}) is at `{source}`. Its `CLAUDE.md` is the map for adding or changing a panel. When the user asks to add a panel or change how the window works:

1. Check the tools for building: `xcode-select -p` (Xcode Command Line Tools) and `cargo` (Rust). If one is missing, tell the user what it is and ask before installing it (`xcode-select --install`; Rust from https://rustup.rs).
2. The first build compiles every dependency and takes several minutes; later builds only recompile what changed. If `{source}/dist/{name}.app` does not exist yet, start the build in the background right away, before reading or writing any code, so it compiles while you work: `REUSE_FROM='{running}' '{source}/scripts/make-app.sh'` (it takes tmux and the fonts from this app instead of building and downloading them).
3. Edit the code, run the tests with `'{source}/scripts/test.sh'` (it shares the build with the app; `-p armature <name>` runs only some), and build with `REUSE_FROM='{running}' '{source}/scripts/make-app.sh'`.
4. Put the build in place of this copy: `ditto '{source}/dist/{name}.app' '{running}'`. A green restart icon (circling arrows) appears at the bottom of the right column; the user clicks it (or presses ⇧⌘R) to switch. From then on the app knows where its code is.

The folder has no Git history. If the user wants to follow updates, make it a Git checkout of {repo} at the tag `v{version}`, keeping their changes. `.armature-version` in the folder says which version it came from; if it is not {version}, tell the user before building.
";

const UNPACKED_JA: &str = "\
## この窓を作り替える

この {name} は組み上がったものをダウンロードした版で、自分のコードを持っている。この版({version})のソースは `{source}` にあり、パネルの足し方・直し方の地図はそこの `CLAUDE.md`。パネルを足したい・窓の動きを変えたいと頼まれたら:

1. 組む道具を確かめる: `xcode-select -p`(Xcode Command Line Tools)と `cargo`(Rust)。無いものがあれば、それが何かを利用者に伝え、入れてよいか聞いてから入れる(`xcode-select --install`、Rust は https://rustup.rs)。
2. 初回の組み立ては依存を全部組むので数分かかる。2 回目からは変えたところだけ。`{source}/dist/{name}.app` がまだ無ければ、コードを読み書きする前に、組み立てを裏で走らせておく(作業の間に組み上がる): `REUSE_FROM='{running}' '{source}/scripts/make-app.sh'`(tmux と字体はこのアプリから写し、組まない・取り寄せない)。
3. コードを直し、`'{source}/scripts/test.sh'` で試験を通し(アプリと組み立てを分け合う。`-p armature <名前>` で一部だけ)、`REUSE_FROM='{running}' '{source}/scripts/make-app.sh'` で組む。
4. 組んだものをこの版と入れ替える: `ditto '{source}/dist/{name}.app' '{running}'`。右の列の下に緑の再起動のアイコン(円を描く矢印)が出るので、利用者がそれ(または ⇧⌘R)を押すと切り替わる。以後のアプリは自分のコードの在り処を知っている。

このフォルダに Git の履歴は無い。利用者が更新を追いたいなら、変更を残したまま {repo} のタグ `v{version}` の Git の checkout にする。フォルダの `.armature-version` はどの版から書き出したかを示す。{version} でなければ、組む前に利用者に伝える。
";

/// This app's version (the source a downloaded copy carries is of this version).
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The source a signed build carries (`Resources/source.tar.gz`, written by `scripts/make-app.sh`).
fn bundled_source() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let tar = exe.parent()?.parent()?.join("Resources/source.tar.gz");
    tar.is_file().then_some(tar)
}

/// Where a downloaded copy unpacks its source: `source` in the app's folder (`~/Armature`).
#[must_use]
pub fn unpacked_source_dir() -> PathBuf {
    armature_core::paths::tasks_home().join("source")
}

/// The first launch of a downloaded copy unpacks the source it carries, once. An existing
/// folder is never touched: it may hold the user's changes.
pub fn unpack_source() {
    if built_from().is_some() {
        return;
    }
    let Some(tar) = bundled_source() else {
        return;
    };
    let dir = unpacked_source_dir();
    if dir.exists() {
        return;
    }
    // Unpack beside it and rename, so a half-written folder never looks like the source.
    let partial = dir.with_file_name(".source-unpacking");
    let _ = std::fs::remove_dir_all(&partial);
    if std::fs::create_dir_all(&partial).is_err() {
        return;
    }
    let unpacked = std::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(&tar)
        .arg("-C")
        .arg(&partial)
        .status()
        .is_ok_and(|status| status.success());
    if unpacked
        && std::fs::write(partial.join(".armature-version"), format!("{VERSION}\n")).is_ok()
        && std::fs::rename(&partial, &dir).is_ok()
    {
        return;
    }
    let _ = std::fs::remove_dir_all(&partial);
}

fn unpacked_part(running: &std::path::Path) -> String {
    tr!(UNPACKED_EN, UNPACKED_JA)
        .replace("{name}", armature_core::paths::name())
        .replace("{version}", VERSION)
        .replace("{source}", &unpacked_source_dir().to_string_lossy())
        .replace("{running}", &running.to_string_lossy())
        .replace("{repo}", REPOSITORY)
}

/// The app bundle this process runs from.
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.parent()?.to_path_buf())
}

/// Whether this process runs from an app bundle (a downloaded copy, or a local build).
fn in_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .is_some_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// What `source.txt` in this app's bundle says, or `None` (a signed build, or run with cargo).
fn built_from() -> Option<(std::collections::HashMap<String, String>, PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let contents = exe.parent()?.parent()?;
    let text = std::fs::read_to_string(contents.join("Resources/source.txt")).ok()?;
    let fields = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    Some((fields, contents.parent()?.to_path_buf()))
}

/// The "changing this app" part, from what the build left in the bundle.
fn change_part(fields: &std::collections::HashMap<String, String>, running: &std::path::Path) -> Option<String> {
    let source = fields.get("source")?;
    let repo = fields.get("repo")?;
    let build = fields.get("build")?;
    let built = fields.get("app").map(String::as_str).unwrap_or_default();
    let running = running.to_string_lossy();
    let copy = if built.is_empty() || built == running {
        String::new()
    } else {
        tr!(COPY_EN, COPY_JA)
            .replace("{built}", built)
            .replace("{running}", &running)
    };
    // The checkout of Armature itself (its workspace is `<repo>/rust`), not a crate of the user's.
    let own_crate = if std::path::Path::new(source).starts_with(repo) {
        tr!(OWN_CRATE_EN, OWN_CRATE_JA).to_string()
    } else {
        String::new()
    };
    Some(
        tr!(CHANGE_EN, CHANGE_JA)
            .replace("{source}", source)
            .replace("{repo}", repo)
            .replace("{build}", build)
            .replace("{own_crate}", &own_crate)
            .replace("{copy}", &copy),
    )
}

/// Where the notes are written.
fn path() -> PathBuf {
    armature_core::paths::state().join("claude-notes.md")
}

/// Writes the notes: Armature's own part, then each shown panel's note, in order. Unchanged text
/// is not rewritten.
pub fn write(panel_notes: &[String]) {
    let mut text = tr!(HEADER_EN, HEADER_JA)
        .replace("{name}", armature_core::paths::name())
        .replace("{panels}", &crate::layout::Layout::path().to_string_lossy());
    for note in panel_notes {
        text.push('\n');
        text.push_str(note.trim_end());
        text.push('\n');
    }
    let change = match built_from() {
        Some((fields, running)) => change_part(&fields, &running),
        // A signed, downloaded copy carries no paths from the Mac it was built on. It carries
        // its own source (unpacked by `unpack_source`), or, from before it did, points at the
        // repository.
        None if in_bundle() => match (bundled_source(), running_bundle()) {
            (Some(_), Some(running)) => Some(unpacked_part(&running)),
            _ => Some(tr!(PREBUILT_EN, PREBUILT_JA).replace("{repo}", REPOSITORY)),
        },
        None => None,
    };
    if let Some(change) = change {
        text.push('\n');
        text.push_str(change.trim_end());
        text.push('\n');
    }
    let path = path();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, text);
}

/// The file to pass to Claude, once it has been written.
#[must_use]
pub fn file() -> Option<PathBuf> {
    let path = path();
    path.exists().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn a_local_build_tells_claude_where_its_code_is_and_how_to_rebuild() {
        let _en = armature_core::lang::scoped(armature_core::lang::Lang::En);
        let built = fields(&[
            ("source", "/Users/me/dev/armature/rust"),
            ("repo", "/Users/me/dev/armature"),
            ("app", "/Users/me/dev/armature/dist/Armature.app"),
            ("build", "'/Users/me/dev/armature/scripts/make-app.sh'"),
        ]);
        let text = change_part(&built, std::path::Path::new("/Users/me/dev/armature/dist/Armature.app")).unwrap();
        assert!(text.contains("`/Users/me/dev/armature/rust`"));
        assert!(text.contains("'/Users/me/dev/armature/scripts/make-app.sh'"));
        assert!(text.contains("crate of their own"), "plain Armature suggests a crate of the user's");
        assert!(!text.contains("ditto"), "running from where it was built: nothing to copy");
        // Copied to /Applications: say how to put the new build in its place.
        let text = change_part(&built, std::path::Path::new("/Applications/Armature.app")).unwrap();
        assert!(text.contains("ditto '/Users/me/dev/armature/dist/Armature.app' '/Applications/Armature.app'"));
        // A crate of the user's: no advice to make one.
        let mine = fields(&[
            ("source", "/Users/me/dev/my-armature"),
            ("repo", "/Users/me/dev/armature"),
            ("app", "/Users/me/dev/my-armature/dist/My.app"),
            ("build", "make"),
        ]);
        let text = change_part(&mine, std::path::Path::new("/Users/me/dev/my-armature/dist/My.app")).unwrap();
        assert!(!text.contains("crate of their own"));
        assert!(change_part(&fields(&[]), std::path::Path::new("/x")).is_none());
    }

    #[test]
    fn a_downloaded_copy_points_claude_at_the_repository() {
        for text in [PREBUILT_EN, PREBUILT_JA] {
            let text = text.replace("{repo}", REPOSITORY);
            assert!(text.contains("https://github.com/ttobari/armature"));
            assert!(text.contains("CLAUDE.md"));
            assert!(!text.contains("/Users/"), "no paths from the Mac it was built on");
        }
    }

    #[test]
    fn a_downloaded_copy_with_its_source_says_where_it_is_and_how_to_swap_the_build_in() {
        for text in [UNPACKED_EN, UNPACKED_JA] {
            for key in ["{name}", "{version}", "{source}", "{running}", "{repo}"] {
                assert!(text.contains(key), "{key} is not used");
            }
            assert!(text.contains("xcode-select --install") && text.contains("rustup.rs"));
            assert!(text.contains("ditto '{source}/dist/{name}.app' '{running}'"));
            assert!(text.contains("make-app.sh"));
            assert!(text.contains("REUSE_FROM='{running}'") && text.contains("scripts/test.sh"));
        }
        let _en = armature_core::lang::scoped(armature_core::lang::Lang::En);
        let text = unpacked_part(std::path::Path::new("/Applications/Armature.app"));
        assert!(text.contains("'/Applications/Armature.app'"));
        assert!(text.contains(&format!("v{VERSION}")));
        assert!(!text.contains('{'), "every placeholder is filled: {text}");
    }

    #[test]
    fn the_header_names_the_panel_file() {
        for header in [HEADER_EN, HEADER_JA] {
            assert!(header.contains("{panels}"));
            assert!(header.contains("{name}"));
        }
    }
}
