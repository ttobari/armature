//! セッション一覧(左の列のパネル)の値。窓のタブ1枚につき1行を組む。
//!
//! ```text
//!   窓のタブ(tmux の窓)──→ 1行 ← 行が在るかどうかはこれだけで決まる
//!        │ tty で照合
//!        └─ ps ─→ claude agents --json の1件 ─→ 会話ログ(モデル・effort・文脈・道具)
//!                                              └→ subagents/*.meta.json(いま動いている配下)
//! ```
//!
//! **読むだけ**。どの取得も「取れなければ `None`」で、呼び手は前回値を残す。
//!
//! 名簿(`claude agents --json`)だけが重い(node の起動込みで1秒級)。行の中身は
//! 窓のタブと会話ログから速く組めるので、名簿は付加情報として遅い刻で引く。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

/// PATH に頼らず実行ファイルを探す(Dock から起こした窓はログインシェルの PATH を
/// 持たない)。
#[must_use]
pub fn which(name: &str) -> Option<String> {
    // アプリ束に同梱した道具(`Contents/Helpers`)を最初に見る(tmux を同梱している)。
    if let Some(bundled) = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("Helpers").join(name)))
        .filter(|path| path.is_file())
    {
        return Some(bundled.to_string_lossy().into_owned());
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{home}/.local/bin/{name}"),
        format!("/opt/homebrew/bin/{name}"),
        format!("/usr/local/bin/{name}"),
        format!("/usr/bin/{name}"),
    ];
    for path in candidates {
        if std::fs::metadata(&path).is_ok_and(|m| m.is_file()) {
            return Some(path);
        }
    }
    // 最後に PATH も見る(手元で走らせるとき用)。
    let out = crate::proc::child_env(&mut Command::new("/usr/bin/which"))
        .arg(name)
        .output()
        .ok()?;
    let found = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!found.is_empty()).then_some(found)
}

/// タブの3値(タブ色のトレース)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabState {
    /// 動いている(緑+スピナー)。
    Working,
    /// 返事が来ていて未読(金+✳)。
    Unread,
    /// 読んだ(灰+○)。
    #[default]
    Read,
}

/// いま握ったままの道具(会話ログの末尾で結果が返っていない `tool_use`)。
///
/// 「動いている」だけでは何をしているか分からない——`claude agents` の busy は
/// 真偽値でしかなく、行に出せる中身を持たない。結果待ちの道具を1つ拾えば
/// 「Bash実行中: cargo test を走らせる」まで書ける。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// 道具の名前(`Bash` / `Edit` …)。
    pub tool: String,
    /// その手の中身。Bash は description(日本語)、編集・読みはファイル名、
    /// 委譲は description。取れなければ空。
    pub note: String,
}

/// いま動いているサブエージェント1体。
///
/// 呼び出しの木は取れない(`spawnDepth` と `teamName` はメタに残るが、孫が
/// 誰の下に居るかは辿れない)ので、**親セッションの直下に字下げで並べる**
/// ベストエフォート表示に留める。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agent {
    /// 名前(`d-session-list` など。無ければ agentType)。
    pub name: String,
    /// 何をさせているか(メタの description・日本語)。
    pub note: String,
    /// モデルの短名(`opus` など)。
    pub model: Option<String>,
    /// effort(`xhigh` など)。メタには無いので会話ログ側から取る。
    pub effort: Option<String>,
    /// 直近のリクエストが積んだ文脈(トークン)。
    pub context: Option<f64>,
    /// 文脈の残り(%)。窓を決めた後の値([`spread_windows`])。
    pub context_left: Option<f64>,
    /// 握ったままの道具。`Some` なら手が動いている。
    pub activity: Option<Activity>,
    /// 手番の途中か([`Snapshot::mid_turn`])。真なら**返事を負っている**
    /// ——道具は握っていないが、考えて字を組み立てている最中。
    pub mid_turn: bool,
    /// 発言の途中で切れているか([`Snapshot::mid_stream`])。
    pub mid_stream: bool,
    /// 最後に書き込んでからの経過(秒)。
    pub idle_sec: f64,
}

impl Agent {
    /// 生きているか。**画面の印(スピナーか○か)はこれで決める。**
    ///
    /// 道具の有無で決めてはいけない——8体を並列で走らせている最中に全行が
    /// ○(止まっている印)で出た(実測2026-08-13)。手が動いている時間より
    /// **考えている時間のほうが長い**ので、道具を握っている瞬間だけを
    /// 「動いている」と見なすと、走っている木がほとんど止まって見える。
    ///
    /// 判定は [`scan_agents`] の篩と同じ根([`Self::live_within`])を使う
    /// ——一覧に残す条件と印の条件がずれると、消える寸前の行だけ嘘をつく。
    #[must_use]
    pub fn live(&self) -> bool {
        self.live_within(AGENT_LIVE_WINDOW)
    }

    /// 生死の根。次のどれかを満たすものが生きている:
    ///
    /// 1. **道具を握ったまま結果を待っている**——ただし [`AGENT_BUSY_WINDOW`]
    ///    まで。握ったまま殺された木は戻りを書き残さないので、ここで落とす。
    /// 2. **道具は握っていないが手番の途中**([`Self::mid_turn`])——返事を
    ///    負ったまま考えている。こちらは [`AGENT_THINK_WINDOW`] まで。
    /// 3. **会話ログの最終更新が `live_window` 以内**。
    ///
    /// 2 が無いと、**長く考える木が一覧から消える**。時計で測るだけでは
    /// 「5分考えている」と「5分前に返して黙った」が同じ姿になり、`live_window`
    /// (60秒)を跨いだ側がまとめて落ちる——実測2026-08-18、記事を書く worker
    /// (`writing-first-post`)が道具の戻りを受けてから次の発言までに313秒黙り、
    /// その間ずっと枝が出ていなかった(会話ログの逐語は本文の但し書き参照)。
    /// 手番はログの形に出るので、時計ではなくそちらで見分ける。
    fn live_within(&self, live_window: f64) -> bool {
        // **握っている木は先に裁く。** 道具を握った行は「まだ返していない」
        // 行でもあるので、手番のほうで拾わせると窓が [`AGENT_THINK_WINDOW`]
        // まで広がり、握ったまま殺された木(ゾンビ)が15分居座る。
        if self.activity.is_some() {
            return self.idle_sec <= AGENT_BUSY_WINDOW.max(live_window);
        }
        // **発言の途中で切れた木は、考え込んでいる木より狭い窓で裁く。**
        // 手番の窓([`AGENT_THINK_WINDOW`])は「道具の戻りを受けてから最初の字を
        // 書き出すまで」の空白に合わせた幅で、書き出したあとの続きはそれよりずっと
        // 速い。広いほうで裁くと、思考の断片だけ書いて殺された木が15分居座る。
        if self.mid_stream {
            return self.idle_sec <= AGENT_STREAM_WINDOW.max(live_window);
        }
        if self.mid_turn && self.idle_sec <= AGENT_THINK_WINDOW {
            return true;
        }
        self.idle_sec <= live_window
    }
}

/// 1つの Claude セッション(tmux のペインに紐付いたものだけ)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    pub tab_state: TabState,
    pub title: String,
    /// 最後に動いてからの経過(秒)。取れなければ `None`。
    pub idle_sec: Option<f64>,
    /// 切替の宛先("armature:1")。
    pub target: String,
    /// 中央ペインにいま表示されているタブか。
    pub active: bool,
    /// モデルの短名(`fable` / `opus` / `sonnet`…)。取れなければ `None`。
    pub model: Option<String>,
    /// effort(`low` / `medium` / `high` / `xhigh` / `max`)。
    pub effort: Option<String>,
    /// 直近のリクエストが積んだ文脈(トークン)。
    pub context: Option<f64>,
    /// 文脈の**残り**(%)。窓を決めた後の値([`spread_windows`])。
    ///
    /// **欄は残り・画面は使用量。** 一覧は `100 - left` に直して「78%」と出す。
    /// ここへ入れる値は残りのままにすること。
    pub context_left: Option<f64>,
    /// `claude agents` が busy と言っているか。
    pub busy: bool,
    /// 握ったままの道具。
    pub activity: Option<Activity>,
    /// いまそのパネルで走っている会話のID(`claude agents` が言う `sessionId`)。
    ///
    /// **窓が起こすときに振った ID とは別物になりうる。** タブの中で `/resume`
    /// を叩けば同じ tty のまま別の会話へ移るので、窓が覚えている振った ID は
    /// そこで嘘になる——「次の起動で開き直す会話」を決めるのはこちら。
    pub session_id: Option<String>,
    /// いま動いているサブエージェント(親の直下に字下げで出す)。
    ///
    /// **稼働中だけ**が入る。一度は木を丸ごと畳んだが、
    /// 「実際に動いているものは出してほしい」と一部が撤回された同日の指示で
    /// 稼働中だけを戻した——止まっているものは入らない([`live_agents`])。
    pub agents: Vec<Agent>,
}

/// Claude がペインタイトルの頭に書く状態マーク(✳ や回転中のブライユ点字)を剥がす。
/// 状態はこちらが印で出すので、タイトルからは削る。
#[must_use]
pub fn strip_title_mark(title: &str) -> String {
    title
        .trim_start_matches(|c: char| {
            c.is_whitespace()
                || ('\u{2800}'..='\u{28FF}').contains(&c) // ブライユ点字
                || ('\u{2700}'..='\u{27BF}').contains(&c) // 装飾記号
                || ('\u{25D0}'..='\u{25D3}').contains(&c) // 半月(◐◑◒◓)の回転
                || matches!(c, '·' | '•' | '●' | '◦' | '*')
        })
        .trim()
        .to_string()
}

/// 名乗りの頭のマークが示す状態。読めなければ `None`。
///
/// **色の源はここ**。以前は窓が自分の PTY に出力が届いたかで見ていたが、
/// あれは1文字描かれるだけで立って消えるので、行が緑や黄へ瞬いて戻る。マークは
/// Claude Code 自身が「いま何をしているか」を窓の名前へ書いたもので、手が動いて
/// いる間だけ回り、返事が出揃うと `✳` に変わる。
///
/// 読むのは**手が動いている印だけ**(点字 `⠋` と半月 `◐` の回転)。
///
/// **`✳` は未読にしない。** あれは「返事が出揃った」の印で、利用者が読んだかどうかを
/// 持たない——見ても消えないので、これを未読へ写すと黄が居座る。読んだかを知っているのは
/// 窓の側(いまどのタブを見ているか)だけなので、未読はそちらの見立てに任せる。
#[must_use]
pub fn title_state(title: &str) -> Option<TabState> {
    for c in title.chars() {
        if c.is_whitespace() {
            continue;
        }
        if ('\u{2800}'..='\u{28FF}').contains(&c) || ('\u{25D0}'..='\u{25D3}').contains(&c) {
            return Some(TabState::Working);
        }
        // 頭の1字が回転の印でなければ、この名乗りは状態を持たない。
        return None;
    }
    None
}

/// 外のコマンドを待つ上限。
///
/// **性能の調整値ではなく、固まらないための命綱**。ここに時間切れが無いと、
/// 外のコマンドが1つ帰ってこないだけで取得の糸が永久に止まり、一覧が最後に
/// 描いた姿のまま貼り付く。`claude agents --json` の8秒級に倍の余裕を置いた値。
const CMD_TIMEOUT: Duration = Duration::from_secs(20);

