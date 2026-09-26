use std::collections::HashMap;
use armature_core::tr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Armature 専用の tmux の口とセッションの名(`-L <slug>`)。名前を替えた版(`App::name`)は
/// 別の口を使うので、素の Armature のタブと混ざらない。利用者の tmux には触らない。
fn tmux_label() -> &'static str {
    armature_core::paths::slug()
}
/// 一度のホイールで送る報告の上限。慣性スクロールの積み上がりを抑える。
const WHEEL_REPORT_LIMIT: u32 = 10;

#[derive(Clone, Debug)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Claude,
    Shell,
}

#[must_use]
pub fn prepare() -> Launch {
    let working_directory = session_directory();
    let claude = find_program("claude");
    let tmux = find_program("tmux");

    if let Some(tmux) = tmux.as_ref() {
        // Claude Code が無い機体では、最初の窓で入れ方を案内する(Enter で公式の
        // インストーラが走る)。入れ終われば ⌘T で Claude のタブが立つ。
        configure_tmux(tmux, claude.as_deref(), &working_directory);
        return Launch {
            program: tmux.to_string_lossy().into_owned(),
            args: vec![
                "-L".into(),
                tmux_label().into(),
                "-f".into(),
                "/dev/null".into(),
                "attach-session".into(),
                "-t".into(),
                format!("={label}", label = tmux_label()),
            ],
            working_directory,
        };
    }

    if let Some(claude) = claude {
        return Launch {
            program: claude.to_string_lossy().into_owned(),
            args: default_claude_arguments(),
            working_directory,
        };
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    Launch {
        program: shell,
        args: Vec::new(),
        working_directory,
    }
}

#[must_use]
pub fn settings(launch: &Launch) -> iced_term::settings::Settings {
    let (program, args, env) = pty_command(launch, armature_core::proc::child_environment());
    iced_term::settings::Settings {
        font: font_settings(),
        theme: iced_term::settings::ThemeSettings::new(Box::new(terminal_palette())),
        backend: iced_term::settings::BackendSettings {
            program,
            args,
            env,
            working_directory: Some(launch.working_directory.clone()),
        },
    }
}

/// 端末の PTY で起こす実体・引数・足す環境。
///
/// PTY の口(alacritty_terminal)は環境を足せても落とせないので、落とす名前が
/// あるときは `/usr/bin/env -u …` を前に挟む。窓の環境は触らない。
fn pty_command(
    launch: &Launch,
    child_env: &armature_core::proc::ChildEnv,
) -> (String, Vec<String>, HashMap<String, String>) {
    let mut env: HashMap<String, String> = child_env.set.iter().cloned().collect();
    env.insert("TERM".into(), "xterm-256color".into());
    env.insert("COLORTERM".into(), "truecolor".into());
    if child_env.remove.is_empty() {
        return (launch.program.clone(), launch.args.clone(), env);
    }
    let mut args = Vec::new();
    for key in &child_env.remove {
        args.extend(["-u".to_string(), key.clone()]);
    }
    args.push(launch.program.clone());
    args.extend(launch.args.iter().cloned());
    ("/usr/bin/env".into(), args, env)
}

fn font_settings() -> iced_term::settings::FontSettings {
    iced_term::settings::FontSettings {
        size: 12.0,
        scale_factor: 1.2,
        // Claude Codeのロゴは半ブロック文字で組まれているため、
        // 端末には字形の揃った等幅書体を使う。
        font_type: crate::font::TERMINAL,
    }
}

pub fn suppress_app_shortcuts(terminal: &mut iced_term::Terminal) {
    use iced_term::Command as TerminalCommand;

    let _ = terminal.handle(TerminalCommand::AddBindings(app_shortcut_bindings()));
}

fn app_shortcut_bindings() -> Vec<(
    iced_term::bindings::Binding<iced_term::bindings::InputKind>,
    iced_term::bindings::BindingAction,
)> {
    use iced::keyboard::{Modifiers, key::Named};
    use iced_term::TermMode;
    use iced_term::bindings::{Binding, BindingAction, InputKind};

    let binding = |target, modifiers| Binding {
        target,
        modifiers,
        terminal_mode_include: TermMode::empty(),
        terminal_mode_exclude: TermMode::empty(),
    };
    vec![
        (
            binding(InputKind::Char("t".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(
                InputKind::Char("t".into()),
                Modifiers::COMMAND | Modifiers::SHIFT,
            ),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::KeyCode(Named::Tab), Modifiers::CTRL),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(
                InputKind::KeyCode(Named::Tab),
                Modifiers::CTRL | Modifiers::SHIFT,
            ),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("w".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("e".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("f".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("n".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("o".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("r".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("p".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("x".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("g".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("s".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("b".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("h".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("l".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
        (
            binding(InputKind::Char("/".into()), Modifiers::COMMAND),
            BindingAction::Esc(String::new()),
        ),
    ]
}

/// 起こした window に貼る印。閉じたときに台帳と突き合わせる鍵。
///
/// window の題では紐づけない——`claude` という題は何枚も並ぶので取り違える。
const SESSION_OPTION: &str = "@claude-session";

/// 新しい session id を振る。macOS 標準の uuidgen を使い、依存を増やさない。
fn new_session_id() -> Option<String> {
    let output = Command::new("/usr/bin/uuidgen").output().ok()?;
    let id = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();
    (!id.is_empty()).then_some(id)
}

/// window を起こし、session id の印を貼って台帳へ控える。
///
/// `new-window -P` に起こした window を名乗らせてから貼る——`list-windows` を
/// 数えて当てると、続けて起こした回に取り違える。
fn spawn_claude_window(
    tmux: &std::path::Path,
    command: &mut Command,
    id: &str,
    tab: &str,
    cwd: &str,
) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| {
            tr!(
                format!("Couldn't open the tab: {error}"),
                format!("タブを起こせない: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(tr!(
            format!("Opening the tab stopped with {}", output.status),
            format!("タブの起動が終了コード{}で止まった", output.status),
        ));
    }
    let window = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !window.is_empty() {
        let _ = child(tmux)
            .args(["-L", tmux_label(), "-f", "/dev/null", "set-option"])
            .args(["-t", &window, "-w", SESSION_OPTION, id])
            .status();
    }
    crate::session_ledger::record_opened(id, tab, cwd);
    Ok(window)
}

/// いま生きている Claude タブの session id。
///
/// 印を貼っていない window(素のシェル・印を貼る前に死んだ回)は空行で来るので落とす。
#[must_use]
pub fn live_session_ids() -> Vec<String> {
    let Some(tmux) = find_program("tmux") else {
        return Vec::new();
    };
    let Ok(output) = child(tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "list-windows", "-a"])
        .args(["-F", &format!("#{{{SESSION_OPTION}}}")])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// 閉じたタブを、同じ題・同じ場所・同じ会話・**同じ model と effort** で起こし直す。
///
/// `--resume` は session id を変えないので、戻した window にも同じ印を貼る。
/// model と effort は会話ログから読んで明示する(`resume.rs`)——`--resume` だけだと
/// その時点の既定で立ち、別のタブで `/model` を打った値が黙って乗る。
pub fn reopen(entry: &crate::session_ledger::Entry) -> Result<String, String> {
    let tmux = find_program("tmux")
        .ok_or_else(|| tr!("tmux not found", "tmuxが見つからない").to_string())?;
    let claude = find_program("claude")
        .ok_or_else(|| tr!("claude not found", "claudeが見つからない").to_string())?;
    let cwd = if entry.cwd.is_empty() {
        session_directory()
    } else {
        std::path::PathBuf::from(&entry.cwd)
    };
    let tab = if entry.tab.is_empty() {
        "claude".to_string()
    } else {
        entry.tab.clone()
    };
    let mut command = child(&tmux);
    command
        .args(["-L", tmux_label(), "-f", "/dev/null", "new-window"])
        .args(["-t", &format!("={label}:", label = tmux_label())])
        .args(["-P", "-F", "#{window_id}"])
        .args(["-c"])
        .arg(&cwd)
        .args(["-n", &tab])
        .arg("/usr/bin/env")
        .args([
            "-u",
            "TMUX",
            "-u",
            "NO_COLOR",
            "COLORTERM=truecolor",
            "FORCE_COLOR=3",
        ])
        .arg(claude)
        .args(["--resume", &entry.id])
        .args(default_claude_arguments())
        .args(crate::resume::for_session(&entry.id).arguments());
    spawn_claude_window(&tmux, &mut command, &entry.id, &tab, &cwd.to_string_lossy())?;
    Ok(tr!(format!("Brought back {tab}"), format!("{tab} を戻した")))
}

pub fn open_tab(kind: TabKind) -> bool {
    let Some(tmux) = find_program("tmux") else {
        return false;
    };
    let cwd = session_directory();
    let mut command = child(&tmux);
    command
        .args(["-L", tmux_label(), "-f", "/dev/null", "new-window"])
        .args(["-t", &format!("={label}:", label = tmux_label())])
        .args(["-c"])
        .arg(&cwd);
    match kind {
        TabKind::Claude => {
            let Some(claude) = find_program("claude") else {
                return false;
            };
            // id をこちらで振っておく。閉じたあとに会話ログから当てずに戻せる。
            let Some(id) = new_session_id() else {
                return false;
            };
            command
                .args(["-P", "-F", "#{window_id}"])
                .args(["-n", "claude"])
                .arg("/usr/bin/env")
                .args(["-u", "NO_COLOR", "COLORTERM=truecolor", "FORCE_COLOR=3"])
                .arg(claude)
                .args(["--session-id", &id])
                .args(default_claude_arguments());
            return spawn_claude_window(&tmux, &mut command, &id, "claude", &cwd.to_string_lossy())
                .is_ok();
        }
        TabKind::Shell => {
            command.args(["-n", "terminal"]);
        }
    }
    command.status().is_ok_and(|status| status.success())
}

/// ホイールを SGR のマウス報告に写す。tmux(`mouse on`)がこれをペインへ配る。
///
/// Claude Code は alternate screen で動くため tmux の履歴が溜まらず、
/// copy-mode で遡る方式は効かない(2026-08-17実測: alt=1, history=0)。
/// 一方で自らマウス報告を要求している(mouse_any=1)ので、報告を届ければ
/// トランスクリプトを自分で辿る。報告を見ないシェルでは tmux が履歴へ入る。
/// 位置は使われないので原点固定(64=上/65=下)。
/// 向きは利用者の好みで反転済み。
#[must_use]
pub fn wheel_report(lines: i32) -> Vec<u8> {
    let button = if lines > 0 { 65 } else { 64 };
    let mut report = Vec::new();
    for _ in 0..lines.unsigned_abs().min(WHEEL_REPORT_LIMIT) {
        report.extend_from_slice(format!("\x1b[<{button};1;1M").as_bytes());
    }
    report
}

/// 外から足したパネルが頼んだ Claude のタブ(`panel::Host::start_claude`)。
/// `arguments` はそのまま claude へ渡す(最初の一言は最後)。
pub fn start_claude(title: &str, arguments: &[String]) -> Result<String, String> {
    let tmux = find_program("tmux")
        .ok_or_else(|| tr!("tmux not found", "tmuxが見つからない").to_string())?;
    let claude = find_program("claude")
        .ok_or_else(|| tr!("claude not found", "claudeが見つからない").to_string())?;
    let cwd = session_directory();
    let tab = match title.trim() {
        "" => "claude",
        title => title,
    };
    let id = new_session_id().ok_or_else(|| "can't make a session id".to_string())?;
    let mut command = child(&tmux);
    command
        .args(["-L", tmux_label(), "-f", "/dev/null", "new-window"])
        .args(["-t", &format!("={label}:", label = tmux_label())])
        .args(["-P", "-F", "#{window_id}"])
        .args(["-c"])
        .arg(&cwd)
        .args(["-n", tab])
        .arg("/usr/bin/env")
        .args([
            "-u",
            "TMUX",
            "-u",
            "NO_COLOR",
            "COLORTERM=truecolor",
            "FORCE_COLOR=3",
        ])
        .arg(claude)
        .args(["--session-id", &id])
        // 外から足したパネルの引数は最後(`--` の後ろに最初の一言が来うる)。
        .args(default_claude_arguments())
        .args(arguments);
    spawn_claude_window(&tmux, &mut command, &id, tab, &cwd.to_string_lossy())?;
    Ok(tr!(format!("Started {tab}"), format!("{tab} を起動した")))
}

/// 素の Claude タブの引数。**モデルも思考量も決め打ちしない**——利用者の
/// Claude Code の既定(`/model` で選んだもの)をそのまま使う。画面は全画面で起こし
/// ([`fullscreen_arguments`])、この窓の約束を渡す([`app_arguments`])。
fn default_claude_arguments() -> Vec<String> {
    plain_claude_arguments(crate::notes::file().as_deref(), &armature_core::paths::tasks_dir())
}

fn plain_claude_arguments(protocol: Option<&Path>, tasks_dir: &Path) -> Vec<String> {
    let mut args = fullscreen_arguments();
    args.extend(app_arguments(protocol, tasks_dir));
    args
}

/// この窓で起こす Claude がみな受け取る引数。約束の文(この窓とパネルの並び、出ている
/// パネルそれぞれの説明——`notes`)と、Armature の置き場(タスク)を開けておく `--add-dir`。
/// **タスクを足すのは Claude に頼む**ので、どのタブからでも置き場へ書けるようにしておく。
fn app_arguments(protocol: Option<&Path>, tasks_dir: &Path) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(protocol) = protocol {
        args.extend([
            "--append-system-prompt-file".into(),
            protocol.to_string_lossy().into_owned(),
        ]);
    }
    args.extend(["--add-dir".into(), tasks_dir.to_string_lossy().into_owned()]);
    args
}

/// Claude Code を全画面の TUI(`"tui": "fullscreen"`)で起こす引数。
///
/// 中央のパネルは Claude の画面そのもの。既定の描き方は会話を端末の履歴へ流すので、
/// 入力欄が会話の末尾について上下に動き、パネルの下半分が空く。全画面なら入力欄は
/// パネルの下端に据わり、会話は上の枠の中で送る。`--settings` で渡すのはこの1欄だけで、
/// 利用者の設定ファイルは書き換えない(`/tui` で切り替えれば、その会話の中ではそちらが勝つ)。
fn fullscreen_arguments() -> Vec<String> {
    vec!["--settings".into(), r#"{"tui":"fullscreen"}"#.into()]
}

pub fn next_tab() -> bool {
    let Some(tmux) = find_program("tmux") else {
        return false;
    };
    child(tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "next-window"])
        .args(["-t", &format!("={label}", label = tmux_label())])
        .status()
        .is_ok_and(|status| status.success())
}

pub fn previous_tab() -> bool {
    let Some(tmux) = find_program("tmux") else {
        return false;
    };
    child(tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "previous-window"])
        .args(["-t", &format!("={label}", label = tmux_label())])
        .status()
        .is_ok_and(|status| status.success())
}

/// 左の一覧で選んだIced専用tmuxの窓へ直接移る。
pub fn select_tab(target: &str) -> bool {
    let Some(index) = target.strip_prefix(&format!("{label}:", label = tmux_label())) else {
        return false;
    };
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Some(tmux) = find_program("tmux") else {
        return false;
    };
    child(tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "select-window"])
        .args(["-t", target])
        .status()
        .is_ok_and(|status| status.success())
}

/// 一覧で掴んで動かした並びを、専用tmuxの**窓順そのもの**へ写す。
///
/// **表示だけ並べ替えない。**窓順は ⌃Tab の巡回順でもあるので、見えている並びと
/// tmux の並びが食い違うと「すぐ下に見えているパネルが、次のタブではない」状態になる。
/// 窓を動かせば並びは窓側に残り、名簿の着弾を待たずに次の列挙から新しい順で返る。
///
/// `order` は `"armature:N"` の宛先を新しい並びで渡す。いま在る窓の**過不足の
/// ない並べ替え**でなければ何もしない——一覧には名簿由来の(この窓のタブでない)
/// 行も混ざるので、取りこぼしを黙って詰めると別の窓を動かす。
#[must_use]
pub fn reorder_tabs(order: &[String]) -> bool {
    let Some(tmux) = find_program("tmux") else {
        return false;
    };
    let Ok(output) = child(&tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "list-windows"])
        .args(["-t", &format!("={label}", label = tmux_label())])
        // 動かす相手は窓番号ではなく**窓ID(@n)**で指す。番号は移し替えの途中で
        // 入れ替わるが、IDは窓が死ぬまで変わらない。
        .args(["-F", "#{window_index}\t#{window_id}\t#{window_active}"])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    // **前面のパネルは並べ替えの前に控える。**窓を外して繋ぎ直すと tmux は前面を
    // 手放し、最後に動かした窓が前に出る。並びを直したら控えた窓へ必ず戻す。
    let front = active_window_id(&listing).map(str::to_string);
    let Some(moves) = reorder_moves(&listing, order) else {
        return false;
    };
    let mut command = child(&tmux);
    command.args(["-L", tmux_label(), "-f", "/dev/null"]);
    for (position, (id, index)) in moves.iter().enumerate() {
        if position > 0 {
            command.arg(";");
        }
        // `-d` を落とすと移した窓が前面になる。並べ替えで見ているパネルを奪わない。
        command.args([
            "move-window",
            "-d",
            "-s",
            id,
            "-t",
            &format!("={label}:{index}", label = tmux_label()),
        ]);
    }
    if let Some(front) = &front {
        command.args([";", "select-window", "-t", front]);
    }
    if command.status().is_ok_and(|status| status.success()) {
        return true;
    }
    // **落ちた列は途中まで効いている。**tmux の `;` の列は最初の失敗で止まるので
    // (移し替えの最中に窓が閉じた等)、退避番号に置き去りの窓が残りうる。連番へ
    // 詰め直して、9番・10番へ飛んだパネルが一覧の末尾に居座るのを防ぐ(2026-09-04 実測)。
    let mut repair = child(&tmux);
    repair
        .args(["-L", tmux_label(), "-f", "/dev/null", "move-window", "-r"])
        .args(["-t", &format!("={label}", label = tmux_label())]);
    if let Some(front) = &front {
        repair.args([";", "select-window", "-t", front]);
    }
    let _ = repair.status();
    false
}

/// `list-windows` の出力から、いま前面に居る窓のID。
fn active_window_id(raw: &str) -> Option<&str> {
    raw.lines().find_map(|line| {
        let mut fields = line.splitn(3, '\t');
        let _index = fields.next()?;
        let id = fields.next()?.trim();
        (fields.next()?.trim() == "1").then_some(id)
    })
}

/// `list-windows` の出力と望む並びから、`move-window` の手順を組む。
///
/// **二段で動かす。**行き先が埋まっている限り `move-window` は失敗するので、
/// いったん全部を空き番号(いまの最大+1 以降)へ退避してから、元の番号の並びへ
/// 落とし直す。番号の集合はこれで完全に保たれる——`-r`(連番の振り直し)を
/// 使うと `base-index` 次第で全部の番号がずれる。
fn reorder_moves(raw: &str, order: &[String]) -> Option<Vec<(String, u32)>> {
    let mut windows: Vec<(u32, String)> = raw
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (index, id) = (fields.next()?, fields.next()?);
            let id = id.trim();
            if id.is_empty() {
                return None;
            }
            Some((index.trim().parse().ok()?, id.to_string()))
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    windows.sort_by_key(|(index, _)| *index);
    let prefix = format!("{label}:", label = tmux_label());
    let wanted: Vec<String> = order
        .iter()
        .filter_map(|target| {
            let index: u32 = target.strip_prefix(&prefix)?.parse().ok()?;
            windows
                .iter()
                .find(|(current, _)| *current == index)
                .map(|(_, id)| id.clone())
        })
        .collect();
    // 過不足(取りこぼし・重複)があれば触らない。重複を通すと、二段目で
    // 同じ窓を2度落として**退避したままの窓が残る**。
    if wanted.len() != windows.len() {
        return None;
    }
    let mut distinct: Vec<&str> = wanted.iter().map(String::as_str).collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() != windows.len() {
        return None;
    }
    if wanted
        .iter()
        .zip(windows.iter())
        .all(|(id, (_, current))| id == current)
    {
        return None;
    }
    let scratch = windows.last()?.0.checked_add(1)?;
    let moves = wanted
        .iter()
        .enumerate()
        .map(|(position, id)| (id.clone(), scratch + position as u32))
        .chain(
            wanted
                .iter()
                .zip(windows.iter())
                .map(|(id, (index, _))| (id.clone(), *index)),
        )
        .collect();
    Some(moves)
}

/// いま見ているタブを閉じる。
///
/// **閉じた瞬間に台帳へ刻む。**`session_ledger::sweep` は ⌘⇧T を押した時点で
/// 消えている控えへまとめて同じ刻を打つので、続けて2枚閉じるとどちらが最後か
/// 分からなくなり、戻す相手を取り違えていた。閉じる側なら瞬間が分かる。
pub fn close_tab() -> bool {
    let Some(tmux) = find_program("tmux") else {
        return false;
    };
    let Ok(output) = child(&tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "list-windows"])
        .args(["-t", &format!("={label}", label = tmux_label())])
        // 印も一緒に読む。閉じてからでは、その窓がどの会話だったか引けない。
        .args([
            "-F",
            &format!("#{{window_id}} #{{window_active}} #{{{SESSION_OPTION}}}"),
        ])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let windows = String::from_utf8_lossy(&output.stdout);
    if windows.lines().count() <= 1 {
        return false;
    }
    let Some(active) = active_window(&windows) else {
        return false;
    };
    let session = active_session(&windows).map(str::to_string);
    let closed = child(tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "kill-window"])
        .args(["-t", active])
        .status()
        .is_ok_and(|status| status.success());
    if closed && let Some(session) = session {
        crate::session_ledger::mark_closed(&session);
    }
    closed
}

/// タブの題。ペインが名乗った題を先に採る。**名乗りが無いときの tmux の既定は機体の名前**
/// (`#{host}`)で、題として出すと「vm.local」のような字がタブ名の代わりに並ぶ——そのときは
/// 窓の名(起こしたタスクの題・コマンド名)を採る。
///
/// **端末への問い合わせの字も題にしない。** Claude Code は立ち上がりに kitty の画像の問い
/// (`ESC _ Gi=31,s=1,…; … ESC \`)を送る。tmux は `ESC _ … ESC \` を screen の流儀で
/// 「題を付ける」と読むので、名乗りの前に `Gi=31,s=1,v=1,a=q,…` が題として残る。
fn pane_or_window_title<'a>(pane: &'a str, host: &str, window: &'a str) -> &'a str {
    if pane.is_empty() || pane == host || is_graphics_query(pane) {
        window
    } else {
        pane
    }
}

/// kitty の画像の問いの中身(`G` の後に `鍵=値` を `,` で並べ、`;` で本体が続く)か。
fn is_graphics_query(title: &str) -> bool {
    let Some(control) = title.strip_prefix('G').and_then(|rest| rest.split(';').next()) else {
        return false;
    };
    !control.is_empty()
        && control.split(',').all(|pair| {
            pair.split_once('=').is_some_and(|(key, _)| {
                key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic())
            })
        })
}

#[must_use]
pub fn local_tabs() -> Vec<armature_core::monitor::LocalTab> {
    let Some(tmux) = find_program("tmux") else {
        return Vec::new();
    };
    let Ok(output) = child(tmux)
        .args(["-L", tmux_label(), "-f", "/dev/null", "list-panes", "-a"])
        .args([
            "-F",
            // 印(@claude-session)は題の**前**に置く——題はタブを含みうるので末尾列でなければ
            // 分割できない。印は窓が振った会話ID(空なら素のシェル)。
            &format!(
                "#{{session_name}}\t#{{pane_tty}}\t#{{window_index}}\t#{{window_name}}\t#{{window_active}}\t#{{{SESSION_OPTION}}}\t#{{host}}\t#{{pane_title}}"
            ),
        ])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_local_tabs(&String::from_utf8_lossy(&output.stdout))
}

fn parse_local_tabs(raw: &str) -> Vec<armature_core::monitor::LocalTab> {
    let mut tabs: Vec<(i64, armature_core::monitor::LocalTab)> = raw
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.splitn(8, '\t').collect();
            if fields.len() != 8 || fields[0] != tmux_label() {
                return None;
            }
            let index = fields[2].parse().ok()?;
            let title = pane_or_window_title(fields[7], fields[6], fields[3]);
            let session_id = Some(fields[5].trim())
                .filter(|id| !id.is_empty())
                .map(str::to_string);
            Some((
                index,
                armature_core::monitor::LocalTab {
                    tty: fields[1].trim_start_matches("/dev/").to_string(),
                    target: format!("{label}:{index}", label = tmux_label()),
                    // 左の一覧は窓名よりペインの題を優先し、状態印はmonitor側で剥がす。
                    // 題中のタブは末尾列として分割せず、そのまま保持する。
                    title: title.to_string(),
                    named: true,
                    active: fields[4] == "1",
                    unread: false,
                    busy: false,
                    session_id,
                },
            ))
        })
        .collect();
    tabs.sort_by_key(|(index, _)| *index);
    tabs.into_iter().map(|(_, tab)| tab).collect()
}

/// タブ列挙から、生きている Claude タブの session id だけを抜く。
///
/// [`live_session_ids`] と同じ答えを、すでに手元にある列挙から作る——一覧を
/// 写すたびに tmux をもう1回起こさない。
#[must_use]
pub fn session_ids_of(tabs: &[armature_core::monitor::LocalTab]) -> Vec<String> {
    tabs.iter().filter_map(|tab| tab.session_id.clone()).collect()
}

fn active_window(windows: &str) -> Option<&str> {
    windows.lines().find_map(|line| {
        let (id, rest) = line.split_once(' ')?;
        let active = rest.split_once(' ').map_or(rest, |(active, _)| active);
        (active.trim() == "1").then_some(id)
    })
}

/// いま見ている窓に貼ってある会話の印。素のシェルの窓には無いので `None`。
fn active_session(windows: &str) -> Option<&str> {
    windows.lines().find_map(|line| {
        let (_, rest) = line.split_once(' ')?;
        let (active, session) = rest.split_once(' ')?;
        let session = session.trim();
        (active == "1" && !session.is_empty()).then_some(session)
    })
}

/// tmux サーバの環境から親セッションの印を落とす。
///
/// 窓側で落とすだけでは届かない——サーバは窓より長生きで、印を抱えた親から起きた
/// サーバが残っていると、以後どの `new-window` もそこから環境を相続する。相続すると
/// タブの Claude Code が自分を子セッションと誤認し、transcript の保存を切る。
/// `-r` は「無い」ではなく「明示的に空」を据えるので、掛け直しても増えない。
fn shed_server_markers(tmux: &Path, base: &[&str; 4]) {
    let Ok(output) = child(tmux)
        .args(base)
        .args(["show-environment", "-g"])
        .output()
    else {
        return;
    };
    let listing = String::from_utf8_lossy(&output.stdout);
    let keys = server_environment_markers(&listing);
    if keys.is_empty() {
        return;
    }
    let mut command = child(tmux);
    command.args(base);
    for key in keys {
        command.args(["set-environment", "-gr", key, ";"]);
    }
    let _ = command.status();
}

/// `show-environment -g` の出力から、落とすべき名前を拾う。
fn server_environment_markers(listing: &str) -> Vec<&str> {
    listing
        .lines()
        .map(|line| line.split_once('=').map_or(line, |(key, _)| key))
        .filter(|key| armature_core::proc::is_inherited_marker(key))
        .collect()
}

/// Claude Code が見つからないとき、最初の窓で走らせる案内。
const INSTALL_GUIDE_EN: &str = r#"printf '\n  Claude Code is not installed.\n\n  Press Enter to run the official installer:\n    curl -fsSL https://claude.ai/install.sh | bash\n  Press Ctrl+C to stop.\n\n'
read _
curl -fsSL https://claude.ai/install.sh | bash
printf '\n  When it finishes, open a Claude tab with ⌘T (the first time, you sign in in the browser).\n\n'
exec "${SHELL:-/bin/zsh}" -l"#;

const INSTALL_GUIDE_JA: &str = r#"printf '\n  Claude Code が見つかりません。\n\n  Enter を押すと公式のインストーラを実行します:\n    curl -fsSL https://claude.ai/install.sh | bash\n  やめるときは Ctrl+C。\n\n'
read _
curl -fsSL https://claude.ai/install.sh | bash
printf '\n  終わったら ⌘T で Claude のタブを開いてください(初回はブラウザでログインします)。\n\n'
exec "${SHELL:-/bin/zsh}" -l"#;

fn configure_tmux(tmux: &Path, claude: Option<&Path>, cwd: &Path) {
    let base = ["-L", tmux_label(), "-f", "/dev/null"];
    let target = format!("={label}", label = tmux_label());
    let exists = child(tmux)
        .args(base)
        .args(["has-session", "-t", &target])
        .status()
        .is_ok_and(|status| status.success());
    if !exists {
        // サーバはこの1本の環境(`child` が当てた差分)を以後のタブへ配る。
        let mut command = child(tmux);
        command
            .args(base)
            // history-limit は作成済みのペインへ効かないので、窓0より先に据える。
            .args(["start-server", ";"])
            .args(["set-option", "-g", "history-limit", "50000", ";"])
            // 窓の名を付けておく。付けないと tmux が走っているプロセスの名(Claude Code
            // では版の数字「2.1.282」)をタブ名にする。
            .args(["new-session", "-d", "-s", tmux_label(), "-n", "claude", "-c"])
            .arg(cwd);
        match claude {
            Some(claude) => {
                command
                    .arg("/usr/bin/env")
                    .args(["-u", "NO_COLOR", "COLORTERM=truecolor", "FORCE_COLOR=3"])
                    .arg(claude)
                    .args(default_claude_arguments());
            }
            None => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                command
                    .arg(shell)
                    .args(["-l", "-c", tr!(INSTALL_GUIDE_EN, INSTALL_GUIDE_JA)]);
            }
        }
        let _ = command.status();
    }
    shed_server_markers(tmux, &base);
    // **1プロセスで全部据える。** 1つずつ起こしていた頃は、起動直後(名簿の Node・
    // 原画パックの読み込みと CPU を取り合う間)に10回の spawn で 3.4 秒掛かった
    // (2026-08-30 実測)。tmux は `;` 区切りで複数の命令を1度に受ける。
    let mut command = child(tmux);
    command.args(base);
    for (option, value) in [
        ("status", "off"),
        ("prefix", "None"),
        ("escape-time", "0"),
        ("history-limit", "50000"),
        ("destroy-unattached", "off"),
        // ホイール報告をペインのアプリへ配らせる(Claude Code は自前で辿る)。
        ("mouse", "on"),
    ] {
        command.args(["set-option", "-t", tmux_label(), option, value, ";"]);
    }
    // 外側端末(alacritty系)は同期更新(mode 2026)を解するが、xterm-256color の
    // terminfo からは検出されない。明示しないと tmux が描き直しの途中経過を
    // マーカーなしで流し、カーソルが瞬く(2026-08-17実測)。
    // 窓の出入りをペインのアプリへ伝える。既定は off で、`-f /dev/null` で起こす
    // このサーバは `~/.tmux.conf` を読まないので、ここで立てるほかに口が無い
    // (Claude Code は off のとき「focus tracking が効かない」と毎回言う)。
    command.args(["set-option", "-s", "focus-events", "on", ";"]);
    command.args([
        "set-option",
        "-s",
        "terminal-features",
        // `hyperlinks` を足す理由。**tmux は外側が解せないと判断すると
        // OSC 8 を剥がす**——リンクの札(見える字)だけが通り、URL は捨てられる。
        // こちらの端末は alacritty_terminal で OSC 8 を解するのに、
        // xterm-256color の terminfo には `Hls` が無いので名乗れていなかった。
        // そのせいで ⌘クリックが開く先を持てなかった(2026-08-23 実測:
        // 升の hyperlink が空・画面の字を舐める正規表現も札しか見つけられない)。
        "xterm-256color:sync:hyperlinks",
    ]);
    let _ = command.status();
}

fn session_directory() -> PathBuf {
    // 解決順は「環境変数 → config.env → 既定」。作業場は機体で違うので、どこで
    // claude を起こすかは設定ファイルで差し替えられる。束から起こした窓には
    // 環境変数が届かないため、環境変数だけでは機体の設定の置き場にならない。
    let configured = crate::config::read_value("CLAUDE_SESSION_CWD");
    if let Some(raw) = configured {
        let path = expand_home(&raw);
        if path.is_dir() {
            return path;
        }
    }
    // 既定はこのアプリの置き場(タスクの親)。**ホームにはしない**——Claude Code は
    // ホームで「このフォルダを信頼するか」を聞いても答えを覚えず、タブを開くたびに
    // 同じ問いが出る。自分の置き場なら最初の一回で済む。
    // どこで起こすかは設定(config.env の CLAUDE_SESSION_CWD)で変えられる。
    let own = armature_core::paths::tasks_home();
    if own.is_dir() || std::fs::create_dir_all(&own).is_ok() {
        return own;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        return PathBuf::from(home);
    }
    PathBuf::from("/")
}

pub(crate) fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest);
    }
    PathBuf::from(raw)
}

/// 子を起こす口。窓の環境は触らず、子にだけ差分(親のセッションの印を落とす・PATH・
/// locale・色)を当てる(`armature_core::proc::child_env`)。
fn child(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    armature_core::proc::child_env(&mut command);
    command
}

pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    // アプリ束に同梱した道具(`Contents/Helpers`)を最初に見る。tmux は同梱してあるので、
    // 利用者の Mac に Homebrew が無くてもタブが立つ。
    if let Some(bundled) = bundled_helper(name) {
        return Some(bundled);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for path in [
        PathBuf::from(&home).join(".local/bin").join(name),
        PathBuf::from("/opt/homebrew/bin").join(name),
        PathBuf::from("/usr/local/bin").join(name),
        PathBuf::from(&home).join(".claude/local").join(name),
        PathBuf::from(&home).join(".npm-global/bin").join(name),
    ] {
        if path.is_file() {
            return Some(path);
        }
    }
    login_shell_lookup(name)
}

/// 束の `Contents/Helpers/<name>`。束の外で起こしたとき(`cargo run`)は無い。
fn bundled_helper(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.parent()?.join("Helpers").join(name);
    path.is_file().then_some(path)
}

/// 利用者のログインシェルに聞く(nvm などで入れた claude は決まった棚に居ない)。
/// Dock から起こした窓はシェルの PATH を持たないので、ここでシェルを1本起こして引く。
fn login_shell_lookup(name: &str) -> Option<PathBuf> {
    static CACHE: std::sync::Mutex<Vec<(String, Option<PathBuf>)>> = std::sync::Mutex::new(Vec::new());
    if let Ok(cache) = CACHE.lock()
        && let Some((_, found)) = cache.iter().find(|(key, _)| key == name)
    {
        return found.clone();
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let found = child(shell)
        .args(["-l", "-c", &format!("command -v {name}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|text| text.lines().last().map(|line| PathBuf::from(line.trim())))
        .filter(|path| path.is_absolute() && path.is_file());
    // 見つからなかった回は覚えない。案内から入れた直後の ⌘T で見つけ直すため。
    if found.is_some()
        && let Ok(mut cache) = CACHE.lock()
    {
        cache.push((name.to_string(), found.clone()));
    }
    found
}

/// 端末の色。窓の絵の具(`palette`)から起こす。
///
/// ANSI の16色は意味色と絵の具から引き当てる。dim は同じ色を地へ半分沈めたもの。
/// 明るい白は本文を白へ寄せた色——Catppuccin の明るい白(桃)だと、入力行の字が
/// 桃色に出る。8番の地(入力行の帯)は画面の地から1段だけ浮かせ、字に使う8番
/// (補助の灰色)はそのまま読める明るさに残す。
pub fn terminal_palette() -> iced_term::ColorPalette {
    use crate::palette;
    let hex = |color: iced::Color| {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!(
            "#{:02x}{:02x}{:02x}",
            channel(color.r),
            channel(color.g),
            channel(color.b)
        )
    };
    let dim = |color: iced::Color| hex(palette::mix(color, palette::crust(), 0.45));
    let bright_white = palette::lighten(palette::text_ink(), 0.5);
    iced_term::ColorPalette {
        foreground: hex(palette::text_ink()),
        // 器と同じ地(`palette::terminal_seat_style`)。
        background: hex(palette::surface_card()),
        black: hex(palette::crust()),
        red: hex(palette::red()),
        green: hex(palette::green()),
        yellow: hex(palette::yellow()),
        blue: hex(palette::blue()),
        magenta: hex(palette::mauve()),
        cyan: hex(palette::sky()),
        white: hex(palette::text_ink()),
        bright_black: hex(palette::overlay1()),
        bright_red: hex(palette::maroon()),
        bright_green: hex(palette::green()),
        bright_yellow: hex(palette::peach()),
        bright_blue: hex(palette::blue()),
        bright_magenta: hex(palette::mauve()),
        bright_cyan: hex(palette::sky()),
        bright_white: hex(bright_white),
        bright_foreground: Some(hex(bright_white)),
        dim_foreground: hex(palette::subtext0()),
        dim_black: dim(palette::crust()),
        dim_red: dim(palette::red()),
        dim_green: dim(palette::green()),
        dim_yellow: dim(palette::yellow()),
        dim_blue: dim(palette::blue()),
        dim_magenta: dim(palette::mauve()),
        dim_cyan: dim(palette::sky()),
        dim_white: dim(palette::subtext0()),
        bright_black_background: Some(hex(palette::surface0())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_becomes_sgr_mouse_reports() {
        assert_eq!(wheel_report(1), b"\x1b[<65;1;1M".to_vec());
        assert_eq!(wheel_report(-1), b"\x1b[<64;1;1M".to_vec());
        assert_eq!(wheel_report(2).len(), 20);
        assert!(wheel_report(0).is_empty());
        // 慣性で積み上がっても上限で頭打ち。
        assert_eq!(wheel_report(999).len(), 10 * WHEEL_REPORT_LIMIT as usize);
    }

    #[test]
    fn reordering_reads_the_front_window_so_it_can_be_put_back() {
        // 並べ替えは前面を手放す(tmux は最後に動かした窓を前に出す)ので、
        // 控えを取れることが前提になる。
        let listing = "1\t@1\t0\n2\t@2\t1\n3\t@3\t0\n";
        assert_eq!(active_window_id(listing), Some("@2"));
        assert_eq!(active_window_id("1\t@1\t0\n"), None);
    }

    #[test]
    fn reordering_parks_every_window_before_it_fills_the_old_numbers() {
        let listing = "1\t@1\t0\n2\t@2\t1\n3\t@3\t0\n";
        let order = ["armature:3", "armature:1", "armature:2"].map(String::from);
        // 退避(最大+1 から)→ 元の番号へ落とし直し、の二段。
        assert_eq!(
            reorder_moves(listing, &order),
            Some(vec![
                ("@3".into(), 4),
                ("@1".into(), 5),
                ("@2".into(), 6),
                ("@3".into(), 1),
                ("@1".into(), 2),
                ("@2".into(), 3),
            ])
        );
    }

    #[test]
    fn reordering_keeps_its_hands_off_unless_the_new_order_covers_every_window() {
        let listing = "0\t@1\t1\n1\t@2\t0\n";
        let same = ["armature:0", "armature:1"].map(String::from);
        // 並びが変わっていないなら動かさない。
        assert_eq!(reorder_moves(listing, &same), None);
        // 名簿由来の行(この窓のタブでない)が混ざって数が足りない・
        // 知らない窓を指している——どちらも触らない。
        let short = ["armature:1".to_string()];
        assert_eq!(reorder_moves(listing, &short), None);
        let stranger = ["armature:1", "armature:9"].map(String::from);
        assert_eq!(reorder_moves(listing, &stranger), None);
        let doubled = ["armature:1", "armature:1"].map(String::from);
        assert_eq!(reorder_moves(listing, &doubled), None);
        assert_eq!(reorder_moves("", &same), None);
    }

    #[test]
    fn the_server_environment_gives_up_only_the_parent_session_markers() {
        let listing = "AI_AGENT=claude-code\nCLAUDE_CODE_CHILD_SESSION=1\n-CLAUDE_CODE_SESSION_ID\nPATH=/usr/bin\nANTHROPIC_API_KEY=x\n";
        assert_eq!(
            server_environment_markers(listing),
            ["AI_AGENT", "CLAUDE_CODE_CHILD_SESSION"]
        );
    }

    #[test]
    fn the_tmux_identity_is_separate_from_the_running_cockpit() {
        assert_eq!(tmux_label(), "armature");
        assert_ne!(tmux_label(), "inner");
    }

    #[test]
    fn the_terminal_child_drops_the_markers_through_env_without_touching_the_app() {
        let launch = Launch {
            program: "/opt/homebrew/bin/tmux".into(),
            args: vec!["-L".into(), "armature".into(), "attach-session".into()],
            working_directory: PathBuf::from("/tmp"),
        };
        let child_env = armature_core::proc::ChildEnv {
            remove: vec!["CLAUDECODE".into(), "NO_COLOR".into()],
            set: vec![("PATH".into(), "/a:/b".into()), ("LANG".into(), "en_US.UTF-8".into())],
        };
        let (program, args, env) = pty_command(&launch, &child_env);
        assert_eq!(program, "/usr/bin/env");
        assert_eq!(
            args,
            [
                "-u",
                "CLAUDECODE",
                "-u",
                "NO_COLOR",
                "/opt/homebrew/bin/tmux",
                "-L",
                "armature",
                "attach-session"
            ]
        );
        assert_eq!(env.get("PATH").map(String::as_str), Some("/a:/b"));
        assert_eq!(env.get("LANG").map(String::as_str), Some("en_US.UTF-8"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        // 落とすものが無ければ挟まない。
        let (program, args, _) = pty_command(&launch, &armature_core::proc::ChildEnv::default());
        assert_eq!(program, launch.program);
        assert_eq!(args, launch.args);
    }

    #[test]
    fn a_home_relative_session_directory_is_expanded() {
        let got = expand_home("~/work");
        assert!(got.ends_with("work"));
        assert!(!got.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn the_active_tmux_window_is_selected_for_closing() {
        // 印の欄が付いた形。印の無い窓(素のシェル)も混ざる。
        let windows = "@1 0 aaa\n@3 1 bbb\n@5 0 \n";
        assert_eq!(active_window(windows), Some("@3"));
        assert_eq!(active_session(windows), Some("bbb"));
    }

    /// 素のシェルの窓には会話の印が無い。閉じても台帳へ刻むものが無い。
    #[test]
    fn a_plain_shell_window_has_no_session_to_stamp() {
        let windows = "@1 0 aaa\n@3 1 \n";
        assert_eq!(active_window(windows), Some("@3"));
        assert_eq!(active_session(windows), None);
    }

    #[test]
    fn local_tabs_use_the_dedicated_tmux_identity() {
        let raw = "other\t/dev/ttys001\t0\tignored\t1\t\tmac\t外\narmature\t/dev/ttys005\t2\tclaude\t1\tabc-123\tmac\t✳ 設計の続き\narmature\t/dev/ttys003\t1\tterminal\t0\t\tmac\t作業シェル\n";
        let tabs = parse_local_tabs(raw);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].target, "armature:1");
        assert_eq!(tabs[0].tty, "ttys003");
        assert_eq!(tabs[0].title, "作業シェル");
        assert!(tabs[0].named);
        assert!(!tabs[0].active);
        assert_eq!(tabs[0].session_id, None);
        assert_eq!(tabs[1].target, "armature:2");
        assert_eq!(tabs[1].title, "✳ 設計の続き");
        assert!(tabs[1].active);
        assert_eq!(tabs[1].session_id.as_deref(), Some("abc-123"));
        assert_eq!(session_ids_of(&tabs), vec!["abc-123".to_string()]);
    }

    #[test]
    fn pane_titles_override_window_names_and_keep_embedded_tabs() {
        let raw = "armature\t/dev/ttys001\t0\twindow-name\t1\t\tmac\t\narmature\t/dev/ttys002\t1\tignored\t0\tsid\tmac\t✳ pane\twith\ttab\n";
        let tabs = parse_local_tabs(raw);

        assert_eq!(tabs[0].title, "window-name");
        assert_eq!(tabs[1].title, "✳ pane\twith\ttab");
    }

    #[test]
    fn only_dedicated_numeric_tmux_targets_can_be_selected() {
        assert!(!select_tab("pid:123"));
        assert!(!select_tab("armature:1;kill-server"));
    }

    #[test]
    fn application_shortcuts_are_consumed_by_the_terminal_bindings() {
        use iced::keyboard::Modifiers;
        use iced_term::bindings::{BindingAction, InputKind};

        let bindings = app_shortcut_bindings();
        for key in [
            "w", "f", "n", "o", "r", "p", "x", "g", "s", "b", "h", "l", "/",
        ] {
            let shortcut = bindings
                .iter()
                .find(|(binding, _)| {
                    binding.target == InputKind::Char(key.into())
                        && binding.modifiers == Modifiers::COMMAND
                })
                .unwrap_or_else(|| panic!("Cmd+{key} must be reserved by the terminal binding"));
            assert_eq!(shortcut.1, BindingAction::Esc(String::new()));
        }
    }

    /// 素のタブはモデルも思考量も決め打ちしない。渡すのは全画面の1欄と、この窓の約束。
    #[test]
    fn ordinary_claude_tabs_use_the_users_own_defaults() {
        let args = plain_claude_arguments(
            Some(Path::new("/tmp/protocol.md")),
            Path::new("/tmp/tasks"),
        );
        assert!(!args.iter().any(|arg| arg == "--model" || arg == "--effort"));
        assert_eq!(
            args,
            [
                "--settings",
                r#"{"tui":"fullscreen"}"#,
                "--append-system-prompt-file",
                "/tmp/protocol.md",
                "--add-dir",
                "/tmp/tasks",
            ]
        );
        let settings: serde_json::Value = serde_json::from_str(&args[1]).unwrap();
        assert_eq!(settings, serde_json::json!({"tui": "fullscreen"}));
    }

    #[test]
    fn the_hostname_placeholder_never_names_a_tab() {
        assert_eq!(pane_or_window_title("mac.local", "mac.local", "3 Ship it"), "3 Ship it");
        assert_eq!(pane_or_window_title("", "mac.local", "claude"), "claude");
        assert_eq!(pane_or_window_title("✳ Fix the bug", "mac.local", "claude"), "✳ Fix the bug");
        assert_eq!(
            pane_or_window_title("Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA", "mac.local", "3 Ship it"),
            "3 Ship it"
        );
        assert!(!is_graphics_query("Go to the store"));
        assert!(!is_graphics_query("G"));
    }
}
