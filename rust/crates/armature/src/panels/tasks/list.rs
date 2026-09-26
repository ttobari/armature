
use armature_core::tr;
use iced::widget::{Space, button, column, container, row, scrollable, stack, text};
use iced::{Alignment, Background, Border, Element, Length, Shadow};

use crate::ellipsized;
use crate::font;
use crate::glow;
use super::task_tree;
use crate::palette::{
    self, accent_alert, accent_attention, sunk, surface_active, surface_raised,
    text_faint, text_muted, text_primary, text_secondary,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Message {
    Select(usize),
    SetModel(Model),
    SetEffort(Effort),
    /// 一覧を送った位置(画素)。パネルを離れて戻ったとき同じ所を出すために覚える。
    Scrolled(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Sonnet,
    Opus,
    Fable,
}

impl Model {
    const ALL: [Self; 3] = [Self::Sonnet, Self::Opus, Self::Fable];

    const fn value(self) -> &'static str {
        match self {
            Self::Sonnet => "sonnet",
            Self::Opus => "opus",
            Self::Fable => "fable",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Sonnet => "Sonnet",
            Self::Opus => "Opus",
            Self::Fable => "Fable",
        }
    }

    fn next(current: &str) -> Self {
        Self::ALL
            .iter()
            .position(|model| model.value() == current)
            .map_or(Self::ALL[0], |index| {
                Self::ALL[(index + 1) % Self::ALL.len()]
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::Xhigh, Self::Max];

    const fn value(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    fn next(current: &str) -> Self {
        Self::ALL
            .iter()
            .position(|effort| effort.value() == current)
            .map_or(Self::ALL[0], |index| {
                Self::ALL[(index + 1) % Self::ALL.len()]
            })
    }
}

#[derive(Clone, Debug)]
struct Item {
    id: String,
    title: String,
    depth: usize,
    bundle: bool,
    /// 解けていない `待ち` を持つ(= 今は着手できない)。バンドル見出しでは
    /// [`wire_blocked_bundles`] が「配下に着手可が1つも無い」も同じ値へ畳む。
    blocked: bool,
    /// 列ごとに「この行を貫いて下へ続くか」(長さ = `depth`)。末尾は自分の枝の列で、
    /// `false` なら末子。[`crate::task_tree::Tree`] がそのまま受け取る。
    stems: Vec<bool>,
    /// 配下を持つ見出し。印の下から行の下端へ幹を下ろす。
    trunk: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSpec {
    pub id: String,
    pub title: String,
    pub model: String,
    pub effort: String,
}

#[allow(dead_code)]
impl LaunchSpec {
    /// Claude Code に渡す元コンセプトの直接起動プロンプト。
    #[must_use]
    pub fn prompt(&self) -> String {
        format!("/start {}", self.id)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsSpec {
    pub field: &'static str,
    pub value: &'static str,
    previous: String,
    ui: serde_json::Value,
}

/// `dd` が倒す相手。題も持つのは、倒したあとの知らせに出すため
/// (一覧はその場で消えるので、ID だけだと何を倒したのか読めない)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteSpec {
    pub id: String,
    pub title: String,
    /// バンドルなら、その下に連なる実在の行の全部。親だけ倒すと盤から消えた
    /// 見出しの下に開いたままの子が残り、どこからも辿れなくなる。
    pub children: Vec<String>,
}

/// `x` で掴んだ行の落とし先。`Return` を打つまで盤には触らない。
///
/// 親替えと並べ替えは別の書き込みなので、片方だけの回もある
/// (同じ親の中で位置だけ変える回は `reparent` が false)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveSpec {
    pub id: String,
    pub title: String,
    /// 移す先の親。空ならルート。
    pub parent: String,
    /// 親が変わるか。変わらないなら並べ替えだけで済む。
    pub reparent: bool,
    /// 移したあとの、その親の下の子順(ルートならルートどうしの順)。
    pub order: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SaveSettings(SettingsSpec),
}

#[derive(Clone, Debug, Default)]
pub struct Tasks {
    board: Option<armature_core::board::Board>,
    /// 一覧の行。**深さ順の1本の並び**——入れ子の器は持たない。階層は行ごとの
    /// [`Item::stems`] が引く枝で出す(`task_tree` の頭注)。
    items: Vec<Item>,
    selected: Option<usize>,
    pending_action: Option<Action>,
    status: Option<Result<String, String>>,
    /// 知らせを出した刻。**出しっぱなしにしない**——「NNN を起動中」が翌日まで
    /// パネルの下に残っていた。[`STATUS_LIFETIME`] で消える。
    status_at: Option<std::time::Instant>,
    error: Option<String>,
    /// 一覧をどこまで送っていたか(画素)。パネルは開き直すたび作り直されるので、
    /// 器の側で覚えていないと毎回先頭へ戻る。
    scroll: f32,
    /// `x` で掴んでいる行のID。**添字ではなくIDで持つ**——一覧は取り込みの
    /// たび組み直され、添字は別の行を指すようになる。
    moving: Option<String>,
}

/// 印と ID・ID と題の間。**行の `spacing` ではなく桝で置く**——一律の隙間は
/// 印の桝を枝の桁からずらす(`row_view` の註)。
const MARK_GAP: f32 = 5.0;

/// 題・ID・印の字。**パネルの丈に何行収まるかを決める**——利用者の要は「全体が把握できる」
/// ことなので、読める下限まで落とす(2026-09-14)。
const TITLE_SIZE: f32 = 10.0;
/// 選んでいる行の題の滲み。時計の字の6分の1ほどの大きさなので、滲みも浅く。
const TITLE_GLOW: f32 = 0.6;
const ID_SIZE: f32 = 9.0;
const MARK_SIZE: f32 = 9.0;

/// バンドルの印。**束そのものを描いた字**(Nerd Font の fa-folder。実体は
/// `font::USER_FALLBACK` の JetBrains Mono Nerd Font)。
///
/// 2026-09-14 まではドラフトパネルと同じ畳み印 `▾` を借りていたが、(1) このパネルに畳む
/// 操作は無いので「押せば畳める」と嘘を言い、(2) 9px の `text_faint` は配下の
/// 箇条点 `·` とほとんど見分けが付かず、**束であることを何も言っていなかった**。
const BUNDLE_MARK: &str = "\u{f07b}";
/// バンドルの印の大きさ。箇条点より小さく置く——字面が込んでいるぶん、同じ
/// 級数だと絵として重く見える。
const BUNDLE_MARK_SIZE: f32 = 8.0;

/// 待ちの行の印。**着手できない**ことだけを言う字で、箇条の点 `·` と見分けが付く
/// 形を選んだ(Moralerspace が持つ U+2298)。状態の印はこれと `·` の2値だけ。
/// 待ちは印に加えて**字と題の明るさ**でも言う。
const BLOCKED_MARK: &str = "⊘";

/// 知らせがパネルに残る長さ。1秒ごとの Tick で消すので、実際には最大1秒ぶん長く出る。
const STATUS_LIFETIME: std::time::Duration = std::time::Duration::from_secs(3);

impl Tasks {
    pub fn set_board(&mut self, board: armature_core::board::Board) {
        let selected_id = self.selected_item().map(|item| item.id.clone());
        let mut items = Vec::new();
        for node in &armature_core::board::build_roots(&board) {
            push_tree(&board, node, 0, &mut items);
        }
        wire_stems(&mut items);
        wire_blocked_bundles(&mut items);
        self.selected = selected_id
            .and_then(|id| items.iter().position(|item| item.id == id))
            .or_else(|| (!items.is_empty()).then_some(0));
        self.board = Some(board);
        self.items = items;
        self.pending_action = None;
        self.error = None;
        // 掴んだ行が盤から消えた回は手を離す。残すと行き先の計算が空を掴む。
        if self
            .moving
            .as_ref()
            .is_some_and(|id| !self.items.iter().any(|item| &item.id == id))
        {
            self.moving = None;
        }
    }

    pub fn failed(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = Some(index);
        }
    }

    /// Iced の入力メッセージをモデルへ適用する。
    pub fn update(&mut self, message: Message) -> bool {
        match message {
            Message::Select(index) => {
                let before = self.selected;
                self.select(index);
                self.selected != before
            }
            Message::SetModel(model) => self.set_ui_field("model", model.value()),
            Message::SetEffort(effort) => self.set_ui_field("effort", effort.value()),
            Message::Scrolled(offset) => {
                self.scroll = offset;
                false
            }
        }
    }

    /// 前に離れたときの送り位置。パネルを開くとき器がここへ戻す。
    #[must_use]
    pub fn scroll_offset(&self) -> f32 {
        self.scroll
    }

    /// クリックで変わった起動条件を親へ一度だけ渡す。
    ///
    /// 表示は `update` で即時に切り替わる。親はこの action を blocking task へ
    /// 退避し、[`persist_settings`] の結果を [`settings_saved`](Self::settings_saved)
    /// へ戻せばよい。
    #[allow(dead_code)]
    pub fn take_action(&mut self) -> Option<Action> {
        self.pending_action.take()
    }

    fn set_ui_field(&mut self, field: &'static str, value: &'static str) -> bool {
        let Some(board) = self.board.as_mut() else {
            return false;
        };
        let previous = board.ui_field(field).to_string();
        if previous == value {
            return false;
        }
        let ui = board.ui_with(field, value);
        board.ui = ui.clone();
        self.pending_action = Some(Action::SaveSettings(SettingsSpec {
            field,
            value,
            previous,
            ui,
        }));
        self.set_status(None);
        true
    }

    pub fn cycle_model(&mut self) -> bool {
        let Some(current) = self.board.as_ref().map(|board| board.model().to_string()) else {
            return false;
        };
        self.set_ui_field("model", Model::next(&current).value())
    }

    pub fn cycle_effort(&mut self) -> bool {
        let Some(current) = self.board.as_ref().map(|board| board.effort().to_string()) else {
            return false;
        };
        self.set_ui_field("effort", Effort::next(&current).value())
    }

    /// `dd` が対象にする、選択中の実在タスク。
    ///
    /// バンドルの空行(placeholder)は倒さない——実体が無く、倒しても
    /// frontmatter を持つファイルが存在しない。
    #[must_use]
    pub fn complete_spec(&self) -> Option<CompleteSpec> {
        let index = self.selected?;
        let item = self.items.get(index)?;
        if armature_core::board::is_placeholder(&item.id) {
            return None;
        }
        Some(CompleteSpec {
            id: item.id.clone(),
            title: item.title.clone(),
            children: if item.bundle {
                self.descendants(index)
            } else {
                Vec::new()
            },
        })
    }

    /// `x`。選択中の行を掴む/離す。掴んでいる間は行き先を選ぶだけで、
    /// 盤には触らない。空行(placeholder)は実体が無いので掴めない。
    pub fn toggle_grab(&mut self) -> bool {
        if self.moving.take().is_some() {
            self.set_status(None);
            return true;
        }
        let Some(item) = self.selected_item() else {
            return false;
        };
        if armature_core::board::is_placeholder(&item.id) {
            return false;
        }
        self.moving = Some(item.id.clone());
        self.set_status(None);
        true
    }

    #[must_use]
    pub fn is_moving(&self) -> bool {
        self.moving.is_some()
    }

    /// 行の親(1つ外側のバンドル)。平坦な `items` は親の直後に部分木が
    /// 深さ順で並ぶ(`build_display` の押し順)ので、深さが1つ浅い直近の行が親。
    fn parent_of(&self, index: usize) -> Option<usize> {
        let depth = self.items.get(index)?.depth;
        if depth == 0 {
            return None;
        }
        (0..index).rev().find(|i| self.items[*i].depth == depth - 1)
    }

    /// その行の部分木が終わる位置(この添字は含まない)。
    fn subtree_end(&self, index: usize) -> usize {
        let Some(depth) = self.items.get(index).map(|item| item.depth) else {
            return index;
        };
        index
            + 1
            + self.items[index + 1..]
                .iter()
                .take_while(|item| item.depth > depth)
                .count()
    }

    /// その親の直属の子(空行は除く)。ルート(`None`)は最上段の行。
    fn siblings(&self, parent: Option<usize>) -> Vec<String> {
        let ids: Vec<&Item> = match parent {
            Some(index) => {
                let depth = self.items[index].depth;
                self.items[index + 1..]
                    .iter()
                    .take_while(|item| item.depth > depth)
                    .filter(|item| item.depth == depth + 1)
                    .collect()
            }
            None => self.items.iter().filter(|item| item.depth == 0).collect(),
        };
        ids.into_iter()
            .filter(|item| !armature_core::board::is_placeholder(&item.id))
            .map(|item| item.id.clone())
            .collect()
    }

    /// 掴んだ行を、いまカーソルが居る行と同じ親の下・その行の直前へ置く仕様。
    ///
    /// **行き先はカーソル行そのものではなくカーソル行の親**——バンドルの中へ
    /// 入れたいときは中の行に付ける。この一本の規則だけで、束への出し入れ・
    /// 同じ親の中の並べ替え・ルートへの引き上げが全部言える(見出しに付けたら
    /// 中へ、という規則を混ぜると、束の隣へ並べる先が言えなくなる)。
    ///
    /// 並びは兄弟の順(`order`)で渡す。ルートどうしの順も同じ形。
    fn move_target(&self) -> Result<MoveSpec, String> {
        let moving = self.moving.as_deref().ok_or(tr!("No row is picked up", "掴んでいる行が無い"))?;
        let from = self
            .items
            .iter()
            .position(|item| item.id == moving)
            .ok_or(tr!("The picked-up row is gone", "掴んだ行が見当たらない"))?;
        let to = self.selected.ok_or(tr!("No destination selected", "行き先が選ばれていない"))?;
        if to == from {
            return Err(tr!("That's where it already is", "元の場所です").into());
        }
        if to > from && to < self.subtree_end(from) {
            return Err(tr!("Can't move a task under itself", "自分の配下へは移せない").into());
        }
        let parent = self.parent_of(to);
        let parent_id = parent.map_or(String::new(), |index| self.items[index].id.clone());
        let was = self
            .parent_of(from)
            .map_or(String::new(), |index| self.items[index].id.clone());
        let anchor = &self.items[to].id;
        let mut order = self.siblings(parent);
        order.retain(|id| id != moving);
        let at = order
            .iter()
            .position(|id| id == anchor)
            .unwrap_or(order.len());
        order.insert(at, moving.to_string());
        Ok(MoveSpec {
            id: moving.to_string(),
            title: self.items[from].title.clone(),
            reparent: parent_id != was,
            parent: parent_id,
            order,
        })
    }

    /// `Return`。移せる先なら仕様を返す。移せない先は知らせだけ出して掴んだまま。
    pub fn commit_move(&mut self) -> Option<MoveSpec> {
        match self.move_target() {
            Ok(spec) => {
                // 書き終えた盤を取り込み直したとき、カーソルが移した行に乗るように
                // (`set_board` は選択をIDで引き継ぐ)。
                if let Some(index) = self.items.iter().position(|item| item.id == spec.id) {
                    self.selected = Some(index);
                }
                self.set_status(Some(Ok(tr!(
            format!("Moving {}", spec.id),
            format!("{} を移している", spec.id),
        ))));
                Some(spec)
            }
            Err(message) => {
                self.set_status(Some(Err(message)));
                None
            }
        }
    }

    /// 移し終えた結果を知らせる。一覧そのものは次の取り込みで入れ替わる。
    pub fn moved(&mut self, spec: &MoveSpec, result: Result<(), String>) {
        self.moving = None;
        self.set_status(Some(match result {
            Ok(()) => Ok(tr!(
                format!("Moved {} {}", spec.id, spec.title),
                format!("{} {} を移した", spec.id, spec.title),
            )),
            Err(error) => Err(error),
        }));
    }

    /// 掴んでいる行と、いまの行き先の言い方。パネルの下に出しっぱなしにする。
    fn move_notice(&self) -> Option<String> {
        let moving = self.moving.as_deref()?;
        let to = self.selected.and_then(|index| self.items.get(index));
        let where_to = match to {
            None => tr!("no destination yet", "行き先未定").to_owned(),
            Some(item) => {
                let parent = self
                    .selected
                    .and_then(|index| self.parent_of(index))
                    .and_then(|index| self.items.get(index));
                let place = match parent {
                    Some(parent) => tr!(
                        format!("inside {} {}", parent.id, parent.title),
                        format!("{} {} の中", parent.id, parent.title),
                    ),
                    None => tr!("the top level", "ルート").to_owned(),
                };
                tr!(
                    format!("{place} (above {})", item.id),
                    format!("{place} ({} の上)", item.id),
                )
            }
        };
        Some(tr!(
            format!("{moving} → {where_to} — Return to confirm / x to cancel"),
            format!("{moving} を {where_to} へ — Return で確定 / x で取消"),
        ))
    }

    /// バンドルの下に連なる実在の行。
    ///
    /// 平坦な `items` は親の直後に部分木が深さ順で並ぶ(`build_display` の押し順)ので、
    /// 深さが親まで戻るところで切れば子孫の全部になる。入れ子のバンドルも含む。
    fn descendants(&self, parent: usize) -> Vec<String> {
        let Some(depth) = self.items.get(parent).map(|item| item.depth) else {
            return Vec::new();
        };
        self.items[parent + 1..]
            .iter()
            .take_while(|item| item.depth > depth)
            .filter(|item| !armature_core::board::is_placeholder(&item.id))
            .map(|item| item.id.clone())
            .collect()
    }

    pub fn completing(&mut self, spec: &CompleteSpec) {
        self.set_status(Some(Ok(tr!(
            format!("Marking {}{} done", spec.id, child_note(spec)),
            format!("{} を{}完了へ倒している", spec.id, child_note(spec)),
        ))));
    }

    /// 倒し終えた結果を知らせる。一覧そのものは次の取り込みで入れ替わる
    /// ——ここで自前に消すと、書き込みが失敗した回に行だけが消える。
    pub fn completed(&mut self, spec: &CompleteSpec, result: Result<(), String>) {
        self.set_status(Some(match result {
            Ok(()) => Ok(tr!(
                format!("Marked {} {}{} done", spec.id, spec.title, child_note(spec)),
                format!("{} {} を{}完了にした", spec.id, spec.title, child_note(spec)),
            )),
            Err(error) => Err(error),
        }));
    }

    /// 起動条件の保存結果を表示へ戻す。失敗時は、その間に別の選択が無ければ
    /// 楽観更新した値を元へ戻す。
    #[allow(dead_code)]
    pub fn settings_saved(&mut self, spec: &SettingsSpec, result: Result<(), String>) {
        match result {
            Ok(()) => self.set_status(None),
            Err(error) => {
                if let Some(board) = self.board.as_mut()
                    && board.ui_field(spec.field) == spec.value
                {
                    board.ui = board.ui_with(spec.field, &spec.previous);
                }
                self.set_status(Some(Err(tr!(
                    format!("Couldn't save the launch settings: {error}"),
                    format!("起動条件を保存できない: {error}"),
                ))));
            }
        }
    }

    pub fn move_by(&mut self, step: i32) {
        if self.items.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0);
        let next = if step < 0 {
            current.saturating_sub(step.unsigned_abs() as usize)
        } else {
            current.saturating_add(step as usize).min(self.items.len() - 1)
        };
        self.selected = Some(next);
    }

    /// `l`。バンドル見出しから、その最初の中身へ入る。
    pub fn enter_bundle(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let Some(item) = self.items.get(selected) else {
            return;
        };
        if !item.bundle {
            return;
        }
        let depth = item.depth;
        if self
            .items
            .get(selected + 1)
            .is_some_and(|next| next.depth > depth)
        {
            self.selected = Some(selected + 1);
        }
    }

    /// `h`。現在位置を包んでいる直近のバンドル見出しへ戻る。
    pub fn exit_bundle(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let Some(item) = self.items.get(selected) else {
            return;
        };
        let depth = item.depth;
        if depth == 0 {
            return;
        }
        self.selected = (0..selected)
            .rev()
            .find(|index| {
                self.items
                    .get(*index)
                    .is_some_and(|parent| parent.bundle && parent.depth + 1 == depth)
            })
            .or(self.selected);
    }

    #[must_use]
    pub fn launch_spec(&self) -> Option<LaunchSpec> {
        let board = self.board.as_ref()?;
        let item = self.selected_item()?;
        if armature_core::board::is_placeholder(&item.id) {
            return None;
        }
        Some(LaunchSpec {
            id: item.id.clone(),
            title: item.title.clone(),
            model: board.model().to_string(),
            effort: board.effort().to_string(),
        })
    }

    /// 知らせを差し替える。**時計を一緒に押す**——ここを通さない書き込みは
    /// 消える刻を持たず、パネルに残り続ける。
    fn set_status(&mut self, value: Option<Result<String, String>>) {
        self.status_at = value.is_some().then(std::time::Instant::now);
        self.status = value;
    }

    /// 出しっぱなしの知らせを引っ込める。1秒ごとの Tick から呼ぶ。
    pub fn expire_status(&mut self, now: std::time::Instant) {
        if let Some(at) = self.status_at
            && now.duration_since(at) >= STATUS_LIFETIME
        {
            self.status = None;
            self.status_at = None;
        }
    }

    pub fn launching(&mut self, id: &str) {
        self.set_status(Some(Ok(tr!(
            format!("Starting {id}"),
            format!("{id} を起動中"),
        ))));
    }

    pub fn launched(&mut self, result: Result<String, String>) {
        self.set_status(Some(result));
    }

    fn selected_item(&self) -> Option<&Item> {
        self.selected.and_then(|index| self.items.get(index))
    }

    /// いま選んでいる行の、一覧の中での縦位置(0.0〜1.0)。器はこの比で
    /// 一覧を送る——行の高さを定数で持たなくても、行が揃っている限り
    /// 選んだ行は視野に入る(証明は `lib.rs` の `follow_active_tab`)。
    #[must_use]
    pub fn scroll_ratio(&self) -> Option<f32> {
        let position = self.selected?;
        let last = self.items.len().checked_sub(1)?;
        if last == 0 {
            return Some(0.0);
        }
        #[allow(clippy::cast_precision_loss)]
        Some((position as f32 / last as f32).clamp(0.0, 1.0))
    }

    #[cfg(test)]
    fn selected_id(&self) -> Option<&str> {
        self.selected_item().map(|item| item.id.as_str())
    }

    /// タスクのパネル。**外周を1本の枠で囲う**。モデル/effort の桝もこの枠の内側に入り、
    /// 枠の中のもう一段小さい面として出る。
    pub fn view(&self, id: iced::widget::Id) -> Element<'_, Message> {
        // 一覧が空の間は何も書かない(読み込み中も、タスクが1件も無いときも)。
        // 読めなかったときだけ、そう書く。
        if self.items.is_empty() {
            let body: Element<'_, Message> = match self.error.as_deref() {
                Some(error) => container(text(error).size(11).font(font::UI).color(accent_alert()))
                    .padding(14)
                    .into(),
                None => Space::new().into(),
            };
            return seat(body);
        }

        // **バンドルは配下を囲む**。
        // 囲みは**面の高度差だけ**で出す。枠と影を持つ箱を入れ子にすると面が濁って
        // 親子の境が追えなくなる——枠と影を落とせば同じ入れ子でも濁らない。
        let mut nodes = column![].spacing(6).width(Length::Fill);
        let places: Vec<usize> = (0..self.items.len()).collect();
        let mut at = 0;
        while at < places.len() {
            let index = places[at];
            let item = &self.items[index];
            if item.depth == 0 && item.bundle {
                let end = self.box_end(&places, at);
                nodes = nodes.push(self.bundle_box(&places[at..end], 0));
                at = end;
            } else {
                nodes = nodes.push(self.row_view(index, 0, true));
                at += 1;
            }
        }
        let board = column![nodes].spacing(8).width(Length::Fill);
        // 縦の摘まみは出さない。ホイールでは動くが、
        // パネルの右端に棒が立たない——右の列は幅が狭く、棒1本ぶんが効く。
        let list = container(
            scrollable(board)
                .id(id)
                .on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y))
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::hidden(),
                ))
                .height(Length::Fill),
        )
        .padding(iced::Padding::ZERO.left(8).right(8).bottom(6))
        .width(Length::Fill)
        .height(Length::Fill);
        let mut panel = column![list]
            .spacing(5)
            .padding(iced::Padding::ZERO.top(8))
            .width(Length::Fill)
            .height(Length::Fill);
        if let Some(notice) = self.move_notice() {
            panel = panel.push(
                container(text(notice).size(10).color(accent_attention()))
                    .padding([0, 10])
                    .width(Length::Fill),
            );
        }
        if let Some(status) = &self.status {
            let (message, color) = match status {
                Ok(message) => (message.as_str(), text_muted()),
                Err(message) => (message.as_str(), accent_alert()),
            };
            panel = panel.push(
                container(text(message).size(10).color(color))
                    .padding([0, 10])
                    .width(Length::Fill),
            );
        }
        panel = panel.push(self.launcher_view());
        seat(panel.into())
    }

    /// 1行。**器は行1枚だけ**——バンドルごとの箱(枠+影)は 2026-09-14 に外した。
    /// 入れ子にすると深いほど枠と影が重なって面が濁り、親と子の境が「線」ではなく
    /// 「薄い面」になって追えなくなる。
    /// 階層は**囲み**が出す([`Self::bundle_box`])。`base` は囲みが始まる深さで、
    /// 字下げはそこからの相対で引く——絶対の深さで引くと囲みの詰め物と二重に効いて、
    /// 深い行がパネルの右端へ流れる。`guide` は囲みの外の行だけ true(囲みの中で枝を
    /// 引くと、面の境と線の2本立てになって読みどころが割れる)。
    fn row_view(&self, index: usize, base: usize, guide: bool) -> Element<'_, Message> {
        let item = &self.items[index];
        let active = self.selected == Some(index);
        let bundle = item.bundle;
        // 選んでいる行は面で塗らない。題を太字にして光らせる——時計・暦の今日と同じ
        // 「いま」の合図。ほかの行は本文より1段引いて、光る行との差を開ける。
        let title_ink = if active {
            palette::text_lit()
        } else {
            text_secondary()
        };
        let body: Element<'_, Message> = {
            let (mark, mark_color) = self.mark_of(item);
            let (id, id_color) = row_id(item);
            // **`spacing` は使わない。**要素の間に一律の隙間が入ると印の桝が
            // `task_tree` の言う桁からずれ、枝の縦線が印の真下を通らなくなる。
            // 隙間は桝として明示的に置く(ドラフト一覧と同じ作法)。
            row![
                // 深さは字下げで出す。桁の答えは `task_tree` が持つ。
                Space::new().width(Length::Fixed(
                    task_tree::mark_x(item.depth.saturating_sub(base)) - task_tree::PAD_X
                )),
                // 印は桝の**中央**に寄せる——放っておくと左詰めになり、枝の縦線が
                // 印の墨からずれる(`session_tree` が同じ穴を踏んでいる)。
                text(mark)
                    .size(if bundle { BUNDLE_MARK_SIZE } else { MARK_SIZE })
                    .color(mark_color)
                    .width(Length::Fixed(task_tree::MARK_BOX))
                    .align_x(iced::widget::text::Alignment::Center),
                Space::new().width(Length::Fixed(MARK_GAP)),
                text(id).size(ID_SIZE).color(id_color),
                Space::new().width(Length::Fixed(MARK_GAP)),
                // 題は**1行で省略する**。折り返すと1件が2〜3行を食い、パネルに収まる
                // 件数が半分以下になって「全体が把握できない」。
                // 待ちの行は題を面へ沈める。**分けているのは明るさで、色ではない**
                // ——意味色は生死の軸が使っているので、そこへ割り込ませない。
                self.title_view(item, title_ink, active),
                Space::new().width(Length::Fixed(7.0)),
            ]
            .spacing(0)
            .align_y(Alignment::Center)
            .into()
        };
        let row = button(body)
            .padding([task_tree::PAD_Y, task_tree::PAD_X])
            .width(Length::Fill)
            .clip(true)
            .on_press(Message::Select(index))
            .style(move |_, status| selection_style(status));
        if !guide {
            return row.into();
        }
        let guide = task_tree::Tree::new(
            item.stems.clone(),
            item.trunk,
            palette::with_alpha(text_faint(), 0.45),
        );
        // 枝は**行の上に重ねる**。行の中に置くと詰め物のぶん上下が余り、隣の行の線と
        // 繋がらない。枝の Widget は `update` を実装していないので押し心地は変わらない。
        if guide.is_empty() {
            row.into()
        } else {
            stack![row, guide].into()
        }
    }

    /// 行の題。束の見出しと選んでいる行は太字。選んでいる行は光らせる。
    fn title_view<'a>(&self, item: &'a Item, ink: iced::Color, glowing: bool) -> Element<'a, Message> {
        let ink = if item.blocked { sunk(ink) } else { ink };
        let font = if item.bundle || glowing {
            font::UI_STRONG
        } else {
            font::UI
        };
        if glowing {
            glow::glow(
                move |ink| ellipsized::styled(&item.title, TITLE_SIZE, ink, font),
                ink,
                TITLE_GLOW,
            )
        } else {
            ellipsized::styled(&item.title, TITLE_SIZE, ink, font)
        }
    }

    /// `places[at]` の配下がどこまで続くか(自分より浅い行が来たら終わり)。
    fn box_end(&self, places: &[usize], at: usize) -> usize {
        let base = self.items[places[at]].depth;
        let mut end = at + 1;
        while end < places.len() && self.items[places[end]].depth > base {
            end += 1;
        }
        end
    }

    /// バンドル1つぶんの囲み。見出しの行を頭に置き、配下をその中へ積む。
    ///
    /// 段は**面の高度差だけ**で作る(`palette` の elevation ladder)。入れ子は
    /// 2段まで——CARD < RAISED < ACTIVE の3段しか無く、ACTIVE は選んでいる行が
    /// 使っている。3段目から下は字下げで出す。
    fn bundle_box(&self, places: &[usize], level: usize) -> Element<'_, Message> {
        let base = self.items[places[0]].depth;
        let mut inner = column![].spacing(0).width(Length::Fill);
        inner = inner.push(self.row_view(places[0], base, false));
        let mut at = 1;
        while at < places.len() {
            let index = places[at];
            let item = &self.items[index];
            if level == 0 && item.depth == base + 1 && item.bundle {
                let end = self.box_end(places, at);
                inner = inner.push(self.bundle_box(&places[at..end], level + 1));
                at = end;
            } else {
                inner = inner.push(self.row_view(index, base, false));
                at += 1;
            }
        }
        // **段は1つずつ確実に離す**。地(カード) < 囲み0 < 囲み1 < ホバー < 選択 の
        // 5段が単調に上がるように取る。半段刻みにすると隣どうしの差が
        // 10px のパネルでは読めない——ノートパネルが RAISED / ACTIVE の丸ごと1段で
        // 効いていたのは、あちらに選択とホバーが無くて段を2つ使い切れたから。
        let face = if level == 0 {
            surface_raised()
        } else {
            palette::mix(surface_raised(), surface_active(), 0.5)
        };
        container(inner)
            .padding([3, 3])
            .width(Length::Fill)
            .style(move |_| iced::widget::container::Style {
                background: Some(Background::Color(face)),
                border: Border {
                    radius: palette::radius_card().into(),
                    ..Border::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// 行頭の印。掴んでいる行だけは状態より先に「掴んでいる」を出す
    /// ——カーソルは行き先へ離れていくので、印が無いと何を持っているか読めない。
    fn mark_of(&self, item: &Item) -> (&'static str, iced::Color) {
        if self.moving.as_deref() == Some(item.id.as_str()) {
            return ("↕", accent_attention());
        }
        row_mark(item)
    }

    fn launcher_view(&self) -> Element<'_, Message> {
        let (active_model, active_effort) = self
            .board
            .as_ref()
            .map_or(("", ""), |board| (board.model(), board.effort()));

        let mut settings = row![].spacing(2).align_y(Alignment::Center);
        // 選んだものに色も面も付けない。字を2段明るくするだけ——下端に字だけが並ぶ。
        let chosen = |_: iced::Color| text_secondary();
        for model in Model::ALL {
            let active = active_model == model.value();
            let value = model.value();
            settings = settings.push(
                button(launcher_text(model.label(), if active {
                    chosen(palette::model(value))
                } else {
                    text_faint()
                }))
                .padding([2, 4])
                .on_press(Message::SetModel(model))
                .style(|_, status| launcher_option_style(status)),
            );
        }
        settings = settings.push(launcher_text("/", text_faint()));
        for effort in Effort::ALL {
            let active = active_effort == effort.value();
            let value = effort.value();
            settings = settings.push(
                button(launcher_text(value, if active {
                    chosen(palette::effort(value))
                } else {
                    text_faint()
                }))
                .padding([2, 3])
                .on_press(Message::SetEffort(effort))
                .style(|_, status| launcher_option_style(status)),
            );
        }

        // 箱に入れない。字だけがパネルの下端に並ぶ。
        container(settings)
            .align_x(Alignment::Center)
            .padding([6, 7])
            .width(Length::Fill)
            .into()
    }
}

/// パネルそのものの器。面は画面の地と同じで、隣のパネルとの境は窓の側が線で引く。
fn seat(body: Element<'_, Message>) -> Element<'_, Message> {
    container(body)
        .padding(4)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| palette::card_style())
        .into()
}

/// 行の面。**行の面が言うのは「指を乗せている」だけ**——選んでいる行は面で塗らず、
/// 題を光らせる([`Tasks::row_view`])。束は囲み([`Tasks::bundle_box`])が1段浮かせる。
///
/// 囲みはホバーの面と近い高度なので、**ホバーだけはもう1段上げる**
/// ——同じ色になると、指を乗せた行が見出しに溶ける。
fn selection_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let hovered = matches!(
        status,
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed
    );
    let face = if hovered {
        // **囲みより上**。囲みが RAISED と ACTIVE の半ばまで使うので、
        // ホバーはその上・選択の手前に置く(2026-09-18)。
        Some(palette::mix(surface_raised(), surface_active(), 0.8))
    } else {
        // 見出しの帯は**囲み**([`Tasks::bundle_box`])が持つ。行でも塗ると
        // 同じ面を二度塗ることになり、囲みの縁が見えなくなる(2026-09-18)。
        None
    };
    iced::widget::button::Style {
        background: face.map(Background::Color),
        text_color: text_primary(),
        border: Border {
            radius: palette::radius_control().into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

fn launcher_option_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let hovered = matches!(
        status,
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed
    );
    // 選んでいるものは字の明るさだけで示す(`launcher_view`)。面は指を乗せたときだけ。
    let face = hovered.then_some(surface_raised());
    iced::widget::button::Style {
        background: face.map(Background::Color),
        text_color: text_primary(),
        border: Border {
            radius: palette::radius_control().into(),
            ..Border::default()
        },
        shadow: Shadow::default(),
        snap: true,
    }
}

/// ランチャの字。題と同じヒラギノで組む。
fn launcher_text(content: &str, color: iced::Color) -> iced::widget::Text<'_> {
    text(content).size(9).color(color).font(font::UI)
}

/// 木を**深さ順の1本**に潰して `items` へ積む。見出しを先、その配下を続けて積むので、
/// 並びはそのまま画面の行順になる(`board.rs::flatten` と同じ順)。
fn push_tree(
    board: &armature_core::board::Board,
    node: &armature_core::board::Node,
    depth: usize,
    items: &mut Vec<Item>,
) {
    match node {
        armature_core::board::Node::Box { id, body } => {
            push_item(board, id, None, depth, true, items);
            for child in body {
                push_tree(board, child, depth + 1, items);
            }
        }
        armature_core::board::Node::Row { id, title } => {
            push_item(board, id, Some(title), depth, false, items);
        }
        // **空バンドルの投下口は積まない**。子が全員完了した束は見出しだけが残るので、
        // 受け皿の空行を置くと囲みの中に何も無い帯が出る——パネルから手でタスクを移す
        // 操作を使わなくなったので、受け皿ごと落とす。
        armature_core::board::Node::Empty { .. } => {}
    }
}

/// 行ごとに「どの列がこの行を貫いて下へ続くか」を数える。**枝の形はここ1箇所**で
/// 決まり、[`task_tree::Tree`] はそれを引くだけ。
///
/// 列 `a` はふたたび深さ `a + 1` の行が現れるかどうかで決まる——間により浅い行が
/// 挟まれば、その束はそこで閉じている。閉じた列を通すと、終わったはずの束の縦線が
/// 下の束まで伸びる。
fn wire_stems(rows: &mut [Item]) {
    let depths: Vec<usize> = rows.iter().map(|item| item.depth).collect();
    for (index, depth) in depths.iter().copied().enumerate() {
        let stems = (0..depth)
            .map(|column| {
                depths[index + 1..]
                    .iter()
                    .copied()
                    .find(|next| *next <= column + 1)
                    .is_some_and(|next| next == column + 1)
            })
            .collect();
        rows[index].stems = stems;
        rows[index].trunk = depths.get(index + 1).is_some_and(|next| *next == depth + 1);
    }
}

/// バンドル見出しの沈み。**見出しが言うのは「この束に手を付けられる仕事があるか」**
/// ——束を開く前にそれが読めたほうがいい。だから配下の実態が、束自身の `待ち` より強い。
///
/// - 配下に着手可が1件でもあれば沈めない(束自身に `待ち` が書いてあっても)
///   ——見出しの `待ち` は束の次の一歩の話で、中の仕事が止まっている証拠ではない。
///   ここを沈めると、やれる仕事を抱えた束が死んで見える
/// - 配下が待ちだけなら沈める
/// - 子を1件も持たない束は触らない——「待たされている」のではなく「まだ空」で、
///   別のことを言っている
///
/// `rows` は一覧の**深さ順の並び**(`wire_stems` と同じ前提)なので、
/// 自分より深い行が続く限りが配下。後ろから畳むと、深い束の答えが先に決まって
/// 浅い束がそれを読める。
fn wire_blocked_bundles(rows: &mut [Item]) {
    for index in (0..rows.len()).rev() {
        if !rows[index].bundle {
            continue;
        }
        let depth = rows[index].depth;
        let mut has_child = false;
        let mut any_open = false;
        for child in &rows[index + 1..] {
            if child.depth <= depth {
                break;
            }
            has_child = true;
            if !child.blocked {
                any_open = true;
                break;
            }
        }
        if has_child {
            rows[index].blocked = !any_open;
        }
    }
}

/// 行頭の印と色。**状態の印は2値だけ**——`·` 着手可 / `⊘` 触れない。
///
/// 落としたのは 金`●`(セッションが返事待ち)・灰`○`(セッションが停止)・
/// 金`◎`の3つ。**2つの軸が同じ桝を取り合っていた**のが増えた原因で、
/// `·` `⊘` はタスクの `待ち` 欄の話、`●` `○` はそのタスクに紐づくセッションの生死の話。
/// パネルはタスクの一覧であってセッションの一覧ではないので、後者はパネルから落とす。
/// `◎` (判断待ち) は着手可の側へ倒す——利用者が動けば進む行なので、触れない行とは別。
///
fn row_mark(item: &Item) -> (&'static str, iced::Color) {
    if item.bundle {
        // バンドルは束の字([`BUNDLE_MARK`])。配下の箇条点より一段濃い色で置く
        // ——印の濃さそのものが「こちらが親」を言う。枝の幹はこの印の墨の
        // 真下から生える。沈んだ束は色だけ一段落とし、字は束のまま
        // ——束であることは待ちかどうかと関係なく読めなければならない。
        let ink = if item.blocked {
            sunk(text_secondary())
        } else {
            text_secondary()
        };
        return (BUNDLE_MARK, ink);
    }
    if item.blocked {
        return (BLOCKED_MARK, sunk(text_faint()));
    }
    // 素の行は箇条の点。**枝の肘がここで止まる**ので、桝を空にしない
    // ——空だと線が宙で切れて見える(`task_tree` の頭注)。
    ("·", text_faint())
}

/// 行の番号と色。**目印の見せ方にかかわらず出す**——2026-09-23 の意匠刷新で
/// 見本(案1)に合わせ、光のときだけ番号を空にしていた。番号が無いと、盤の外
/// (タスクの md・会話・台帳)で呼んでいる行がどれか引けない。
///
/// 番号も題と一緒に沈める。片方だけ沈めると「字が薄い行」に見えて
/// 「沈んだ行」には見えない——行として1段落ちていることが読みどころ。
fn row_id(item: &Item) -> (&str, iced::Color) {
    let ink = if item.bundle { text_muted() } else { text_faint() };
    (item.id.as_str(), if item.blocked { sunk(ink) } else { ink })
}

#[allow(clippy::too_many_arguments)]
fn push_item(
    board: &armature_core::board::Board,
    id: &str,
    title: Option<&str>,
    depth: usize,
    bundle: bool,
    items: &mut Vec<Item>,
) -> usize {
    let item = items.len();
    items.push(Item {
        id: id.to_string(),
        title: board.label_of(id, title),
        depth,
        bundle,
        blocked: board.is_blocked(id),
        // 枝の桁は一覧を積み終えてから `wire_stems` が入れる。
        stems: Vec::new(),
        trunk: false,
    });
    item
}

pub fn fetch() -> Result<armature_core::board::Board, String> {
    let mut vault = armature_core::vault::Vault::new();
    let source = vault.data().to_string();
    armature_core::board::Board::parse(&source).ok_or_else(|| tr!("The task list is malformed", "タスク一覧の形式が不正").into())
}

/// `x` → `Return` の副作用。親替えを先に済ませ、兄弟の並びを後から書く。
pub fn persist_move(spec: &MoveSpec) -> Result<(), String> {
    let writer = armature_core::write::Writer::new();
    if spec.reparent {
        armature_core::write::move_task(&writer, &spec.id, &spec.parent)?;
    }
    armature_core::write::save_order(&writer, &spec.order)
}

/// 子の件数を知らせに挟む字。子が無いバンドル・素のタスクでは空。
fn child_note(spec: &CompleteSpec) -> String {
    if spec.children.is_empty() {
        String::new()
    } else {
        let count = spec.children.len();
        tr!(
            format!(" and its {count} subtasks"),
            format!("子{count}件ごと"),
        )
    }
}

/// `dd` の副作用。状態を「完了」へ倒す。親は blocking task から呼ぶこと。
///
/// バンドルは子孫を先に倒し、見出しを最後に倒す。途中で落ちたときに残るのが
/// 「開いた親」側になり、一覧から見えるのでやり直せる(逆順だと閉じた親の下に
/// 開いた子が隠れる)。
pub fn persist_complete(spec: &CompleteSpec) -> Result<(), String> {
    let writer = armature_core::write::Writer::new();
    for id in spec.children.iter().chain(std::iter::once(&spec.id)) {
        armature_core::write::set_state(&writer, id, armature_core::vault::DONE)
            .map_err(|error| format!("{id}: {error}"))?;
    }
    Ok(())
}

/// model/effort 選択の副作用。`SettingsSpec` は既存の `ui` 全体を保つので、
/// このパネルが知らない設定を消さない。
pub fn persist_settings(spec: &SettingsSpec) -> Result<(), String> {
    armature_core::write::save_launch_settings(&armature_core::write::Writer::new(), &spec.ui).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn board() -> armature_core::board::Board {
        armature_core::board::Board::parse(
            r#"{
                "roots":["100"], "bundles":["100","200"],
                "children":{"100":["200"]},
                "members":{"100":[{"id":"101","title":"親の仕事"}],"200":[{"id":"201","title":"子の仕事"}]},
                "tagLabel":{"100":"親バンドル","200":"子バンドル"},
                "tasks":{"100":{"title":"親バンドル"},"101":{"title":"親の仕事"},"200":{"title":"子バンドル"},"201":{"title":"子の仕事"}},
                "ui":{"model":"opus","effort":"high"}
            }"#,
        )
        .expect("盤")
    }

    fn two_bundle_board() -> armature_core::board::Board {
        armature_core::board::Board::parse(
            r#"{
                "roots":["100","300"],
                "bundles":["100","300"], "children":{},
                "members":{"100":[{"id":"101","title":"今の仕事"}],"300":[{"id":"301","title":"後の仕事"}]},
                "tagLabel":{"100":"今","300":"後"},
                "tasks":{"100":{"title":"今"},"101":{"title":"今の仕事"},"300":{"title":"後"},"301":{"title":"後の仕事"}},
                "ui":{"model":"opus","effort":"high"}
            }"#,
        )
        .expect("束が2つの盤")
    }

    fn board_with_empty_bundle() -> armature_core::board::Board {
        armature_core::board::Board::parse(
            r#"{
                "roots":["400"], "bundles":["400"],
                "children":{}, "members":{},
                "tagLabel":{"400":"空バンドル"},
                "tasks":{"400":{"title":"空バンドル"}},
                "ui":{"model":"opus","effort":"high"}
            }"#,
        )
        .expect("空バンドルを持つ盤")
    }

    #[test]
    fn bundles_keep_their_hierarchy_and_every_row_keeps_its_id() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        assert_eq!(
            tasks
                .items
                .iter()
                .map(|item| (item.id.as_str(), item.depth, item.bundle))
                .collect::<Vec<_>>(),
            vec![
                ("100", 0, true),
                ("101", 1, false),
                ("200", 1, true),
                ("201", 2, false)
            ]
        );
        // 階層は入れ子の器ではなく、行ごとの枝が持つ。100 は配下を持つので幹を
        // 下ろし、101 は 200 が続くので列 0 を貫き、200 は末子なので肘で止まる。
        assert_eq!(
            tasks
                .items
                .iter()
                .map(|item| (item.stems.as_slice(), item.trunk))
                .collect::<Vec<_>>(),
            vec![
                (&[][..], true),
                (&[true][..], false),
                (&[false][..], true),
                (&[false, false][..], false),
            ]
        );
    }

    #[test]
    fn a_bundle_takes_its_whole_subtree_down_with_it() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        assert_eq!(tasks.selected_id(), Some("100"));
        let bundle = tasks.complete_spec().expect("親バンドルは倒せる");
        assert_eq!(bundle.children, ["101", "200", "201"]);

        tasks.select(1);
        let leaf = tasks.complete_spec().expect("素のタスクも倒せる");
        assert_eq!(leaf.id, "101");
        assert!(leaf.children.is_empty(), "タスクは自分だけを倒す");

        tasks.set_board(board_with_empty_bundle());
        let empty = tasks.complete_spec().expect("空バンドルの見出しは倒せる");
        assert!(empty.children.is_empty(), "空行を倒しにいってはいけない");
    }

    /// 掴んだ行の落とし先は「カーソル行そのもの」ではなく「カーソル行の親」。
    /// 101(親バンドル直属)を 201(子バンドルの中)へ付けると 200 の子になる。
    #[test]
    fn x_then_return_adopts_the_parent_of_the_row_under_the_cursor() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.select(1); // 101
        assert!(tasks.toggle_grab());
        tasks.select(3); // 201(子バンドルの中)
        let spec = tasks.commit_move().expect("移せる");
        assert_eq!(spec.id, "101");
        assert!(spec.reparent);
        assert_eq!(spec.parent, "200");
        assert_eq!(spec.order, vec!["101".to_string(), "201".to_string()]);
    }

    /// 同じ親の中で位置だけ変える回は親替えを打たない(並びだけ書く)。
    #[test]
    fn moving_between_siblings_only_rewrites_the_child_order() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.select(2); // 200(子バンドル)
        assert!(tasks.toggle_grab());
        tasks.select(1); // 101 の上へ
        let spec = tasks.commit_move().expect("移せる");
        assert!(!spec.reparent);
        assert_eq!(spec.parent, "100");
        assert_eq!(spec.order, vec!["200".to_string(), "101".to_string()]);
    }

    /// ルートへ引き上げる回は、ルートどうしの並びを丸ごと渡す。
    #[test]
    fn promoting_to_root_sends_the_order_of_every_root() {
        let mut tasks = Tasks::default();
        tasks.set_board(two_bundle_board());
        tasks.select(3); // 301(300 の中)
        assert!(tasks.toggle_grab());
        tasks.select(2); // 300 の見出し=ルート直下
        let spec = tasks.commit_move().expect("移せる");
        assert!(spec.reparent);
        assert!(spec.parent.is_empty());
        assert_eq!(spec.order, vec!["100", "301", "300"]);
    }

    /// 空バンドルの空行は掴めない(実体が無い)が、行き先にはできる。
    #[test]
    fn a_grabbed_row_moves_to_the_place_the_cursor_points_at() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.select(1); // 101
        assert!(tasks.toggle_grab());
        tasks.select(2); // 200 の見出し=100 の子のまま
        let spec = tasks.commit_move().expect("移せる");
        assert!(!spec.reparent);
        assert_eq!(spec.order, vec!["101".to_string(), "200".to_string()]);
    }

    /// 自分の配下へは移せない。バンドルごと自分の中に沈むと盤から辿れなくなる。
    #[test]
    fn a_bundle_refuses_to_move_inside_its_own_subtree() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.select(0); // 100
        assert!(tasks.toggle_grab());
        tasks.select(3); // 自分の孫
        assert!(tasks.commit_move().is_none());
        assert!(tasks.is_moving(), "移せない先では掴んだまま");
    }

    #[test]
    fn x_twice_lets_go_of_the_row() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        assert!(tasks.toggle_grab());
        assert!(tasks.is_moving());
        assert!(tasks.toggle_grab());
        assert!(!tasks.is_moving());
    }

    #[test]
    fn l_enters_a_bundle_and_h_returns_to_its_heading() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        assert_eq!(tasks.selected_id(), Some("100"));
        tasks.enter_bundle();
        assert_eq!(tasks.selected_id(), Some("101"));
        tasks.exit_bundle();
        assert_eq!(tasks.selected_id(), Some("100"));
    }

    #[test]
    fn launching_uses_the_selected_id_and_the_board_conditions() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.move_by(1);
        let spec = tasks.launch_spec().expect("実在タスクは起動できる");
        assert_eq!(
            spec,
            LaunchSpec {
                id: "101".into(),
                title: "親の仕事".into(),
                model: "opus".into(),
                effort: "high".into(),
            }
        );
        assert_eq!(spec.prompt(), "/start 101");
    }

    #[test]
    fn model_and_effort_selection_change_the_next_launch_and_emit_save_actions() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.move_by(1);

        assert!(tasks.update(Message::SetModel(Model::Fable)));
        let Some(Action::SaveSettings(model)) = tasks.take_action() else {
            panic!("model変更は保存actionを返す");
        };
        assert_eq!((model.field, model.value), ("model", "fable"));
        assert_eq!(model.previous, "opus");
        assert_eq!(model.ui["effort"], "high", "知らないui欄を落とさない");

        assert!(tasks.update(Message::SetEffort(Effort::Max)));
        let Some(Action::SaveSettings(effort)) = tasks.take_action() else {
            panic!("effort変更は保存actionを返す");
        };
        assert_eq!((effort.field, effort.value), ("effort", "max"));
        assert_eq!(effort.previous, "high");
        let launch = tasks.launch_spec().expect("選択中の起動条件");
        assert_eq!(
            (launch.model.as_str(), launch.effort.as_str()),
            ("fable", "max")
        );

        assert!(!tasks.update(Message::SetEffort(Effort::Max)));
        assert!(tasks.take_action().is_none(), "同じ値は再保存しない");
    }

    #[test]
    fn model_and_effort_cycle_to_the_next_candidate_and_emit_save_actions() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());

        assert!(tasks.cycle_model());
        let Some(Action::SaveSettings(model)) = tasks.take_action() else {
            panic!("model cycle must save");
        };
        assert_eq!(
            (model.field, model.previous, model.value),
            ("model", "opus".into(), "fable")
        );

        assert!(tasks.cycle_effort());
        let Some(Action::SaveSettings(effort)) = tasks.take_action() else {
            panic!("effort cycle must save");
        };
        assert_eq!(
            (effort.field, effort.previous, effort.value),
            ("effort", "high".into(), "xhigh")
        );
    }

    /// 「NNN を起動中」がパネルの下に残り続けていた。
    /// 出してから [`STATUS_LIFETIME`] で消える。
    #[test]
    fn a_launch_notice_clears_itself_after_a_few_seconds() {
        let _ja = armature_core::lang::scoped(armature_core::lang::Lang::Ja);
        let mut tasks = Tasks::default();
        tasks.launching("885");
        assert_eq!(tasks.status.as_ref().unwrap().as_deref(), Ok("885 を起動中"));

        // まだ出したばかりなら消えない。
        tasks.expire_status(std::time::Instant::now());
        assert!(tasks.status.is_some(), "出した直後に消えてはいけない");

        // 寿命を越えた刻から見ると消えている。
        let later = std::time::Instant::now() + STATUS_LIFETIME + Duration::from_millis(1);
        tasks.expire_status(later);
        assert!(tasks.status.is_none(), "知らせが残り続けている");
        assert!(tasks.status_at.is_none(), "時計が残っている");

        // 何も出ていないところで刻んでも壊れない。
        tasks.expire_status(later);
        assert!(tasks.status.is_none());
    }

    #[test]
    fn footer_keeps_every_model_and_effort_candidate_available() {
        assert_eq!(Model::ALL.map(Model::value), ["sonnet", "opus", "fable"]);
        assert_eq!(
            Effort::ALL.map(Effort::value),
            ["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn a_successful_setting_save_does_not_repeat_the_selected_value_as_status_text() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        assert!(tasks.update(Message::SetModel(Model::Fable)));
        let Some(Action::SaveSettings(spec)) = tasks.take_action() else {
            panic!("保存指定");
        };
        tasks.settings_saved(&spec, Ok(()));
        assert!(tasks.status.is_none());
    }

    #[test]
    fn failed_setting_save_rolls_back_only_its_optimistic_value() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        assert!(tasks.update(Message::SetModel(Model::Fable)));
        let Some(Action::SaveSettings(spec)) = tasks.take_action() else {
            panic!("保存指定");
        };
        tasks.settings_saved(&spec, Err("書けない".into()));
        assert_eq!(
            tasks.board.as_ref().map(|board| board.model()),
            Some("opus")
        );
    }

    /// 状態の印は**2値だけ**。セッションが走っていようが
    /// 返事を待っていようが止まっていようが、タスクとして着手できるなら箇条の点。
    #[test]
    fn the_seat_marks_tasks_not_sessions() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        let row = tasks
            .items
            .iter()
            .position(|i| !i.bundle)
            .expect("通常タスクの行");
        assert_eq!(row_mark(&tasks.items[row]), ("·", text_faint()));
        assert_ne!(row_mark(&tasks.items[row]).0, BLOCKED_MARK);
    }

    /// 待ちの盤。`101` だけが着手可で、`201`(唯一の子)が待ちなので束 `200` も沈む。
    fn blocked_board() -> armature_core::board::Board {
        armature_core::board::Board::parse(
            r#"{
                "roots":["100"], "bundles":["100","200"],
                "children":{"100":["200"]},
                "members":{"100":[{"id":"101","title":"親の仕事"}],"200":[{"id":"201","title":"子の仕事"}]},
                "tagLabel":{"100":"親バンドル","200":"子バンドル"},
                "tasks":{"100":{"title":"親バンドル"},"101":{"title":"親の仕事"},
                         "200":{"title":"子バンドル"},"201":{"title":"子の仕事","waiting":"778"}},
                "ui":{"model":"opus","effort":"high"}
            }"#,
        )
        .expect("待ちのある盤")
    }

    #[test]
    fn a_waiting_task_is_marked_and_dimmed_while_a_ready_one_is_not() {
        let mut tasks = Tasks::default();
        tasks.set_board(blocked_board());
        let ready = tasks.items.iter().position(|i| i.id == "101").expect("101");
        let waiting = tasks.items.iter().position(|i| i.id == "201").expect("201");
        assert!(!tasks.items[ready].blocked, "待ちが無い行は着手可");
        assert!(tasks.items[waiting].blocked, "待ちが残る行は沈む");
        assert_eq!(row_mark(&tasks.items[ready]), ("·", text_faint()));
        assert_eq!(
            row_mark(&tasks.items[waiting]),
            (BLOCKED_MARK, sunk(text_faint()))
        );
        assert_ne!(BLOCKED_MARK, "○", "止まったパネルの印と待ちの印は別の字");
    }

    /// 番号はどの行にも出す——束にも待ちの行にも。
    /// [`row_id`] は目印の見せ方を読まない。9/23 の意匠刷新は光のときだけ番号を空にしていた。
    ///
    #[test]
    fn every_row_shows_its_id_and_the_id_sinks_with_its_row() {
        let mut tasks = Tasks::default();
        tasks.set_board(blocked_board());
        let at = |id: &str| tasks.items.iter().find(|i| i.id == id).expect(id);
        assert_eq!(row_id(at("100")), ("100", text_muted()), "束の番号は一段濃い");
        assert_eq!(row_id(at("101")), ("101", text_faint()));
        assert_eq!(
            row_id(at("200")),
            ("200", sunk(text_muted())),
            "配下が待ちだけの束は番号も沈む"
        );
        assert_eq!(
            row_id(at("201")),
            ("201", sunk(text_faint())),
            "待ちの行は番号も題と一緒に沈む"
        );
    }

    #[test]
    fn a_task_waiting_only_on_you_stays_on_the_actionable_side() {
        // `待ち: 自分` は「自分ではどうしようもない」ブロッカーではない——利用者が動けば
        // 進む行なので沈めない。専用の印 `◎` は
        // 2026-09-18 に落とし、着手可の側と同じ箇条の点で出す。
        let board = armature_core::board::Board::parse(
            r#"{
                "roots":["100"], "bundles":["100"],
                "children":{},
                "members":{"100":[{"id":"101","title":"自分待ち"},{"id":"102","title":"理由付き"},
                                  {"id":"103","title":"混在"},{"id":"104","title":"外部"}]},
                "tagLabel":{"100":"束"},
                "tasks":{"100":{"title":"束"},
                         "101":{"title":"自分待ち","waiting":"自分"},
                         "102":{"title":"理由付き","waiting":"自分 (Aさんへの相談)"},
                         "103":{"title":"混在","waiting":"自分, Bさん (PR のレビュー)"},
                         "104":{"title":"外部","waiting":"接合テスト環境"}},
                "ui":{"model":"opus","effort":"high"}
            }"#,
        )
        .expect("盤");
        let mut tasks = Tasks::default();
        tasks.set_board(board);
        let at = |id: &str| tasks.items.iter().position(|i| i.id == id).expect(id);
        for id in ["101", "102"] {
            let row = at(id);
            assert!(!tasks.items[row].blocked, "{id}: 自分の札だけなら沈めない");
            assert_eq!(row_mark(&tasks.items[row]), ("·", text_faint()));
        }
        for id in ["103", "104"] {
            let row = at(id);
            assert!(tasks.items[row].blocked, "{id}: 外部要因が混ざれば沈む");
            assert_eq!(
                row_mark(&tasks.items[row]),
                (BLOCKED_MARK, sunk(text_faint()))
            );
        }
    }

    #[test]
    fn a_bundle_sinks_when_nothing_under_it_can_be_started() {
        let mut tasks = Tasks::default();
        tasks.set_board(blocked_board());
        let child = tasks.items.iter().position(|i| i.id == "200").expect("200");
        let root = tasks.items.iter().position(|i| i.id == "100").expect("100");
        assert!(tasks.items[child].blocked, "配下が待ちだけの束は沈む");
        assert_eq!(
            row_mark(&tasks.items[child]),
            (BUNDLE_MARK, sunk(text_secondary())),
            "沈んだ束の印は面へ溶ける"
        );
        assert!(
            sunk(text_secondary()).a < 1.0,
            "沈みは役割色の1段落としではなく、面への溶かしで作る"
        );
        assert!(
            !tasks.items[root].blocked,
            "配下に着手可(101)が1件でもあれば束は沈めない"
        );
        assert_eq!(tasks.items[root].id, "100");
        assert_eq!(row_mark(&tasks.items[root]), (BUNDLE_MARK, text_secondary()));
    }

    #[test]
    fn a_bundle_with_live_work_under_it_stays_lit_even_if_it_waits_on_something() {
        // 束自身の `待ち` は束の次の一歩の話で、中の仕事が止まっている証拠ではない。
        // ここを沈めると、やれる仕事(101)を抱えた束が死んで見える。
        let board = armature_core::board::Board::parse(
            r#"{
                "roots":["100"], "bundles":["100"],
                "children":{},
                "members":{"100":[{"id":"101","title":"やれる仕事"}]},
                "tagLabel":{"100":"束"},
                "tasks":{"100":{"title":"束","waiting":"自分"},"101":{"title":"やれる仕事"}},
                "ui":{"model":"opus","effort":"high"}
            }"#,
        )
        .expect("盤");
        let mut tasks = Tasks::default();
        tasks.set_board(board);
        let root = tasks.items.iter().position(|i| i.id == "100").expect("100");
        assert!(!tasks.items[root].blocked, "配下の実態が束自身の待ちより強い");
    }

    #[test]
    fn an_empty_bundle_is_not_treated_as_waiting() {
        let mut tasks = Tasks::default();
        tasks.set_board(board_with_empty_bundle());
        assert!(
            tasks.items.iter().all(|i| !i.blocked),
            "子が1件も無い束は「待たされている」ではなく「まだ空」"
        );
    }

    #[test]
    fn moving_past_the_task_list_stops_at_its_edges() {
        let mut tasks = Tasks::default();
        tasks.set_board(board());
        tasks.move_by(-1);
        assert_eq!(tasks.selected_id(), Some("100"));
        tasks.move_by(99);
        assert_eq!(tasks.selected_id(), Some("201"));
    }

    /// 空バンドルは**見出しだけ**。受け皿の空行を
    /// 積まなくなったので、入っても出ても見出しに留まる。
    #[test]
    fn an_empty_bundle_is_just_its_heading() {
        let mut tasks = Tasks::default();
        tasks.set_board(board_with_empty_bundle());
        assert_eq!(tasks.selected_id(), Some("400"));
        assert!(
            !tasks.items.iter().any(|i| i.id.ends_with("::empty")),
            "投下口の空行は積まない"
        );

        tasks.enter_bundle();
        assert_eq!(tasks.selected_id(), Some("400"), "入る先が無い");
        tasks.exit_bundle();
        assert_eq!(tasks.selected_id(), Some("400"));
    }

    /// 行の面が言うのは「指を乗せている」だけ。選んでいる行は面で塗らず、題が光る
    /// ——面で塗ると、線で仕切った画面の中にそこだけ箱が浮く。枠も引かない。
    #[test]
    fn rows_only_lift_under_the_pointer() {
        let hovered = selection_style(iced::widget::button::Status::Hovered);
        let idle = selection_style(iced::widget::button::Status::Active);

        assert_eq!(idle.background, None, "素の行は地のまま");
        let lift = |style: &iced::widget::button::Style| match style.background {
            Some(Background::Color(color)) => color.r + color.g + color.b,
            _ => 0.0,
        };
        // 囲みは `bundle_box` が RAISED と ACTIVE の半ばまで使うので、ホバーはその上に置く。
        assert!(lift(&hovered) > surface_raised().r + surface_raised().g + surface_raised().b);
        assert_eq!(hovered.border.width, 0.0);
        assert_eq!(idle.border.width, 0.0);
    }

    /// ランチャで選んでいるモデル・思考量も面を持たない(字の明るさだけ)。
    #[test]
    fn the_launcher_marks_the_choice_with_ink_not_a_chip() {
        let chosen = launcher_option_style(iced::widget::button::Status::Active);
        assert_eq!(chosen.background, None);
    }

}