/// コマンドを走らせて標準出力を返す。失敗と時間切れは `None`。
///
/// 標準出力は**別スレッドで読み切る**。`wait` してから読む形にすると、出力が
/// パイプの緩衝(64KB)を超えた時点で子が書き込み待ちに入り、親は終了待ちで
/// 止まって噛み合う——`ps -Ao` は数百行あり、その大きさを普通に超える。
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    use std::process::Stdio;
    let mut child = crate::proc::child_env(&mut Command::new(cmd))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut pipe = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let read = std::io::Read::read_to_end(&mut pipe, &mut buf).is_ok();
        let _ = tx.send(read.then_some(buf));
    });
    let Ok(Some(bytes)) = rx.recv_timeout(CMD_TIMEOUT) else {
        // 時間切れ(か読み損ね)。**刈ってから引き取る**——放っておくと
        // 子が居座り、刻むたびに1つずつ増える。
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    if !child.wait().ok()?.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).to_string())
}

/// `ps` 1回ぶん。
///
/// tty を持たない pid も `alive` には入れる。この2つを分けているのは、
/// **生きているか**と**どの端末に居るか**が別の問いだから——一覧に載せるか
/// どうかは前者で決め、どのタブの行かは後者で決める。片方だけで判じると、
/// tty を持たないセッション(D が起こした PTY の外・裸で起きたもの)が
/// 「死んだ」と読まれて消える。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ps {
    /// tty を持つ pid だけの対応表。
    pub ttys: BTreeMap<String, String>,
    /// いま走っている pid 全部。
    pub alive: std::collections::BTreeSet<String>,
    /// そのうち `claude` そのものの pid。
    ///
    /// **名簿を取り直す合図**に使う。`claude agents --json` は8秒級で15秒ごとに
    /// は叩けないが、`ps` なら数msで済む——ここが増えた回だけ名簿を引き直せば、
    /// 新しいセッションが1リフレッシュで一覧に載る。
    pub claude: std::collections::BTreeSet<String>,
}

/// `ps` を1回叩いて pid の生死と tty を取る。取れなければ `None`。
#[must_use]
pub fn ps_snapshot() -> Option<Ps> {
    Some(parse_ps(&run("/bin/ps", &["-Ao", "pid=,tty=,comm="])?))
        .filter(|ps: &Ps| !ps.alive.is_empty())
}

/// `ps -Ao pid=,tty=,comm=` の出力を割る。
///
/// `comm` は空白を含み得る(`/Applications/…/Claude Helper`)ので必ず末尾の
/// 列に置き、分割は2回までに留める。
#[must_use]
pub fn parse_ps(raw: &str) -> Ps {
    let mut out = Ps::default();
    for line in raw.lines() {
        let mut parts = line.trim_start().splitn(3, char::is_whitespace);
        let Some(pid) = parts.next().filter(|p| !p.is_empty()) else {
            continue;
        };
        out.alive.insert(pid.to_string());
        let Some(tty) = parts.next() else { continue };
        if tty != "??" {
            out.ttys.insert(pid.to_string(), tty.to_string());
        }
        // 実行ファイルの名前がちょうど `claude` のものだけを数える。
        // Claude.app(`… /MacOS/Claude Helper`)は端末のセッションではない。
        if parts.next().is_some_and(|comm| comm.trim() == "claude") {
            out.claude.insert(pid.to_string());
        }
    }
    out
}

/// 窓が抱えているタブ1枚(このアプリ専用の tmux の窓)。
///
/// **アプリは自分のタブを知っている**。窓の側から「この tty はうちの N 番のタブ」と
/// 渡してもらうのが最短の配線になる。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalTab {
    /// この PTY の子側の名前(`ttys010`)。`ps` の tty と突き合わせる鍵。
    pub tty: String,
    /// 切替の宛先(`armature:3`)。
    pub target: String,
    /// タブに出している名前。
    pub title: String,
    /// その名前が**名乗られたもの**か(`false` は起こしたコマンド名の代役)。
    ///
    /// 一覧の主語をどこから採るかの分かれ目。名乗りがあるタブはその字が利用者の
    /// 見ている名前なので最優先、無いタブは名簿(`claude agents`)が持つ会話の
    /// 題を借りる。どちらも無ければコマンド名のまま。
    pub named: bool,
    /// いま見ているタブか。
    pub active: bool,
    /// 見ていない間に出力が届いた。
    pub unread: bool,
    /// そのタブが乗っている会話のID(窓が振った/開き直した鍵)。
    ///
    /// **これが在れば名簿を待たない。** 会話ログはモデル・effort・文脈・経過の
    /// 出どころだが、在り処を引くのに名簿(`claude agents --json`・8秒級)の
    /// `sessionId` を使っていた。窓を開き直した直後は新しい pid の名簿が
    /// 届くまで**モデルも effort も空欄**になる。
    /// 窓は自分が何で起こしたかを知っているので、それを先に使う。
    pub session_id: Option<String>,
    /// **いま動いている**(直前まで出力が届き続けている)。
    ///
    /// 出どころは窓が抱える PTY そのもの——`claude agents --json` の `busy` は
    /// 名簿の遅い刻(60秒)でしか来ないので、走り出した/終わったが分に近い遅れで
    /// 出ていた。claude は手を動かして
    /// いる間だけ画面を書き替えるので、**出力が止まったかどうか**が状態の
    /// いちばん速い代理になる。
    pub busy: bool,
}

/// `claude agents --json` の1件。ペインとの照合はまだしていない。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Running {
    /// プロセスID(tty 経由でペインに繋ぐ鍵)。
    pub pid: String,
    /// 会話ログを引くための鍵。
    pub session_id: String,
    /// Claude が付けた名前(ペインタイトルが空のときの代役)。
    pub name: String,
    /// いま手が動いているか。
    pub busy: bool,
}

/// 起動中セッションの名簿を取る。
///
/// **これだけが重い**——`claude agents --json` は node の起動を含めて1秒級。
/// 名簿そのものはセッションを立てるか閉じるまで変わらないので遅い周期で取る。
#[must_use]
pub fn fetch_running() -> Option<Vec<Running>> {
    let claude = which("claude")?;
    let raw = run(&claude, &["agents", "--json"])?;
    let agents: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(
        agents
            .as_array()?
            .iter()
            .filter_map(|agent| {
                Some(Running {
                    pid: agent["pid"].as_i64()?.to_string(),
                    session_id: agent["sessionId"].as_str().unwrap_or_default().to_string(),
                    name: agent["name"].as_str().unwrap_or_default().to_string(),
                    busy: agent["status"].as_str() == Some("busy"),
                })
            })
            .collect(),
    )
}

/// 窓のタブから一覧の行を組む。`ps` が読めなかった回だけ `None`
/// (呼び手は前回表示を残す)。
#[must_use]
pub fn build_sessions(running: &[Running], tabs: &[LocalTab], now: f64) -> Option<Vec<Session>> {
    Some(assemble_tabs(tabs, &ps_snapshot()?, running, now))
}

/// 窓のタブから一覧の行を組む。**1タブ=1行**。
///
/// ```text
///   窓のタブ(LocalTab)──→ 1行 ← 行が在るかどうかはこれだけで決まる
///        │ tty で照合
///        └─ ps ─→ claude agents の1件 ─→ 会話ログ(モデル・effort・文脈・道具)
/// ```
///
/// 外で動いているセッション(別の端末アプリ等)は**一切載らない**——タブが無ければ
/// 行も無い。claude が走っていないタブ(素のシェル)も1行として出す:
/// 利用者が見ているのは「窓のタブ」で、中身が何かは行の有無を決める話ではない。
///
/// タブの行の下には**稼働中のサブエージェント**をぶら下げる([`live_agents`])。
/// 走らせているものが1体も無いセッションでは行が1本も増えず、絵は前と変わらない。
///
/// ## なぜ速い刻に置くか
///
/// この関数は速い刻(動いている間は2秒・タブを切り替えた回は最短400ms)で
/// 回る。遅い刻(60秒)へ逃がすほうが軽いが、それでは**出ている木の大半が
/// 既に終わったもの**になる——動いている・止まったは秒で入れ替わるもので、
/// 分の刻で見せる値ではない。重さのほうは走査の作りで抑える: 篩は安いもの
/// (`read_dir` と `mtime`)から順に掛け、末尾読みは最終更新が動いた体だけに
/// 落とす([`AGENT_MEMO`])。
#[must_use]
pub fn assemble_tabs(locals: &[LocalTab], ps: &Ps, running: &[Running], now: f64) -> Vec<Session> {
    // 設定が明示する窓。1行ごとに読み直さず、この刻で1回だけ拾う。
    let known_windows = known_model_windows();
    let mut rows: Vec<(Option<Log>, Session)> = locals
        .iter()
        .map(|tab| {
            // そのタブの tty に居る claude。素のシェルのタブでは見つからない。
            let claude = running
                .iter()
                .find(|r| ps.ttys.get(&r.pid).is_some_and(|tty| *tty == tab.tty));
            // タブ自身の鍵を先に使う(名簿の到着を待たない)。名簿は
            // タブの中で `/resume` された後の**移った先**を知っているので、
            // 在れば上書きさせる。
            //
            // 会話のIDと在り処は**組で持つ**。サブエージェントは
            // `<会話ログの隣>/<会話ID>/subagents/` に居るので、在り処を名簿から・
            // IDをタブから、と別々に取ると別の会話の木を覗きにいく。
            let log: Option<Log> = [
                claude.map(|r| r.session_id.as_str()),
                tab.session_id.as_deref(),
            ]
            .into_iter()
            .flatten()
            .find_map(|id| {
                Some(Log {
                    id: id.to_string(),
                    path: transcript_path(id)?,
                })
            });
            let transcript = log.as_ref().map(|log| log.path.clone());
            let snap = transcript.as_deref().map(read_snapshot).unwrap_or_default();
            // **窓の申告だけで決める。** 名簿(`claude agents`)の busy は遅い刻
            // (最長60秒)でしか来ないので、`or` で足すと**止めたのに緑のまま**に
            // なる。窓は自分の PTY を見ているので、こちらが速くて正しい。
            // **名簿の busy だけでは足りない。** `claude agents` は背景に残った
            // シェル1本でも busy と言うので、応答を返し終えたパネルが起動中に見えた。
            // 会話ログの末尾で手番を読み、返し終えているパネルでは落とす。ログの姿が
            // 読めないパネル(AIの発言が1つも取れない)では名簿をそのまま信じる。
            let turn_alive = snap.model.is_none() || snap.activity.is_some() || snap.mid_turn;
            let busy = tab.busy && turn_alive;
            // 行の主語はセッションの**題**。タブが名乗っていればその字(盤から
            // 起こしたタブのタスク名・`--name`・子が OSC で言った題)、名乗って
            // いなければ名簿が持つ会話の題を借りる。どちらも無ければタブの字
            // (起こしたコマンド名)のまま。
            //
            // 名乗りは [`strip_title_mark`] に通す。Claude がタブ名の頭に書く
            // 状態マーク(`◐` の回転・`✳`)は、こちらが左端の印で同じことを
            // 出している以上ただの飾りで、題の前に意味の読めない字が1つ挟まる
            // だけになる。
            let mut title = tab
                .named
                .then(|| strip_title_mark(&tab.title))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| {
                    claude
                        .map(|r| r.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| strip_title_mark(&tab.title))
                });
            // Claude は名乗る題を約20字で「…」に切る。全文は会話ログの
            // custom-title 記録にあるので、切られているときだけ引いて戻す。
            if title.ends_with('…')
                && let Some(full) = claude.and_then(|r| custom_title(&r.session_id))
                && completes_truncated_title(&title, &full)
            {
                title = full;
            }
            let session = Session {
                // **名乗りのマークが正**([`title_state`])。読めない名乗り
                // (素のシェル・盤から起こした固定札)だけ窓の見立てへ落ちる。
                tab_state: title_state(&tab.title).unwrap_or(if busy {
                    TabState::Working
                } else if tab.unread {
                    TabState::Unread
                } else {
                    TabState::Read
                }),
                title,
                idle_sec: transcript.as_deref().and_then(mtime).map(|s| now - s),
                target: tab.target.clone(),
                active: tab.active,
                model: snap.model,
                effort: snap.effort,
                context: snap.context,
                context_left: None, // apply_windows が入れる
                busy,
                activity: snap.activity,
                session_id: claude.map(|r| r.session_id.clone()),
                // 木は行が出揃ってから載せる([`attach_agents`])——同じ会話を
                // 2つの行が指すことがあり、1行ずつ組む場では気付けない。
                agents: Vec::new(),
            };
            (log, session)
        })
        .collect();
    attach_agents(&mut rows, now, AGENT_LIVE_WINDOW);
    rows.into_iter()
        .map(|(_, mut session)| {
            // 木を載せ**終えてから**通す——窓の決め方は配下のサブエージェントにも
            // 降りる([`spread_windows_with`])。
            apply_windows(&mut session, &known_windows);
            session
        })
        .collect()
}

/// 行が指している会話(ID と会話ログの在り処)。
///
/// サブエージェントは `<会話ログの隣>/<会話ID>/subagents/` に居るので、
/// **IDと在り処は組で持つ**——在り処を名簿から・IDをタブから、と別々に取ると
/// 別の会話の木を覗きにいく。
struct Log {
    id: String,
    path: PathBuf,
}

/// 木を**会話1つにつき1本の行**へ載せる。
///
/// 同じ会話を2つの行が指すことがある。`claude agents --json` が1つの会話に
/// 2つのプロセスを返すからで(2026-08-13実測: pid 45142 と 77987 がどちらも
/// 会話 `e12e03c1` を名乗っていた——古いほうが生きたまま、別のタブが `/resume`
/// で同じ会話へ入り直した形)、木は会話に紐付くので**同じ5体が2つの行に
/// ぶら下がる**。窓のタブから組む経路でも、同じ会話を2枚のタブで開けば同じ絵に
/// なる。
///
/// 持ち主は〈動いている行〉を先に、並んだら〈上の行〉。動いていないほうは同じ
/// 会話を覗いている2枚目で、そちらに木を出しても嘘になる——実際に走らせて
/// いるのは動いている側。
///
/// 走査も会話ごとに1回で済む(2つの行が同じディレクトリを2度読むことがない)。
fn attach_agents(rows: &mut [(Option<Log>, Session)], now: f64, live_window: f64) {
    let mut owner: BTreeMap<String, usize> = BTreeMap::new();
    for i in 0..rows.len() {
        let Some(log) = &rows[i].0 else { continue };
        // 先に置かれた行を退けるのは、あちらが止まっていてこちらが動いている
        // ときだけ。
        let take = owner
            .get(&log.id)
            .is_none_or(|&prev| !rows[prev].1.busy && rows[i].1.busy);
        if take {
            owner.insert(log.id.clone(), i);
        }
    }
    for i in owner.into_values() {
        let Some(log) = &rows[i].0 else { continue };
        let agents = scan_agents(&log.path, &log.id, now, live_window);
        rows[i].1.agents = agents;
    }
}

/// 会話ログの在り処。`~/.claude/projects/<プロジェクト>/<セッションID>.jsonl`
/// ——プロジェクト名は cwd をエンコードした文字列で、規則を再現するより
/// 総当たりのほうが壊れない(ディレクトリ数は数百で、開くのは1つ)。
#[must_use]
pub fn transcript_path(session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() {
        return None;
    }
    let home = std::env::var("HOME").ok()?;
    let projects = PathBuf::from(home).join(".claude/projects");
    let entries = std::fs::read_dir(projects).ok()?;
    for dir in entries.flatten() {
        let path = dir.path().join(format!("{session_id}.jsonl"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// ファイルの最終更新(UNIX秒)。
fn mtime(path: &std::path::Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_secs_f64())
}

/// 会話ログの末尾から読む「いまの姿」。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// モデルの短名。
    pub model: Option<String>,
    /// effort。
    pub effort: Option<String>,
    /// 直近のリクエストが積んだ文脈(トークン)。
    pub context: Option<f64>,
    /// 結果待ちの道具。
    pub activity: Option<Activity>,
    /// **手番の途中か**——AIがまだ返していない状態。道具の戻りを受けた直後・
    /// 親から声を掛けられた直後・考えているだけの発言を書いた直後が真で、
    /// `text` を書き終えた(返した)ところで偽になる。
    ///
    /// 親でも測る。親のログに混ざる傍流(サブエージェントの発言・その道具の
    /// 戻り)は落としてから読むので、親の手番だけが残る。
    pub mid_turn: bool,
    /// **発言の途中で切れているか**——最後の発言が思考だけで、字も道具の呼び出しも
    /// 持たない状態。書き出しの続きが来ないまま止まった木(殺された木)がこの形で
    /// 残るので、[`mid_turn`](Self::mid_turn) より狭い窓で裁く。
    pub mid_stream: bool,
}

/// 会話ログを末尾から読む量。**全文は読まない**——1本が10MBを超えるものが
/// 常にあり、10本を15秒ごとに舐めれば盤のほうが重くなる。末尾256KBなら
/// 直近の数十行が入り、1本あたり1ms前後で済む(2026-08-12実測)。
pub const TAIL_BYTES: u64 = 256 * 1024;
/// 読み直しの上限。1件でMB級になる道具の戻りの後は、256KBの中にAIの発言が
/// 1つも入らない——そのときだけここまで広げる(常時広く読むと1本120ms級)。
pub const TAIL_BYTES_MAX: u64 = 2 * 1024 * 1024;

/// 既定の文脈窓。
pub const CONTEXT_WINDOW: f64 = 200_000.0;
/// 1M窓。
pub const CONTEXT_WINDOW_1M: f64 = 1_000_000.0;

/// Claude Code の設定が明示するモデル系列と窓。
///
/// `message.model` は `claude-opus-5` までしか残らないが、公式の設定値は
/// `opus[1m]` のように拡張窓をモデル名へ付ける。**印が実在するときだけ**
/// 1Mの証拠にする。単なる `opus` や未知の修飾子から窓を想像しない。
fn configured_window_of(value: &serde_json::Value) -> Option<(String, f64)> {
    let raw = value["model"].as_str()?.trim();
    let lower = raw.to_ascii_lowercase();
    if !lower.ends_with("[1m]") {
        return None;
    }
    let base = raw.get(..raw.len().checked_sub(4)?)?.trim();
    if base.is_empty() {
        return None;
    }
    Some((model_short(base), CONTEXT_WINDOW_1M))
}

/// ユーザー設定から、明示された拡張窓だけを読む。
///
/// 読むのは `CLAUDE_CONFIG_DIR/settings.json` 1本(未指定なら
/// `~/.claude/settings.json`)。プロジェクト全走査や会話ログ全読込はしない。
fn configured_model_window() -> Option<(String, f64)> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))?;
    let text = std::fs::read_to_string(base.join("settings.json")).ok()?;
    configured_window_of(&serde_json::from_str::<serde_json::Value>(&text).ok()?)
}

/// この機体で分かっているモデルごとの窓。族の既定を土台に、設定の明示で上書きする。
fn known_model_windows() -> BTreeMap<String, f64> {
    let mut windows: BTreeMap<String, f64> = FAMILY_WINDOWS
        .into_iter()
        .map(|(model, window)| (model.to_string(), window))
        .collect();
    if let Some((model, window)) = configured_model_window() {
        windows.insert(model, window);
    }
    windows
}

/// 印が無くても窓が分かっている族。
///
/// `[1m]` の印は設定にしか出ず、**Fable は印を付けずに立てても 1M で開く**。
/// 推定は「200kを超えて積めた実績」でしか 1M を認めないので、印の無いパネルでは
/// 200k 窓と誤る(187k 積んだ Fable を 97% と報せた。本人の `/context` は 19%)。
/// 設定が在ればそちらが上書きするので、ここは土台。
const FAMILY_WINDOWS: [(&str, f64); 1] = [("fable", CONTEXT_WINDOW_1M)];

/// 積んだ文脈から窓の大きさを当てる。
///
/// 窓の値は会話ログのどこにも書かれていない——`claude agents` にもプロセスの
/// 引数にも無く、`message.model` の綴りも `claude-opus-5` までで `[1m]` の印は
/// 落ちている。ここから分かるのは「200kを超えて積めているなら1M窓」という
/// 一方向だけなので、それを [`spread_windows`] で兄弟へ広げて補う。
#[must_use]
pub fn context_window(used: f64) -> f64 {
    if used > CONTEXT_WINDOW {
        CONTEXT_WINDOW_1M
    } else {
        CONTEXT_WINDOW
    }
}

/// 文脈の残り(%)。
#[must_use]
pub fn context_left_pct(used: f64, window: f64) -> f64 {
    if window <= 0.0 {
        return 0.0;
    }
    (100.0 * (1.0 - used / window)).clamp(0.0, 100.0)
}

/// 1セッション(親+その配下)の窓をまとめて決め、残り%を書き込む。
///
/// 単独では「200kを超えていない=200k窓」としか言えず、1M窓のまま100kしか
/// 積んでいないセッションを「残50%」と誤報する。同じセッションで同じモデルの
/// 誰かが200kを超えていれば、そのモデルは1M窓だと分かる——兄弟は同じ設定で
/// 立つので、この伝播で実測の誤報(残0%と出た2026-08-12の下見)が消える。
pub fn spread_windows(session: &mut Session) {
    spread_windows_with(session, None);
}

/// [`spread_windows`] に「このパネルの窓は本当はこれ」という手掛かりを差し込む版。
///
/// 同じパネルで立った子は親と同じ設定で立つので、親の窓が1Mだと分かった時点で
/// 親と同じモデルの子も1M窓として読める。
pub fn spread_windows_with(session: &mut Session, window_hint: Option<f64>) {
    let mut wide: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    if window_hint.is_some_and(|w| w > CONTEXT_WINDOW)
        && let Some(model) = session.model.as_deref()
    {
        wide.insert(model);
    }
    let rows = std::iter::once((session.model.as_deref(), session.context))
        .chain(session.agents.iter().map(|a| (a.model.as_deref(), a.context)));
    for (model, used) in rows {
        if let (Some(model), Some(used)) = (model, used)
            && used > CONTEXT_WINDOW
        {
            wide.insert(model);
        }
    }
    let wide: std::collections::BTreeSet<String> =
        wide.into_iter().map(ToString::to_string).collect();
    let left = |model: Option<&String>, used: Option<f64>| {
        let used = used?;
        let window = if model.is_some_and(|m| wide.contains(m)) {
            CONTEXT_WINDOW_1M
        } else {
            context_window(used)
        };
        Some(context_left_pct(used, window))
    };
    session.context_left = left(session.model.as_ref(), session.context);
    for agent in &mut session.agents {
        agent.context_left = left(agent.model.as_ref(), agent.context);
    }
}

/// 1セッションの文脈欄を確定させる。分かっている窓([`known_model_windows`])は
/// モデルの族まで一致したときだけ手掛かりにする——設定が `opus[1m]` でも、
/// ログ上のセッションが sonnet なら適用しない。
fn apply_windows(session: &mut Session, windows: &BTreeMap<String, f64>) {
    let known_window = session
        .model
        .as_deref()
        .and_then(|model| windows.get(model).copied());
    spread_windows_with(session, known_window);
}

/// モデルIDを短名へ。`claude-fable-5` → `fable`、`claude-opus-5[1m]` → `opus`。
/// 知らない名前はそのまま返す(色は既定へ倒れる)。
#[must_use]
pub fn model_short(raw: &str) -> String {
    model_family(raw).unwrap_or_else(|| raw.to_string())
}

/// 綴りの中に族の名があれば、それ。無ければ `None`
/// ——`model_short` と違い、**当たらなかったことを言える**。
#[must_use]
pub fn model_family(raw: &str) -> Option<String> {
    const FAMILIES: [&str; 5] = ["fable", "mythos", "opus", "sonnet", "haiku"];
    let lower = raw.to_ascii_lowercase();
    FAMILIES
        .iter()
        .find(|name| lower.contains(*name))
        .map(|name| (*name).to_string())
}

/// `/model` の控えから、打ち替え後のモデルを読む。
///
/// 控えは飾りの制御文字を含む(`Set model to \u001b[1mFable 5\u001b[22m and saved…`)
/// ので、綴りの中から族の名だけを拾う。族に当たらない字は捨てる——`/model` の
/// 文面が変わったときに、知らない字をそのまま桝へ流し込まないため。
fn declared_model(line: &str) -> Option<String> {
    let rest = line
        .split_once("<local-command-stdout>Set model to ")?
        .1;
    model_family(rest.split(" and saved").next().unwrap_or(rest))
}

/// `/effort` の控えから、打ち替え後の effort を読む。
fn declared_effort(line: &str) -> Option<String> {
    const LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
    let rest = line
        .split_once("<local-command-stdout>Set effort level to ")?
        .1;
    let word: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    LEVELS.contains(&word.as_str()).then_some(word)
}

/// 道具の引数から行に出す1語を選ぶ。
///
/// Bash は description、編集・読みはファイル名だけ
/// ——パスを丸ごと出すと行がパスで埋まって題が消える。
#[must_use]
pub fn tool_note(tool: &str, input: &serde_json::Value) -> String {
    let text = |key: &str| input[key].as_str().unwrap_or_default().to_string();
    let base = |key: &str| {
        let raw = text(key);
        raw.rsplit('/').next().unwrap_or(&raw).to_string()
    };
    let note = match tool {
        "Read" | "Edit" | "Write" | "NotebookEdit" => base("file_path"),
        "Grep" | "Glob" => text("pattern"),
        "WebFetch" => text("url"),
        "WebSearch" => text("query"),
        "Skill" => text("skill"),
        _ => text("description"),
    };
    if !note.is_empty() {
        return note;
    }
    // 説明を持たない回の最後の頼み。コマンドは**前置きを捨てて主文だけ**に
    // する——`cd ~/dev/… && cargo test` は前半が全部同じで、行に出しても
    // どのセッションが何をしているかが読めない。
    let fallback = [
        text("command"),
        text("prompt"),
        base("file_path"),
        text("query"),
    ];
    let raw = fallback.into_iter().find(|s| !s.is_empty()).unwrap_or_default();
    let head = raw.lines().next().unwrap_or_default();
    head.rsplit("&&")
        .find(|part| !part.trim().is_empty())
        .unwrap_or(head)
        .trim()
        .to_string()
}

/// ファイルの末尾を読む(先頭の欠けた行は捨てる)。
///
/// 返すのは**バイトのまま**。ここで `String` に起こすと2MB級の読み直しで
/// 変換と確保だけが効いてくる——行ごとに検査するので、UTF-8として読むのは
/// 実際に中を見る行だけでいい。
/// セッションの完全な題。名乗り(約20字で「…」に切られる)の元の文。
///
/// Claude は題を `custom-title` 記録として会話ログへ全文で書く。記録は
/// 改題のたびに増え、位置が定まらない(先頭287KBに5件目がある実測あり)ので、
/// この関数だけ全文を読む——呼ばれるのは題が切られたタブだけで、走査は
/// 部分文字列の一致だけだから10MB級でも数msで済む。
#[must_use]
pub fn custom_title(session_id: &str) -> Option<String> {
    let path = transcript_path(session_id)?;
    last_custom_title(&std::fs::read(path).ok()?)
}

fn last_custom_title(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    text.lines()
        .rev()
        .filter(|line| line.contains("\"custom-title\""))
        .find_map(|line| {
            let record: serde_json::Value = serde_json::from_str(line).ok()?;
            (record["type"].as_str() == Some("custom-title"))
                .then(|| record["customTitle"].as_str().map(str::to_string))
                .flatten()
        })
        .filter(|title| !title.is_empty())
}

/// 名乗りが切られた題(「…」終わり)を、ログの全文で置き換えてよいか。
///
/// 切られる前の字がそのまま頭に残っているときだけ真——別の題へ挿げ替わる
/// 事故を防ぐ。
#[must_use]
pub fn completes_truncated_title(shown: &str, full: &str) -> bool {
    let Some(prefix) = shown.strip_suffix('…') else {
        return false;
    };
    !prefix.is_empty() && full.len() > shown.len() && full.starts_with(prefix)
}

fn read_tail(path: &std::path::Path, bytes: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut raw = Vec::new();
    if len > bytes {
        file.seek(SeekFrom::Start(len - bytes)).ok()?;
        file.read_to_end(&mut raw).ok()?;
        // 途中から読んだ1行目は壊れている。改行まで捨てる。
        let start = raw
            .iter()
            .position(|b| *b == b'\n')
            .map_or(raw.len(), |i| i + 1);
        raw.drain(..start);
    } else {
        file.read_to_end(&mut raw).ok()?;
    }
    Some(raw)
}

/// この行はAIの発言か(JSONに起こす価値があるか)。
fn is_assistant(line: &str) -> bool {
    line.contains(r#""type":"assistant""#) || line.contains(r#""type": "assistant""#)
}

/// この行は人側の発言か(道具の戻り・親からの声掛け)。どちらも「AIはまだ
/// 返していない」を意味するので、区別せず手番の印だけに使う。
fn is_user(line: &str) -> bool {
    line.contains(r#""type":"user""#) || line.contains(r#""type": "user""#)
}

/// 傍流(サブエージェント)の行か。**JSONに起こさずに見る**——人側の行は
/// 道具の戻りでMB級になることがあり、判定のために起こすと全体が重くなる。
fn is_sidechain(line: &str) -> bool {
    line.contains(r#""isSidechain":true"#) || line.contains(r#""isSidechain": true"#)
}

/// 行の中の `tool_use_id`(結果が返った道具の印)を拾う。
fn result_ids(line: &str) -> impl Iterator<Item = &str> {
    line.match_indices(r#""tool_use_id":""#).filter_map(|(at, key)| {
        let rest = &line[at + key.len()..];
        let end = rest.find('"')?;
        Some(&rest[..end])
    })
}

/// 戻ってきた道具を結果待ちの列から外す。
fn drop_returned(pending: &mut Vec<(String, Activity)>, line: &str) {
    for id in result_ids(line) {
        pending.retain(|(waiting, _)| waiting != id);
    }
}

/// 会話ログの末尾から model / effort / 文脈 / 結果待ちの道具を取る。
///
/// 結果待ちの判定は「窓の中に出た `tool_use` から、窓の中に出た `tool_result`
/// を引く」だけ。結果は必ず後に来るので、窓の外へ出た道具を取りこぼすことは
/// あっても、終わった道具を実行中と読むことはない。
#[must_use]
pub fn read_snapshot(path: &std::path::Path) -> Snapshot {
    snapshot(path, false)
}

/// サブエージェントの会話ログを読む。こちらは**全行が傍流**(`isSidechain`
/// は常に true)なので、親と同じ捨て方をすると何も残らない。
#[must_use]
pub fn read_agent_snapshot(path: &std::path::Path) -> Snapshot {
    snapshot(path, true)
}

fn snapshot(path: &std::path::Path, side: bool) -> Snapshot {
    let snap = snapshot_of(path, side, TAIL_BYTES);
    if snap.model.is_some() {
        return snap;
    }
    // 256KBの中にAIの発言が1つも無かった。直前の道具の戻りが大きい
    // (長いビルドログ・大きなファイルの読み)ときに起きるので、そのときだけ
    // 広げて読み直す——常に広く読むと1本120ms級になる(2026-08-12実測)。
    snapshot_of(path, side, TAIL_BYTES_MAX)
}

fn snapshot_of(path: &std::path::Path, side: bool, bytes: u64) -> Snapshot {
    let mut snap = Snapshot::default();
    let Some(text) = read_tail(path, bytes) else {
        return snap;
    };
    let mut pending: Vec<(String, Activity)> = Vec::new();
    for line in text.split(|byte| *byte == b'\n') {
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        // **手番の印を先に立てる。** 人側の行(道具の戻り・親からの声掛け)は、
        // AIの発言と読み違えられて下の分岐へ落ちることがある(戻りが会話ログを
        // 持ち帰った回)ので、分岐に入る前のここで一度だけ見る。AIの発言だった
        // 場合は、この下で中身を見てから上書きする。
        // 親のログには傍流(サブエージェント)の人側の行も落ちるので、
        // 親の手番を測るときはそれを除く。
        if is_user(line) && (side || !is_sidechain(line)) {
            snap.mid_turn = true;
            snap.mid_stream = false;
        }
        // **`/model` `/effort` の打ち替えは、次の発言まで assistant の行に出ない。**
        // 打った瞬間の控え(スラッシュコマンドの stdout)が今の値そのものなので、
        // 行の順に上書きさせる——後から発言があればそちらが勝つ。
        if let Some(model) = declared_model(line) {
            snap.model = Some(model);
        }
        if let Some(effort) = declared_effort(line) {
            snap.effort = Some(effort);
        }
        // 末尾の大半は道具の戻り(1件でMB級になる)。**JSONに起こさない**
        // ——ここで全行を Value にすると1本あたり25ms級になり、15秒ごとに
        // 10本読む形では効いてくる(2026-08-12実測: 286ms → 20ms級)。
        // 戻りから要るのは「どのIDが返ったか」だけなので、印だけを拾う。
        if !is_assistant(line) {
            drop_returned(&mut pending, line);
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            drop_returned(&mut pending, line);
            continue;
        };
        // 傍流(サブエージェントの発言が親のログにも落ちる版)は親の姿ではない。
        if !side && value["isSidechain"].as_bool() == Some(true) {
            continue;
        }
        // 行の頭で当たりを付けてから起こしているので、ここへ来るのは
        // AIの発言だけ——念のため型を見て、文字列の中にたまたま同じ並びが
        // 入っていた行は落とす。**落とす前に戻りを引く**:道具の戻りが会話
        // ログや `{"type":"assistant"…}` を含む字を持ち帰ると印に当たるので、
        // ここで素通りさせると返り済みの道具が実行中のまま残る。
        if value["type"].as_str() != Some("assistant") {
            drop_returned(&mut pending, line);
            continue;
        }
        let message = &value["message"];
        if let Some(model) = message["model"].as_str() {
            snap.model = Some(model_short(model));
        }
        if let Some(effort) = value["effort"].as_str() {
            snap.effort = Some(effort.to_string());
        }
        let usage = &message["usage"];
        let used: f64 = [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .iter()
        .filter_map(|key| usage[*key].as_f64())
        .sum();
        if used > 0.0 {
            snap.context = Some(used);
        }
        // **返し終えた印は `text`。** 走っている木の末尾は考えているだけの
        // 発言(`thinking`)か道具の呼び出しで、返し終えた木のログは必ず
        // `text` の行で終わる(実測2026-08-18・完走した lane-* 全件)。
        {
            let chunks = || message["content"].as_array().into_iter().flatten();
            let said = chunks().any(|chunk| chunk["type"] == "text");
            snap.mid_turn = !said;
            snap.mid_stream = !said && !chunks().any(|chunk| chunk["type"] == "tool_use");
        }
        for chunk in message["content"].as_array().into_iter().flatten() {
            if chunk["type"] != "tool_use" {
                continue;
            }
            let (Some(id), Some(tool)) = (chunk["id"].as_str(), chunk["name"].as_str()) else {
                continue;
            };
            pending.push((
                id.to_string(),
                Activity {
                    tool: tool.to_string(),
                    note: tool_note(tool, &chunk["input"]),
                },
            ));
        }
    }
    // 並列に投げた回は最後の1つを出す(行は1本しかない)。
    snap.activity = pending.pop().map(|(_, activity)| activity);
    snap
}

/// 道具を握ったままのサブエージェントを残す窓。長い `cargo build` の最中は
/// 誰も書き込まないので、手が止まって見えても落とさない幅を取る。
///
/// **もとは30分で、それが長すぎた。** 道具を握ったまま殺された木は戻りを
/// 書き残さないので、握った印だけが死んだあとも残る——実測2026-08-13、
/// 殺された木が「Bash実行中」のまま26分居座っていた。このリポジトリで最長の道具は release ビルドの
/// 90秒級なので、生きている木を巻き込まない幅として5分を取る。
pub const AGENT_BUSY_WINDOW: f64 = 300.0;

/// 返事を負ったまま考えているサブエージェントを残す窓([`Snapshot::mid_turn`])。
///
/// 道具を握っている間と違い、考えている時間には作りからくる上限が無い
/// ——長い記事を xhigh で書かせれば数分は平気で黙る。この機体に残っている
/// サブエージェントの会話ログ3467本・手番の途中の空白328,126件を測ると
/// (2026-08-18):
///
/// ```text
///    60-120s 2912件 ┃ 120-180s 960件 ┃ 180-300s 553件
///   300-420s 214件 ┃ 420-600s 113件 ┃ 600-900s 164件
///   900-1200s 8件 ┃ 1200-1800s 5件 ┃ 1800s- 7件
/// ```
///
/// 900秒までは滑らかに減り、そこで**桁が落ちる**。越えた先(20件)は中身を
/// 見ると数時間・十数時間の空白で、考えていたのではなく止められた木・放って
/// 置かれた木だった。だから境目はここ。
///
/// [`AGENT_BUSY_WINDOW`](5分)より広いぶん、考えたまま止められた木が最長で
/// 15分居座る。**握ったまま殺された木の窓は広げていない**——利用者が実際に見た
/// ゾンビは道具を握った形で、そちらは5分の
/// ままで落ちる。
pub const AGENT_THINK_WINDOW: f64 = 900.0;

/// 発言の途中で切れたサブエージェントを残す窓([`Snapshot::mid_stream`])。
///
/// 思考の断片だけを書いて閉じていない発言は、走っている木なら続きがすぐ来る
/// ——この機体のサブエージェントの会話ログで、思考だけの発言から次の書き込みまでの
/// 間隔360件を測ると 30秒以内が347件・最長180秒で、そこから先は1件も無い。
/// 対して[`AGENT_THINK_WINDOW`]が受け持つ「戻りを待ってから書き出すまで」は
/// 10分級まで伸びる。同じ窓で裁くと、思考の途中で殺された木が15分居座る。
pub const AGENT_STREAM_WINDOW: f64 = 240.0;

/// **稼働中だけ**を残す窓([`live_agents`])。
///
/// どこまで縮められるか。**走っている最中でも会話ログは連続には伸びない。**
/// 道具を握っている間は [`Snapshot::activity`] で拾えるが、戻りを受けてから
/// 次の発言を書き終えるまでの間はどちらにも映らない。実測(2026-08-13・その
/// とき走っていた worker 1体の末尾60件)で、書き込みの間隔は0〜13秒が大半・
/// 最大52秒だった。ここを短く取ると考えている最中に行が消えて書いた瞬間に
/// 戻る——**点滅は行が1本増えることよりずっと目に障る**。60秒はその最大を
/// 跨げる幅で、裏返せば「終わってから消えるまで」も同じだけ遅れる。
///
/// **その実測は狭かった。** 見ていたのがコードを触る worker ばかりで、道具を
/// 数秒おきに呼ぶぶん間隔が短く出ていた。文章を書く worker は道具の戻りを
/// 受けてから数分考え込む(実測2026-08-18・313秒)ので、時計だけでは足りない
/// ——考えている最中かどうかは [`Snapshot::mid_turn`] で見分け、そちらは
/// [`AGENT_BUSY_WINDOW`] まで残す。この60秒が効くのは**返し終えた木**だけに
/// なった。
pub const AGENT_LIVE_WINDOW: f64 = 60.0;

thread_local! {
    /// サブエージェント1体ぶんの読みの覚え書き。鍵は会話ログの在り処、値は
    /// **読んだときの最終更新**と、そのとき組んだ行。
    ///
    /// 走査は速い刻に置いた([`assemble_tabs`] の但し書き)ので、毎刻ぜんぶを
    /// 読み直すと窓が固まる。1体ぶんの末尾読みは20ms級・道具の戻りが大きい回
    /// で120ms級([`snapshot`] の但し書き)で、実測2026-08-13 のセッションでは
    /// 窓に10体が入っていた——素で回せば1刻200ms級になる。
    ///
    /// **最終更新が動いていなければ中身も動いていない。** だから読まずに同じ
    /// 値を返す——間引きではなく同値の置き換えで、行の中身は素で読んだときと
    /// 1文字も変わらない。走査するスレッドは1本(モニターの刻)なので、
    /// 錠を持たない thread_local で足りる。
    static AGENT_MEMO: std::cell::RefCell<BTreeMap<PathBuf, (f64, Agent)>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

/// セッション配下のサブエージェントのうち、**いま動いているものだけ**を拾う。
///
/// 「動いている」の定めはここ。次のどれかを満たすものだけが残る:
///
/// 1. **道具を握ったまま結果を待っている**([`Snapshot::activity`] が `Some`)。
///    長い `cargo build` の最中は誰も書き込まないので、[`AGENT_BUSY_WINDOW`]
///    (5分)まで残す。握ったまま殺された木はここで消える。
/// 2. **手番の途中**([`Snapshot::mid_turn`])。道具の戻りを受けてから次の
///    発言を書き終えるまでの、考えているだけの時間がこれに当たる。同じく
///    [`AGENT_BUSY_WINDOW`] まで。
/// 3. **会話ログの最終更新が [`AGENT_LIVE_WINDOW`](60秒)以内**。
///
/// どれでもないもの——返し終えて黙ったもの、声を掛けられるのを待っている
/// もの——は落ちる。
#[must_use]
pub fn live_agents(transcript: &std::path::Path, session_id: &str, now: f64) -> Vec<Agent> {
    scan_agents(transcript, session_id, now, AGENT_LIVE_WINDOW)
}

/// 走査の本体。`live_window` は「手が止まっているものを何秒まで残すか」。
///
/// メタ(`subagents/*.meta.json`)は**終わった分も残り続ける**ので、そのまま
/// 並べると起動以来の全員が出る。生きているかどうかは会話ログの最終更新と
/// 結果待ちの道具で測る。
fn scan_agents(
    transcript: &std::path::Path,
    session_id: &str,
    now: f64,
    live_window: f64,
) -> Vec<Agent> {
    let dir = match transcript.parent() {
        Some(parent) => parent.join(session_id).join("subagents"),
        None => return Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Agent> = Vec::new();
    for entry in entries.flatten() {
        let meta_path = entry.path();
        if meta_path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(stem) = meta_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = stem.strip_suffix(".meta.json") else {
            continue;
        };
        let log = dir.join(format!("{stem}.jsonl"));
        // ここまでが**安い篩**(read_dir と mtime だけ)。末尾読みへ進むのは
        // どちらかの窓に入ったものだけで、それも覚え書きに当たれば読まない。
        let Some(seen) = mtime(&log) else { continue };
        let idle = now - seen;
        // 篩は**一番広い窓**で掛ける。狭いほうで先に落とすと、考え込んでいる木
        // ([`AGENT_THINK_WINDOW`])が末尾を読まれる前に消える。
        if idle > AGENT_BUSY_WINDOW.max(AGENT_THINK_WINDOW).max(live_window) {
            continue; // どの窓にも入らない
        }
        let mut agent = memo_agent(&meta_path, &log, seen, stem);
        agent.idle_sec = idle;
        if !agent.live_within(live_window) {
            continue; // どの窓にも入らない=終わっている
        }
        out.push(agent);
    }
    prune_memo(now);
    // 名前順で固定する。動いた順に並べ替えると、行が上下に跳ねて読めなくなる。
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// 1体ぶんの行を組む。**最終更新が前と同じなら読み直さない**([`AGENT_MEMO`])。
fn memo_agent(meta_path: &std::path::Path, log: &std::path::Path, seen: f64, stem: &str) -> Agent {
    let hit = AGENT_MEMO.with(|memo| {
        memo.borrow()
            .get(log)
            .filter(|(at, _)| *at == seen)
            .map(|(_, agent)| agent.clone())
    });
    if let Some(agent) = hit {
        return agent;
    }
    let agent = build_agent(meta_path, log, stem);
    AGENT_MEMO.with(|memo| {
        memo.borrow_mut()
            .insert(log.to_path_buf(), (seen, agent.clone()));
    });
    agent
}

/// 覚え書きから、もうどの窓にも入らないものを落とす。走るたびに1体ぶん積むので、
/// 刈らないとセッションを跨いで溜まり続ける。境目は窓の広いほう。
fn prune_memo(now: f64) {
    let keep = AGENT_BUSY_WINDOW.max(AGENT_THINK_WINDOW);
    AGENT_MEMO.with(|memo| {
        memo.borrow_mut().retain(|_, (seen, _)| now - *seen <= keep);
    });
}

/// メタと会話ログの末尾から1体ぶんを組む。`idle_sec` は呼び手が入れる
/// ——ここは覚え書きに入る値で、刻ごとに変わるものは持たせない。
fn build_agent(meta_path: &std::path::Path, log: &std::path::Path, stem: &str) -> Agent {
    let snap = read_agent_snapshot(log);
    let meta: serde_json::Value = std::fs::read_to_string(meta_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(serde_json::Value::Null);
    let name = meta["name"]
        .as_str()
        .or_else(|| meta["agentType"].as_str())
        .unwrap_or(stem)
        .to_string();
    Agent {
        name,
        note: meta["description"].as_str().unwrap_or_default().to_string(),
        model: snap
            .model
            .or_else(|| meta["model"].as_str().map(model_short)),
        effort: snap.effort,
        context: snap.context,
        context_left: None, // spread_windows が入れる
        activity: snap.activity,
        mid_turn: snap.mid_turn,
        mid_stream: snap.mid_stream,
        idle_sec: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_truncated_title_is_restored_only_from_its_own_full_text() {
        // 切られた題は、切られる前の字が頭に残る全文でだけ置き換わる。
        assert!(completes_truncated_title("長い題の頭…", "長い題の頭と続きの全部"));
        assert!(!completes_truncated_title("長い題の頭…", "別の題"));
        assert!(!completes_truncated_title("切られていない題", "切られていない題と続き"));
        assert!(!completes_truncated_title("…", "何か"));
        // ログからは最後の custom-title 記録の全文を取る。
        let raw = concat!(
            r#"{"type":"custom-title","customTitle":"古い題","sessionId":"s"}"#,
            "\n",
            r#"{"type":"user","message":"custom-titleという文字を含む発言"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"新しい題","sessionId":"s"}"#,
            "\n",
        );
        assert_eq!(last_custom_title(raw.as_bytes()).as_deref(), Some("新しい題"));
    }

    #[test]
    fn title_marks_are_stripped_but_the_text_survives() {
        assert_eq!(strip_title_mark("⠋ 考え中"), "考え中");
        assert_eq!(strip_title_mark("✳ 返事が来た"), "返事が来た");
        assert_eq!(strip_title_mark("● 済み"), "済み");
        assert_eq!(strip_title_mark("  素の題  "), "素の題");
        // の現物
        // (実機のタブ名から採った字。`◐` は U+25D0 で、Claude が回している
        // 半月のひとコマ)。
        assert_eq!(
            strip_title_mark("◐ ライブラリの確認"),
            "ライブラリの確認"
        );
        // 日本語の題は削らない
        assert_eq!(strip_title_mark("盤のRust移行"), "盤のRust移行");
    }

    /// 会話ログを1本でっち上げて、末尾から取れるものを確かめる。
    fn write_log(name: &str, lines: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join("board-monitor-test-logs");
        std::fs::create_dir_all(&dir).expect("作れる");
        let path = dir.join(name);
        std::fs::write(&path, lines.join("\n")).expect("書ける");
        path
    }

    #[test]
    fn the_tail_gives_the_model_effort_and_the_tool_still_running() {
        let path = write_log(
            "running.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":false,"effort":"xhigh","message":{"model":"claude-opus-5","usage":{"input_tokens":1,"cache_creation_input_tokens":9,"cache_read_input_tokens":140000},"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a/b/panel.rs"}}]}}"#,
                r#"{"type":"user","isSidechain":false,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
                r#"{"type":"assistant","isSidechain":false,"effort":"xhigh","message":{"model":"claude-opus-5","usage":{"input_tokens":2,"cache_read_input_tokens":150000},"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"description":"テストを走らせる","command":"cd /x && cargo test"}}]}}"#,
            ],
        );
        let snap = read_snapshot(&path);
        assert_eq!(snap.model.as_deref(), Some("opus"), "モデルは短名で取る");
        assert_eq!(snap.effort.as_deref(), Some("xhigh"));
        assert_eq!(snap.context, Some(150_002.0), "最後のリクエストの積み");
        let act = snap.activity.expect("結果の返っていない道具が残る");
        assert_eq!(act.tool, "Bash");
        assert_eq!(act.note, "テストを走らせる", "説明があるならそれを出す");
    }

    #[test]
    fn a_finished_tool_is_not_reported_as_running() {
        let path = write_log(
            "done.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-fable-5","usage":{"input_tokens":10},"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"走らせる"}}]}}"#,
                r#"{"type":"user","isSidechain":false,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
            ],
        );
        assert_eq!(read_snapshot(&path).activity, None);
    }

    /// 応答を返し終えたパネルと、まだ手番の中に居るパネルを親のログから見分ける。
    ///
    /// 名簿(`claude agents`)の busy は背景に残ったシェル1本でも立つので、
    /// これが無いと応答済みのパネルが起動中に見える。
    #[test]
    fn a_parents_turn_is_read_from_its_own_lines_only() {
        let settled = write_log(
            "settled.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5","usage":{"input_tokens":10},"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"背景で走らせる"}}]}}"#,
                r#"{"type":"user","isSidechain":false,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
                r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5","usage":{"input_tokens":20},"content":[{"type":"text","text":"返し終えた"}]}}"#,
            ],
        );
        let snap = read_snapshot(&settled);
        assert!(!snap.mid_turn, "字で返し終えたパネルは手番の外");
        assert_eq!(snap.activity, None);

        let thinking = write_log(
            "mid-turn.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5","usage":{"input_tokens":20},"content":[{"type":"text","text":"前の返事"}]}}"#,
                r#"{"type":"user","isSidechain":false,"message":{"content":[{"type":"text","text":"次の頼み"}]}}"#,
            ],
        );
        assert!(
            read_snapshot(&thinking).mid_turn,
            "声を掛けられたパネルはまだ返していない"
        );

        // 配下の発言・その道具の戻りは親の手番を動かさない。ここが漏れると、
        // 木を1体でも走らせたパネルが永久に「返していない」ままになる。
        let with_child = write_log(
            "child-noise.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":false,"message":{"model":"claude-opus-5","usage":{"input_tokens":20},"content":[{"type":"text","text":"返し終えた"}]}}"#,
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"c1"}]}}"#,
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","usage":{"input_tokens":5},"content":[{"type":"tool_use","id":"c2","name":"Read","input":{"file_path":"/a.rs"}}]}}"#,
            ],
        );
        assert!(
            !read_snapshot(&with_child).mid_turn,
            "傍流の行で親の手番を立てている"
        );
    }

    #[test]
    fn a_subagent_log_is_all_sidechain_so_the_filter_has_to_be_off() {
        let line = r#"{"type":"assistant","isSidechain":true,"effort":"high","message":{"model":"claude-opus-5","usage":{"input_tokens":100},"content":[]}}"#;
        let path = write_log("side.jsonl", &[line]);
        assert_eq!(read_snapshot(&path).model, None, "親として読めば無視される");
        let snap = read_agent_snapshot(&path);
        assert_eq!(snap.model.as_deref(), Some("opus"), "エージェントとして読めば拾う");
        assert_eq!(snap.effort.as_deref(), Some("high"));
    }

    #[test]
    fn the_command_falls_back_to_its_main_clause() {
        // 説明の無い Bash。前置きの cd は捨てて主文だけ出す。
        let input = serde_json::json!({"command": "cd /very/long/path && cargo test -p armature-core"});
        assert_eq!(tool_note("Bash", &input), "cargo test -p armature-core");
        let edit = serde_json::json!({"file_path": "/a/b/layout.rs"});
        assert_eq!(tool_note("Edit", &edit), "layout.rs", "編集はファイル名だけ");
    }

    #[test]
    fn model_ids_fold_into_their_family() {
        assert_eq!(model_short("claude-fable-5"), "fable");
        assert_eq!(model_short("claude-opus-5[1m]"), "opus");
        assert_eq!(model_short("claude-haiku-4-5-20251001"), "haiku");
        assert_eq!(model_short("opus"), "opus");
        assert_eq!(model_short("gpt-5"), "gpt-5", "知らない名前はそのまま");
    }

    #[test]
    fn only_an_explicit_1m_model_setting_is_a_window_hint() {
        let wide = serde_json::json!({"model": "opus[1m]"});
        assert_eq!(
            configured_window_of(&wide),
            Some(("opus".into(), CONTEXT_WINDOW_1M))
        );
        assert_eq!(
            configured_window_of(&serde_json::json!({"model": "opus"})),
            None,
            "モデル系列だけでは1M窓の証拠にならない"
        );
        assert_eq!(
            configured_window_of(&serde_json::json!({"model": "sonnet[wide]"})),
            None,
            "知らない修飾子を推測してはいけない"
        );
    }

    #[test]
    fn a_sibling_over_200k_proves_the_family_has_the_wide_window() {
        // 兄弟の実績で窓を決める。同じ opus でも、200kを超えた者が居なければ
        // 200k窓のまま——推定を広げるのは実績のあるモデルだけ。
        let mut session = Session {
            model: Some("fable".into()),
            context: Some(100_000.0),
            agents: vec![
                Agent {
                    name: "a".into(),
                    model: Some("opus".into()),
                    context: Some(300_000.0),
                    ..Agent::default()
                },
                Agent {
                    name: "b".into(),
                    model: Some("opus".into()),
                    context: Some(150_000.0),
                    ..Agent::default()
                },
            ],
            ..Session::default()
        };
        spread_windows(&mut session);
        assert_eq!(session.context_left, Some(50.0), "親は200k窓のまま");
        assert_eq!(session.agents[0].context_left, Some(70.0));
        assert_eq!(
            session.agents[1].context_left,
            Some(85.0),
            "兄弟が1M窓だと示したので、200k未満でも1M窓で読む"
        );
    }

    // ---- 文脈の推定 ------------------------------------------------------
    //
    // 一覧が出す数字は `100 - context_left`。ここでは欄の値と、そこから出る
    // **画面の字**の両方を見る。

    /// 画面に出る使用量%(欄は残り%で持っている)。
    fn shown(session: &Session) -> f64 {
        100.0 - session.context_left.expect("文脈欄が空")
    }

    #[test]
    fn an_explicit_matching_model_setting_keeps_the_1m_window() {
        let mut session = Session {
            model: Some("opus".into()),
            effort: Some("high".into()),
            context: Some(168_274.0),
            ..Session::default()
        };
        let configured = BTreeMap::from([("opus".to_string(), CONTEXT_WINDOW_1M)]);
        apply_windows(&mut session, &configured);
        assert!((shown(&session) - 16.8274).abs() < 0.0001);
        assert_eq!(session.model.as_deref(), Some("opus"));
        assert_eq!(session.effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_configured_window_never_crosses_model_families() {
        let mut session = Session {
            model: Some("sonnet".into()),
            context: Some(100_000.0),
            ..Session::default()
        };
        let configured = BTreeMap::from([("opus".to_string(), CONTEXT_WINDOW_1M)]);
        apply_windows(&mut session, &configured);
        assert_eq!(shown(&session), 50.0, "sonnet は従来の200k推定へ倒す");
    }

    /// Fable は印を付けずに立てても 1M 窓で開く。印の無いパネルを 200k 窓と
    /// 誤り、187k 積んだ Fable を 97% と報せた回帰。
    #[test]
    fn fable_reads_as_a_1m_window_without_any_mark() {
        let windows: BTreeMap<String, f64> = FAMILY_WINDOWS
            .into_iter()
            .map(|(model, window)| (model.to_string(), window))
            .collect();
        let mut session = Session {
            model: Some("fable".into()),
            context: Some(187_000.0),
            ..Session::default()
        };
        apply_windows(&mut session, &windows);
        assert!((shown(&session) - 18.7).abs() < 0.0001);
    }

    #[test]
    fn the_mark_on_the_name_decides_the_colour() {
        // 色の源は名乗りのマーク。PTY に出力が届いたかで見ていた頃は、1文字
        // 描かれるだけで緑や黄へ瞬いて戻っていた。
        assert_eq!(title_state("⠋ 何かしている"), Some(TabState::Working), "点字が回っている");
        assert_eq!(title_state("◐ 何かしている"), Some(TabState::Working), "半月が回っている");
        // `✳` は未読にしない——見ても消えない印なので、写すと黄が居座る。
        assert_eq!(title_state("✳ 返事が出揃った"), None);
        assert_eq!(title_state("素の題"), None, "マークの無い名乗りは状態を持たない");
        assert_eq!(title_state("  ⠙ 前に空白があっても"), Some(TabState::Working));
        assert_eq!(title_state(""), None);
    }

    // ---- 一覧に載る行の出どころ ------------------------------------------
    //
    // **行になるかどうかを決めるのは窓のタブだけ**。名簿(`claude agents --json`)
    // と `ps` は、そのタブに居る claude の中身を足すためにある。

    /// 名簿の1件。会話ログを読ませないよう `session_id` は空にする。
    fn running(pid: &str, name: &str, busy: bool) -> Running {
        Running {
            pid: pid.into(),
            session_id: String::new(),
            name: name.into(),
            busy,
        }
    }

    /// `ps` の写し。渡した pid は全部生きていて、tty を持つ。
    fn ps(pairs: &[(&str, &str)]) -> Ps {
        Ps {
            ttys: pairs
                .iter()
                .map(|(pid, tty)| ((*pid).to_string(), (*tty).to_string()))
                .collect(),
            alive: pairs.iter().map(|(pid, _)| (*pid).to_string()).collect(),
            claude: pairs.iter().map(|(pid, _)| (*pid).to_string()).collect(),
        }
    }

    #[test]
    fn ps_tells_the_live_pids_their_ttys_and_which_ones_are_claude() {
        // 名簿を引き直す合図に使う列。`claude agents --json` は8秒級で
        // 15秒ごとには叩けないので、増えた回だけ引く。
        let raw = "  100 ttys008  claude\n\
                   \x20 101 ttys008  node\n\
                   \x20 102 ??       claude\n\
                   \x20 103 ??       /Applications/Claude.app/Contents/MacOS/Claude Helper\n";
        let got = parse_ps(raw);
        assert_eq!(got.alive.len(), 4, "生きている pid は tty の有無に関わらず数える");
        assert_eq!(got.ttys.get("100").map(String::as_str), Some("ttys008"));
        assert!(!got.ttys.contains_key("102"), "tty を持たない pid は対応表に入れない");
        assert_eq!(
            got.claude,
            ["100".to_string(), "102".to_string()].into_iter().collect(),
            "claude そのものだけを数える(Claude.app の補助プロセスは別物)"
        );
    }

    #[test]
    fn a_hung_command_gives_up_instead_of_freezing_the_whole_monitor() {
        // 外のコマンドが帰ってこない回。ここで待ち続けると刻が進まなくなり、
        // 一覧も利用枠も最後の姿で貼り付く——利用者が見た「増えも減りもしない」
        // 一覧の形。時間切れそのものは20秒なので、ここでは**読み切りが
        // 噛み合わないこと**(大きな出力でも詰まらない)を見る。
        let big = run("/bin/sh", &["-c", "seq 1 200000"]).expect("大きな出力を読み切れる");
        assert_eq!(big.lines().count(), 200_000, "パイプの緩衝で噛み合っている");
        // 終了コードが立っている回は取得できなかった扱い(tmux のサーバが
        // 居ないときがこれ)。
        assert!(run("/bin/sh", &["-c", "exit 1"]).is_none());
    }

    /// タブ1枚(名乗っているもの)。
    fn tab(tty: &str, n: u32, title: &str) -> LocalTab {
        LocalTab {
            tty: tty.into(),
            target: format!("armature:{n}"),
            title: title.into(),
            named: true,
            ..LocalTab::default()
        }
    }

    /// 名乗っていないタブ(起こしたコマンド名しか無い)。
    fn bare_tab(tty: &str, n: u32, fallback: &str) -> LocalTab {
        LocalTab {
            named: false,
            ..tab(tty, n, fallback)
        }
    }

    #[test]
    fn the_list_is_exactly_our_tabs_and_nothing_else() {
        // 。
        // 外で動いているセッション(別の端末アプリ等)はタブを持たないので載らない。
        let mut mine = tab("ttys010", 1, "うちの1枚目");
        // 動いているかは**窓の申告**で決まる。名簿(`claude agents`)の busy は
        // 最長60秒古いので、`or` で足すと止めたのに緑のままになる。
        mine.busy = true;
        let tabs = [mine];
        let roster = [
            running("100", "外の端末のやつ", false), // ttys008・タブ無し
            running("200", "うちの中の claude", true),
        ];
        let ps = ps(&[("100", "ttys008"), ("200", "ttys010")]);
        let got = assemble_tabs(&tabs, &ps, &roster, 1000.0);
        assert_eq!(got.len(), 1, "タブの数と行の数が合っていない");
        assert_eq!(got[0].title, "うちの1枚目");
        assert_eq!(got[0].target, "armature:1");
        assert!(got[0].busy, "窓の申告を拾えていない");
        assert_eq!(got[0].tab_state, TabState::Working);
        // 名簿が busy と言っても、窓が静かと言えば静か。
        let mut quiet = tab("ttys010", 1, "うちの1枚目");
        quiet.busy = false;
        let got = assemble_tabs(&[quiet], &ps, &roster, 1000.0);
        assert!(!got[0].busy, "名簿の古い busy を拾っている");
    }

    /// サブエージェント1体ぶんの作り物(メタと会話ログ)を置く。
    fn put_agent(dir: &std::path::Path, name: &str, log: &str, age: f64, now: f64) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(format!("{name}.meta.json")),
            format!(r#"{{"name":"{name}","description":"何かの仕事","model":"opus"}}"#),
        )
        .unwrap();
        let path = dir.join(format!("{name}.jsonl"));
        std::fs::write(&path, log).unwrap();
        // 最終更新を狙った古さに寝かせる(now は試験が決めた刻)。
        let at = UNIX_EPOCH + Duration::from_secs_f64(now - age);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    #[test]
    fn only_the_agents_that_are_actually_working_hang_under_the_row() {
        // 。
        // 「動いている」は〈道具を握ったまま〉か〈ごく最近書いた〉のどちらか。
        let now = 1_800_000_000.0;
        let root = std::env::temp_dir().join(format!("agents-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sid = "s-1";
        let transcript = root.join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&transcript, "").unwrap();
        let subs = root.join(sid).join("subagents");

        // 1) いま字を書いている(道具は握っていないが更新がごく近い)。
        put_agent(&subs, "書いてる", "", 5.0, now);
        // 2) 道具を握ったまま結果を待っている(3分書き込みが無くても稼働中)。
        let held = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"cargo test を走らせる"}}]}}"#;
        put_agent(&subs, "待ってる", held, 200.0, now);
        // 3) 手が止まっていて、書き込みも稼働の窓の外(=終わっている)。
        put_agent(&subs, "終わってる", "", AGENT_LIVE_WINDOW + 30.0, now);

        let got = live_agents(&transcript, sid, now);
        let names: Vec<&str> = got.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["待ってる", "書いてる"], "稼働中の選り分けが違う");
        assert_eq!(
            got[0].activity.as_ref().map(|a| a.tool.as_str()),
            Some("Bash"),
            "握っている道具を拾えていない"
        );

        // 覚え書きに当たっても中身は変わらない(同じ答えが返る)。
        assert_eq!(live_agents(&transcript, sid, now), got, "覚え書きで値が化けた");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tool_that_already_came_back_is_not_running() {
        // ゾンビの芯は「握った印だけが残る」こと。戻りが来ている道具は、
        // その場で結果待ちの列から外す(木のログは全行が傍流なので、
        // 読むのは read_agent_snapshot のほう)。
        let done = write_log(
            "agent-done.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","usage":{"input_tokens":10},"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"ビューアの受け口を見る"}}]}}"#,
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
            ],
        );
        assert_eq!(
            read_agent_snapshot(&done).activity,
            None,
            "戻りが来ているのに実行中と読んでいる"
        );

        // 戻りの中身が会話ログを持ち帰ると、行そのものに `{"type":"assistant"…}`
        // が入る。**印だけの当たり付けはここで外れる**ので、型を見て落とす前に
        // 戻りを引いておかないと、返り済みの道具が実行中のまま残る。
        let nested = write_log(
            "agent-nested-result.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","usage":{"input_tokens":10},"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"ログを覗く"}}]}}"#,
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]},"toolUseResult":{"lines":[{"type":"assistant","message":{"model":"claude-opus-5"}}]}}"#,
            ],
        );
        assert_eq!(
            read_agent_snapshot(&nested).activity,
            None,
            "戻りが会話ログを持ち帰ると引き算を素通りしている"
        );

        // 並べて投げた2つのうち片方だけが返った回は、返っていないほうが残る
        // ——実測2026-08-13の死体(abrowser)がこの形で、こちらは印からは
        // 死んだと分からない。落とすのは窓([`AGENT_BUSY_WINDOW`])の仕事。
        let half = write_log(
            "agent-half.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","usage":{"input_tokens":10},"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"片方"}},{"type":"tool_use","id":"t2","name":"Bash","input":{"description":"もう片方"}}]}}"#,
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t2"}]}}"#,
            ],
        );
        let act = read_agent_snapshot(&half).activity.expect("片方は結果待ち");
        assert_eq!(act.note, "片方", "返ったほうを実行中と読んでいる");
    }

    #[test]
    fn a_tree_killed_holding_a_tool_stops_being_shown() {
        // 。道具を握った
        // まま殺された木は戻りを書き残さないので、印だけでは生死が付かない
        // ——実測では26分「Bash実行中」のまま居座っていた。窓を5分に縮めて落とす。
        let now = 1_800_000_000.0;
        let root = std::env::temp_dir().join(format!("agents-zombie-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sid = "s-1";
        let transcript = root.join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&transcript, "").unwrap();
        let subs = root.join(sid).join("subagents");

        let held = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"release ビルドを回す"}}]}}"#;
        // **秒はわざと直に書く**([`AGENT_BUSY_WINDOW`] で書くと窓を何秒に
        // しても通ってしまい、30分へ戻した退行を捕まえられない)。
        // 生きている木。このリポジトリで最長の道具は release ビルドの90秒級で、
        // 3分20秒黙っていても落としてはいけない。
        put_agent(&subs, "握って生きてる", held, 200.0, now);
        // 握ったまま殺された木。実測の死体は26分居座っていた——10分で落ちること。
        put_agent(&subs, "握って死んでる", held, 600.0, now);
        // 返し終えて黙ったままの木(こちらは稼働の窓で落ちる)。**末尾は
        // `text` の行**——返した木のログは必ずこの形で終わる。ここを道具の
        // 戻りで終わらせると「考え込んでいる木」と同じ形になり、落ちるのが
        // 正しいのかどうかが試験から読めなくなる。
        let returned = format!(
            "{held}\n{}\n{}",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"直しました"}]}}"#
        );
        put_agent(&subs, "返して黙ってる", &returned, AGENT_LIVE_WINDOW + 30.0, now);

        let names: Vec<String> = live_agents(&transcript, sid, now)
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["握って生きてる"], "ゾンビが一覧に残っている");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tree_killed_mid_sentence_goes_before_one_that_is_still_thinking() {
        // 思考の断片だけ書いて止められた木は、道具も握らず字も返していないので
        // 「戻りを受けて考えている木」と同じ形に見える。窓を分けないと15分居座る。
        let now = 1_800_000_000.0;
        let root = std::env::temp_dir().join(format!("agents-stream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sid = "s-2";
        let transcript = root.join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&transcript, "").unwrap();
        let subs = root.join(sid).join("subagents");

        let thinking = r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"thinking","thinking":"どう組むか"}]}}"#;
        let pending = format!("{}\n{}", r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/a/b.rs"}}]}}"#, r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#);
        // **秒は直に書く**(窓の定数で書くと、窓を何秒にしても通ってしまう)。
        // 書き出したあとの続きは実測で最長180秒。120秒はまだ生きている。
        put_agent(&subs, "書きかけで生きてる", thinking, 120.0, now);
        // 同じ形で500秒——実測の外。書き出しの途中で止められた木。
        put_agent(&subs, "書きかけで死んでる", thinking, 500.0, now);
        // 戻りを待ってから書き出すまでの空白は10分級まで伸びるので、残す。
        put_agent(&subs, "戻りを受けて考えてる", &pending, 500.0, now);

        let mut names: Vec<String> = live_agents(&transcript, sid, now)
            .into_iter()
            .map(|agent| agent.name)
            .collect();
        names.sort();
        assert_eq!(names, ["戻りを受けて考えてる", "書きかけで生きてる"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_slash_command_declares_the_model_and_effort_before_the_next_message() {
        // 打った直後は AI がまだ喋っていないので、発言の行からは前の値しか
        // 取れない。控えを読まないと桝が1手遅れる。
        let switched = write_log(
            "session-slash-switch.jsonl",
            &[
                r#"{"type":"assistant","message":{"model":"claude-opus-5","content":[{"type":"text","text":"やった"}]},"effort":"high"}"#,
                r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to [1mFable 5[22m and saved as your default for new sessions</local-command-stdout>"}}"#,
                r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set effort level to xhigh (saved as your default for new sessions): Deeper reasoning</local-command-stdout>"}}"#,
            ],
        );
        let snap = read_snapshot(&switched);
        assert_eq!(snap.model.as_deref(), Some("fable"), "打ち替えを読めていない");
        assert_eq!(
            snap.effort.as_deref(),
            Some("xhigh"),
            "effortの打ち替えを読めていない"
        );

        // 控えのあとに発言があれば、そちらが今の値。
        let spoke = write_log(
            "session-slash-then-speech.jsonl",
            &[r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>Set model to [1mFable 5[22m and saved as your default for new sessions</local-command-stdout>"}}"#, r#"{"type":"assistant","message":{"model":"claude-opus-5","content":[{"type":"text","text":"戻した"}]}}"#],
        );
        assert_eq!(
            read_snapshot(&spoke).model.as_deref(),
            Some("opus"),
            "控えが発言を追い越している"
        );

        // 文面が変わって族に当たらなくなったら、知らない字を桝へ流さない。
        assert_eq!(
            declared_model(r"<local-command-stdout>Set model to \u001b[1m新型\u001b[22m and saved"),
            None
        );
        assert_eq!(
            declared_effort("<local-command-stdout>Set effort level to sideways (saved"),
            None
        );
    }

    /// 手番の途中(返事を負っている)ことがログの形から読めるか。
    #[test]
    fn a_tree_that_owes_a_reply_is_still_thinking() {
        // 道具の戻りで終わっている=まだ返していない。
        let waiting = write_log(
            "agent-mid-turn-result.jsonl",
            &[
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"素材を集める"}}]}}"#,
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
            ],
        );
        let snap = read_agent_snapshot(&waiting);
        assert_eq!(snap.activity, None, "戻りが来た道具を握ったままと読んでいる");
        assert!(snap.mid_turn, "戻りを受けた木を返し終えたと読んでいる");

        // 考えているだけの発言(`thinking`)で終わる回も、まだ返していない。
        let musing = write_log(
            "agent-mid-turn-thinking.jsonl",
            &[
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","content":[{"type":"thinking","thinking":"どう書くか"}]}}"#,
            ],
        );
        assert!(read_agent_snapshot(&musing).mid_turn, "考えている最中を返し終えたと読んでいる");

        // `text` まで書けば返し終えた——完走した木のログはこの形で終わる。
        let done = write_log(
            "agent-replied.jsonl",
            &[
                r#"{"type":"user","isSidechain":true,"message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#,
                r#"{"type":"assistant","isSidechain":true,"message":{"model":"claude-opus-5","content":[{"type":"text","text":"直しました"}]}}"#,
            ],
        );
        assert!(!read_agent_snapshot(&done).mid_turn, "返し終えた木を手番の途中と読んでいる");
    }

    #[test]
    fn a_tree_thinking_between_tools_does_not_fall_off_the_list() {
        // 実測2026-08-18: 記事を書く worker(`writing-first-post`)が道具の
        // 戻り(13:57:06.190Z)を受けてから次の発言(14:02:18.969Z・`thinking`)
        // まで**313秒**黙り、その間ずっと枝が出ていなかった。同じ刻に走って
        // いたコードを触る worker(lane-*)は道具を数秒おきに呼ぶので60秒の窓に
        // 収まり、こちらだけが消えた——「出るサブエージェントと出ないもの」の
        // 差の正体はこれ。
        //
        // 時計だけでは「考え込んでいる」と「返して黙った」が同じ姿になるので、
        // 見分けるのはログの形(手番)。
        let now = 1_800_000_000.0;
        let root = std::env::temp_dir().join(format!("agents-thinking-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sid = "s-1";
        let transcript = root.join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&transcript, "").unwrap();
        let subs = root.join(sid).join("subagents");

        let held = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"description":"素材を集める"}}]}}"#;
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#;
        // 出るケース。戻りを受けて考え込んでいる木は、313秒黙っていても残る。
        let thinking = format!("{held}\n{result}");
        put_agent(&subs, "あ考え込んでる", &thinking, 313.0, now);
        // 出ないケース。`text` まで書いて返した木は、60秒の窓で落ちる。
        let replied = format!(
            "{thinking}\n{}",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"直しました"}]}}"#
        );
        put_agent(&subs, "い返した", &replied, AGENT_LIVE_WINDOW + 30.0, now);
        // 出ないケース。考えたまま止められた木も、握ったまま殺された木と同じく
        // 窓で落とす([`AGENT_THINK_WINDOW`])——考えている印だけが残り続ける
        // ので、ここに蓋が無いとゾンビが戻る。
        put_agent(&subs, "う考えたまま死んでる", &thinking, 1200.0, now);

        let names: Vec<String> = live_agents(&transcript, sid, now)
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["あ考え込んでる"], "手番の見分けが効いていない");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 会話1つぶんの作り物(会話ログと、その配下のサブエージェント1体)。
    fn put_log(root: &std::path::Path, sid: &str, agent: &str, now: f64) -> Log {
        let transcript = root.join(format!("{sid}.jsonl"));
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(&transcript, "").unwrap();
        put_agent(&root.join(sid).join("subagents"), agent, "", 5.0, now);
        Log {
            id: sid.into(),
            path: transcript,
        }
    }

    #[test]
    fn one_conversation_hangs_its_agents_under_a_single_row() {
        // 実測2026-08-13: `claude agents --json` が同じ会話(e12e03c1)を2つの
        // pid で返していた——古いほう(45142・idle)が生きたまま、別のタブが
        // `/resume` で同じ会話へ入り直した(77987・busy)形。木は会話に紐付く
        // ので、そのまま組むと**同じ体が2つの行にぶら下がる**(幅90の下見で
        // 実際に出ていた)。
        let now = 1_800_000_000.0;
        let root = std::env::temp_dir().join(format!("agents-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let one = put_log(&root, "s-同じ会話", "働き手", now);
        let row = |title: &str, busy: bool| Session {
            title: title.into(),
            busy,
            ..Session::default()
        };
        let log = || {
            Some(Log {
                id: one.id.clone(),
                path: one.path.clone(),
            })
        };

        // 止まっている行と動いている行が同じ会話を指す。木は動いているほうへ。
        let mut rows = vec![
            (log(), row("覗いている2枚目", false)),
            (log(), row("動かしている本体", true)),
        ];
        attach_agents(&mut rows, now, AGENT_LIVE_WINDOW);
        assert!(rows[0].1.agents.is_empty(), "同じ木が2つの行にぶら下がった");
        assert_eq!(rows[1].1.agents.len(), 1, "動いている行に木が無い");

        // どちらも止まっていれば上の行が持ち主(並びは動かさない)。
        let mut quiet = vec![(log(), row("上", false)), (log(), row("下", false))];
        attach_agents(&mut quiet, now, AGENT_LIVE_WINDOW);
        assert_eq!(quiet[0].1.agents.len(), 1, "上の行に木が無い");
        assert!(quiet[1].1.agents.is_empty(), "下の行にも木が出た");

        // 別々の会話なら両方に出る——重複潰しが効きすぎていないこと。
        let other = put_log(&root, "s-別の会話", "余所の働き手", now);
        let mut apart = vec![(log(), row("こちら", false)), (Some(other), row("あちら", true))];
        attach_agents(&mut apart, now, AGENT_LIVE_WINDOW);
        assert_eq!(apart[0].1.agents.len(), 1, "別の会話まで潰した");
        assert_eq!(apart[1].1.agents.len(), 1, "別の会話まで潰した");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_row_shows_the_sessions_own_title_before_the_command_name() {
        // 。名乗っている
        // タブはその字、名乗っていないタブは名簿の持つ会話の題を借りる。
        let ps = ps(&[("100", "ttys010")]);

        // 名乗っているタブ(盤から起こした・--name を渡した)はその字が勝つ。
        let named = assemble_tabs(
            &[tab("ttys010", 1, "901 設定画面の見直し")],
            &ps,
            &[running("100", "work-6a", false)],
            1000.0,
        );
        assert_eq!(named[0].title, "901 設定画面の見直し");

        // 名乗っていないタブは名簿の題を借りる(コマンド名 `sh` は出さない)。
        let borrowed = assemble_tabs(
            &[bare_tab("ttys010", 1, "sh")],
            &ps,
            &[running("100", "ライブラリの確認", false)],
            1000.0,
        );
        assert_eq!(borrowed[0].title, "ライブラリの確認");

        // 名簿にも居ないタブ(素のシェル)はコマンド名のまま。
        let fallback = assemble_tabs(&[bare_tab("ttys010", 1, "sh")], &ps, &[], 1000.0);
        assert_eq!(fallback[0].title, "sh");
    }

    #[test]
    fn a_tab_title_drops_the_status_mark_claude_writes_in_front_of_it() {
        // 。Claude はタブ名の頭へ状態マークを書く
        // (`◐ …` と `✳ …`)。状態は一覧が
        // 左端の印で出しているので、題の前のこれは意味を持たない飾り。
        let ps = ps(&[("100", "ttys010"), ("200", "ttys012")]);
        let got = assemble_tabs(
            &[
                tab("ttys010", 1, "◐ ライブラリの確認"),
                tab("ttys012", 2, "✳ 試験の赤を直す"),
            ],
            &ps,
            &[],
            1000.0,
        );
        assert_eq!(got[0].title, "ライブラリの確認");
        assert_eq!(
            got[1].title,
            "試験の赤を直す"
        );

        // マークしか無いタブ名は「名乗っていない」のと同じ——名簿の題を借りる。
        let only = assemble_tabs(
            &[tab("ttys010", 1, "◐ ")],
            &ps,
            &[running("100", "名簿の題", false)],
            1000.0,
        );
        assert_eq!(only[0].title, "名簿の題");
    }

    #[test]
    fn a_new_tab_becomes_a_row_even_before_claude_registers() {
        // 検収条件。cmd+T の直後は `claude agents --json` にまだ載らないし、
        // 素のシェルのタブ(shift+cmd+T)は永久に載らない。**それでも行は出る**
        // ——ここが名簿頼みだったのが「タブを開いても何も増えない」の実体。
        let ps = ps(&[("100", "ttys010")]);
        let before = assemble_tabs(&[tab("ttys010", 1, "1枚目")], &ps, &[], 1000.0);
        assert_eq!(before.len(), 1, "名簿が空だと行が消えている");

        let after = assemble_tabs(
            &[tab("ttys010", 1, "1枚目"), tab("ttys012", 2, "いま開いた")],
            &ps,
            &[],
            1000.0,
        );
        assert_eq!(after.len(), 2, "開いたタブが行になっていない");
        assert_eq!(after[1].title, "いま開いた");
        assert_eq!(after[1].target, "armature:2");
    }

    #[test]
    fn tabs_keep_the_order_the_window_hands_them_over_in() {
        // 並びはタブ順そのもの(ctrl+tab の航法面と一致させる)。
        let tabs = [
            tab("ttys010", 1, "い"),
            tab("ttys011", 2, "ろ"),
            tab("ttys012", 3, "は"),
        ];
        let got = assemble_tabs(&tabs, &Ps::default(), &[], 1000.0);
        let names: Vec<&str> = got.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(names, ["い", "ろ", "は"]);
    }
}
