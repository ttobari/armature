//! The terminal's tabs, one line each: what the Claude in it is doing (working, waiting for
//! you, read), its model, effort and how full its context is, and the subagents it runs under
//! it. Click a line to bring the tab to the front; drag lines to reorder the tabs.
//!
//! Armature tells the panel which tabs there are ([`Notice::Tabs`], cheap and often). The details
//! come from `claude agents` and `ps` ([`fetch_sessions`], heavy), which the panel asks for every
//! 15 seconds and when the tab in front changes — never two at once.

mod drag;
mod into_view;
mod session_tree;

use std::time::Duration;

use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, stack, text};
use iced::{Alignment, Background, Border, Length, Subscription, Task};

use crate::palette::{
    self, accent_active, accent_attention, accent_focus, surface_raised, text_faint, text_muted,
    text_primary,
};
use crate::{Element, Host, Notice, Panel, Tab, ellipsized, font, glow, tr};

/// How often the details are fetched again.
const REFRESH: Duration = Duration::from_secs(15);

/// The session list (`sessions` in `panels.conf`).
pub struct Sessions {
    rows: Vec<SessionRow>,
    /// The latest tabs Armature told of.
    tabs: Vec<Tab>,
    /// Tabs whose output the user has seen ([`track_seen`]).
    seen: std::collections::HashSet<String>,
    error: Option<String>,
    /// A fetch is running; one asked for meanwhile runs when it lands.
    in_flight: bool,
    stale: bool,
    /// The line under the pointer: where a dragged line would go.
    hovered: Option<String>,
    /// The line being dragged.
    dragged: Option<String>,
    /// The line last scrolled into view (only a change of the front tab scrolls).
    followed: Option<String>,
    scroll_id: iced::widget::Id,
    active_id: iced::widget::Id,
    phase: u8,
    /// Whether the browser is in the center (then no tab line shows as in front).
    browsing: bool,
}

#[derive(Clone, Debug)]
pub enum Message {
    Refresh,
    Loaded(Result<Vec<SessionRow>, String>),
    Phase,
    Pressed(String),
    Hovered(String),
    Unhovered(String),
    Left,
    Grabbed(String),
    Dropped,
}

impl Sessions {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            tabs: Vec::new(),
            seen: std::collections::HashSet::new(),
            error: None,
            in_flight: false,
            stale: false,
            hovered: None,
            dragged: None,
            followed: None,
            scroll_id: iced::widget::Id::unique(),
            active_id: iced::widget::Id::unique(),
            phase: 0,
            browsing: false,
        }
    }

    /// Fetches the details, one at a time.
    fn request(&mut self) -> Task<Message> {
        if self.in_flight {
            self.stale = true;
            return Task::none();
        }
        self.in_flight = true;
        let tabs = self.tabs.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || fetch_sessions(tabs))
                    .await
                    .map_err(|error| format!("session loader thread ended: {error}"))?
            },
            Message::Loaded,
        )
    }
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Panel for Sessions {
    const KEY: &'static str = "sessions";
    const SIDE: crate::Side = crate::Side::Left;
    const PADDED: bool = false;
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        self.list()
    }

    fn update(&mut self, message: Message, host: &mut Host) -> Task<Message> {
        match message {
            Message::Refresh => self.request(),
            Message::Loaded(result) => {
                match result {
                    Ok(mut rows) if !rows.is_empty() => {
                        // The fetch started before the latest tabs: lay them over it, so an old
                        // "in front" never shows for a frame.
                        reconcile_tabs(&mut rows, &self.tabs);
                        self.rows = rows;
                        self.error = None;
                        track_seen(&mut self.seen, &self.rows);
                    }
                    Ok(_) => {}
                    Err(error) => self.error = Some(error),
                }
                self.in_flight = false;
                if std::mem::take(&mut self.stale) {
                    return self.request();
                }
                Task::none()
            }
            Message::Phase => {
                self.phase = self.phase.wrapping_add(1);
                Task::none()
            }
            Message::Pressed(target) => {
                // A drag let go outside the window may never report its drop; a click means
                // nothing is held.
                self.dragged = None;
                host.select_tab(target);
                Task::none()
            }
            Message::Hovered(target) => {
                self.hovered = Some(target);
                Task::none()
            }
            Message::Unhovered(target) => {
                // **While dragging, leaving a line keeps it as the drop target** until the next
                // line is entered: the gaps between lines would make the marker flicker, and a
                // drop in a gap would do nothing.
                if self.dragged.is_none() && self.hovered.as_deref() == Some(target.as_str()) {
                    self.hovered = None;
                }
                Task::none()
            }
            Message::Left => {
                // Leaving the list lets go; dropping outside cancels.
                self.hovered = None;
                Task::none()
            }
            Message::Grabbed(target) => {
                self.dragged = Some(target);
                Task::none()
            }
            Message::Dropped => {
                let slots = self.drag_slots();
                self.dragged = None;
                let Some((from, to)) = slots else {
                    return Task::none();
                };
                let row = self.rows.remove(from);
                self.rows.insert(to, row);
                let order = self.renumber_sessions();
                host.move_tabs(order);
                Task::none()
            }
        }
    }

    fn notice(&mut self, notice: &Notice, _host: &mut Host) -> Task<Message> {
        match notice {
            Notice::Tabs(tabs) => {
                let front = |tabs: &[Tab]| tabs.iter().find(|tab| tab.active).map(|tab| tab.target.clone());
                let moved = front(tabs) != front(&self.tabs) || tabs.len() != self.tabs.len();
                self.tabs.clone_from(tabs);
                reconcile_tabs(&mut self.rows, tabs);
                track_seen(&mut self.seen, &self.rows);
                if moved {
                    let follow = self.follow();
                    Task::batch([follow, self.request()])
                } else {
                    Task::none()
                }
            }
            Notice::BrowserTabs { showing, .. } => {
                self.browsing = *showing;
                Task::none()
            }
            _ => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(REFRESH).map(|_| Message::Refresh),
            // The spinner's step: 125 ms, slower while a page is being read.
            iced::time::every(crate::pacer::ornament_interval()).map(|_| Message::Phase),
        ])
    }
}

impl Sessions {
    /// 掴んでいるパネルと、いま乗っているパネルの位置(一覧の並びでの添字)。
    fn drag_slots(&self) -> Option<(usize, usize)> {
        let source = self.dragged.as_deref()?;
        let dest = self.hovered.as_deref()?;
        if source == dest {
            return None;
        }
        let from = self.rows.iter().position(|row| row.target == source)?;
        let to = self.rows.iter().position(|row| row.target == dest)?;
        Some((from, to))
    }

    /// 並べ替えた並びを tmux へ渡す形で返し、**手元の宛先も先に付け替える**。
    ///
    /// 窓順を動かすと窓番号はパネルの間で入れ替わる(番号の集合はそのまま)。名簿の
    /// 着弾を待って付け替えると、落とした行が最大2秒だけ元の位置へ戻って見える。
    /// 宛先を鍵にして覚えているもの(既読・ホバー・追従先)も一緒に付け替える
    /// ——置いていくと、既読の印が隣のパネルへ移る。
    fn renumber_sessions(&mut self) -> Vec<String> {
        let order: Vec<String> = self
            .rows
            .iter()
            .map(|row| row.target.clone())
            .filter(|target| own_tab(target))
            .collect();
        let mut slots = order.clone();
        slots.sort_by_key(|target| tab_index(target));
        let mut renamed = std::collections::HashMap::new();
        for (row, slot) in self
            .rows
            .iter_mut()
            .filter(|row| own_tab(&row.target))
            .zip(slots)
        {
            if row.target != slot {
                renamed.insert(row.target.clone(), slot.clone());
            }
            row.target = slot;
        }
        let renamed_target = |target: &String| renamed.get(target).unwrap_or(target).clone();
        self.seen = self.seen.iter().map(renamed_target).collect();
        self.hovered = self.hovered.as_ref().map(renamed_target);
        self.followed = self.followed.as_ref().map(renamed_target);
        order
    }

    /// セッション一覧のパネル。窓のタブ1枚につき1行、その下に動いている配下。
    fn list(&self) -> Element<'_, Message> {
        // 桝は一覧全体で1組——行ごとに測ると列がずれる。
        let columns = AttrColumns::of(&self.rows);
        // 掴めるのはパネルが2つ以上あるときだけ。1つしかない一覧で指が滑っても
        // 何も起きない。
        let draggable = self.rows.len() > 1;
        let drop_at = self
            .drag_slots()
            .map(|(from, to)| (self.rows[to].target.as_str(), from > to));
        let mut list = column![].spacing(6);
        if self.rows.is_empty() {
            // 名簿を待っている間は空けておく。取れなかったときだけ、そう書く。
            if let Some(error) = self.error.as_deref() {
                list = list.push(
                    container(text(error).size(11).font(font::UI).color(text_faint()))
                        .padding([5, 2])
                        .width(Length::Fill),
                );
            }
        } else {
            // 行と行の隙間は 0。**枝の縦線がそこで切れる**からで、縦の余裕は
            // 行そのものの詰め物([`session_tree::BUTTON_PAD_Y`])へ移してある。
            let mut body = column![].spacing(0);
            for session in &self.rows {
                // 落とし先の線は、掴んだ行が入る隙間に1本だけ出す。上へ運ぶなら
                // 乗っている行の上、下へ運ぶなら下——落ちる場所と線が食い違うと、
                // 一段ずれた並びを掴まされる。
                let marker = drop_at
                    .filter(|(target, _)| *target == session.target.as_str())
                    .map(|(_, above)| above);
                // ブラウザモード中は端末セッションを選択状態として表示しない。
                let row = session_view(
                    session,
                    !self.browsing && session.active,
                    self.seen.contains(&session.target),
                    self.phase,
                    columns,
                    draggable,
                );
                // **線は行の上に重ねる。**行と行の間へ差し込むと、そこから下が
                // 線のぶん下がり、ずれた先の行がポインタの下に来て、また線が動く
                // ——押さえた指の下で一覧が震える。重ねれば寸法は1画素も動かない。
                let row = match marker {
                    Some(above) => stack![row, drop_marker(above)].into(),
                    None => row,
                };
                // 前面の行にだけ名札を付ける。隠れているときに窓へ戻す送りは、
                // ここを実際のレイアウトから測って決める(`into_view`)。
                body = body.push(if session.active {
                    container(row)
                        .id(self.active_id.clone())
                        .width(Length::Fill)
                        .into()
                } else {
                    row
                });
            }
            list = list.push(
                container(body)
                    .padding(SESSION_GROUP_PAD)
                    .width(Length::Fill),
            );
        }

        // 一覧が max_height を超えると送りが要るが、**棒は出さない**。
        // 送りは効かせたまま棒だけ隠す。
        let body = column![
            scrollable(list)
                .id(self.scroll_id.clone())
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::hidden(),
                ))
                .height(Length::Shrink)
        ]
            .spacing(5)
            .width(Length::Fill);
        // 一覧の外へ出たことはパネルごとの `on_exit` では分からない(隙も一覧の内)。
        // 落とし先を手放す口は、カードそのものに1つだけ持たせる。
        mouse_area(
            container(body)
                .padding(SESSION_CARD_PAD)
                .width(Length::Fill)
                .height(Length::Shrink)
                .max_height(620)
                .style(|_| palette::card_style()),
        )
        .on_exit(Message::Left)
        .into()
    }
    /// 前面のセッションが一覧の窓から出ていたら、見えるところまで戻す。
    ///
    /// ⌃Tab / ⌃⇧Tab で隠れているパネルへ移ると、どこにいるのか分からなくなる。
    /// **前面が変わった
    /// ときだけ**送る——名簿は2秒ごとに着弾するので、そのつど送り直すと利用者が
    /// 転がした位置を毎回奪ってしまう。行の高さは群の見出しや配下でまちまちな
    /// ので、位置は実レイアウトから測る([`into_view::scroll_into_view`])。
    fn follow(&mut self) -> Task<Message> {
        let active = self
            .rows
            .iter()
            .find(|row| row.active)
            .map(|row| row.target.clone());
        if active.is_none() || active == self.followed {
            return Task::none();
        }
        self.followed = active;
        iced::advanced::widget::operate(into_view::scroll_into_view(
            self.scroll_id.clone(),
            self.active_id.clone(),
        ))
    }
}

/// One line of the list: a tab, and what is known about the Claude in it.
#[derive(Clone, Debug)]
pub struct SessionRow {
    /// tmux の切替宛先("d:3" 等)。タブとの対応づけの鍵。Cockpit のタブでない行は空。
    target: String,
    title: String,
    status: SessionStatus,
    model: Option<String>,
    effort: Option<String>,
    context_used: Option<f64>,
    agents: Vec<AgentRow>,
    active: bool,
    /// Claude が返事を出し切って、利用者の入力で止まっている。
    ///
    /// 出どころはペインの名乗りの頭に Claude が書く `✳`——「返事が出揃った」の印
    /// (`armature_core::monitor::strip_title_mark` が飾りとして剥がしている字)。回転中の
    /// 点字・半月が working を意味するのと同じ経路なので、質問で止まっていても・
    /// 完了して指示待ちでも、手が動いていない限りこの印になる。
    awaiting: bool,
    /// このパネルの向こうに Claude が居るか。
    ///
    /// **素のシェルと分ける唯一の手がかり。** 緑は「Claude が手を動かしている」の
    /// 印なので、シェルに点けると意味を失う。判定の材料は4つ——会話ID・モデル・
    /// 動いている配下・名乗りに Claude が書いた状態マーク。どれか1つでも在れば
    /// Claude のパネルで、1つも無ければシェルとして扱う。
    claude: bool,
}

#[derive(Clone, Debug)]
struct AgentRow {
    name: String,
    model: Option<String>,
    effort: Option<String>,
    context_used: Option<f64>,
    /// 最後にログへ書き込んでからの経過(秒)。
    ///
    /// **子はプロセスではないので `ps` に出ない。** 生死を測る材料はこの1つだけで、
    /// 止められた木も考え込んでいる木も同じ「黙っている木」に見える
    /// (`armature_core::monitor` の窓は最長15分)。どちらか判じるのは利用者なので、
    /// 判定を勝手に固めず**黙っている長さをそのまま行に出す**。
    idle_sec: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SessionStatus {
    Working,
    Unread,
    Read,
}



/// セッション一覧の card の詰め物(縦・横)。
const SESSION_CARD_PAD: [f32; 2] = [7.0, 9.0];
/// 群の中身(行の外側)の詰め物。
const SESSION_GROUP_PAD: [f32; 2] = [4.0, 2.0];

/// パネルの行の要素間。配下の行より広い(木の枝ぶん字が詰まって見えるのを避ける)。
const SESSION_ROW_SPACING: f32 = 7.0;

/// 配下(サブエージェント)の行の要素間。
const AGENT_ROW_SPACING: f32 = 5.0;

/// 枝の色。**生死の色を借りない**。
///
/// 枝は「どれがどれの子か」を示す地図で、読ませたいのは題のほう。緑や黄で塗ると
/// 一覧の中でいちばん強い色が枝になり、目が題より先に線を追う。地の文より一段
/// 落とした低コントラストの罫線にして、印と題だけが意味色を持つようにする。
fn session_tree_ink() -> iced::Color {
    palette::with_alpha(text_faint(), 0.45)
}
/// 属性の右端から行の右端まで。
///
/// パネルと配下で `spacing` が違うので、行末の余白を素の 6px で揃えると属性の右端が
/// 2px ずれる(`spacing` は最後の要素との間にも入るため)。ここで差を吸収して、
/// パネルの行と配下の行の属性を同じ桁へ落とす。
const SESSION_ROW_TAIL: f32 = 13.0;

fn session_row_tail(spacing: f32) -> Length {
    Length::Fixed(SESSION_ROW_TAIL - spacing)
}

/// この窓のタブを指す宛先(`<slug>:<窓番号>`、素の Armature なら `armature:3`)か。名簿由来の
/// 行——外で走っている claude——はここに当たらないので、選択も並べ替えも効かない。
fn own_tab(target: &str) -> bool {
    tab_suffix(target).is_some()
}

fn tab_suffix(target: &str) -> Option<&str> {
    target
        .strip_prefix(armature_core::paths::slug())?
        .strip_prefix(':')
}

fn tab_index(target: &str) -> Option<u32> {
    tab_suffix(target)?.parse().ok()
}

/// 掴んだ行が落ちる隙間に引く線。行の**上に重ねる**ので、寸法は取らない
/// (`above` は行の上端に引くか下端に引くか)。
///
/// 色は [`accent_focus`]——「いま入力を受けている場所」。稼働の緑・対応待ちの黄は
/// 行そのものの意味を持っているので、並べ替えの都合で借りない。
fn drop_marker(above: bool) -> Element<'static, Message> {
    container(
        container(Space::new().width(Length::Fill).height(Length::Fixed(2.0))).style(|_| {
            container::Style {
                background: Some(Background::Color(accent_focus())),
                border: Border {
                    radius: 1.0.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        }),
    )
    .align_y(if above {
        iced::alignment::Vertical::Top
    } else {
        iced::alignment::Vertical::Bottom
    })
    .padding([0, 4])
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 生死の印(`⠋` / `●` / `○`)。**桝の中で中央に寄せる**。
///
/// 左詰めのままだと、前送りの狭い字(明滅の `⠋`)だけが桝の左へ寄り、丸(`○`/`●`)
/// と桁が 1.25px ずれる——行ごとに印が横へ跳ね、枝の縦線ともずれる。桝はパネルと配下で同じ。
fn session_mark<'a>(mark: &'a str, color: iced::Color) -> iced::widget::Text<'a> {
    text(mark)
        .size(11)
        .color(color)
        .width(Length::Fixed(session_tree::MARK_BOX))
        .align_x(iced::widget::text::Alignment::Center)
}

/// 行に重ねる枝。配下を持たないパネルは線を持たない——一覧のほとんどはこれで、
/// 線が出ないぶん木が読める。
fn session_guide(session: &SessionRow) -> Option<session_tree::Tree> {
    (!session.agents.is_empty()).then(|| session_tree::Tree::trunk(session_tree_ink()))
}

fn session_view(
    session: &SessionRow,
    active: bool,
    seen: bool,
    animation_phase: u8,
    columns: AttrColumns,
    draggable: bool,
) -> Element<'_, Message> {
    let liveness = liveness(session);
    let (mark, color) = match liveness {
        Liveness::Working => (spinner_mark(animation_phase), accent_active()),
        Liveness::Unread => ("●", accent_attention()),
        Liveness::Read => ("○", text_faint()),
    };
    let metadata = agent_metadata(
        session.model.as_deref(),
        session.effort.as_deref(),
        session.context_used,
        columns,
    );
    // 前面のタブの行は面で塗らない。題を太字にして光らせる(タスク一覧の選んでいる行と同じ)。
    let title_ink = session_title_color(session, liveness, seen);
    let title: Element<'_, Message> = if active {
        glow::glow(
            move |ink| ellipsized::styled(&session.title, 13.0, ink, font::UI_STRONG),
            title_ink,
            SESSION_TITLE_GLOW,
        )
    } else {
        ellipsized::styled(&session.title, 13.0, title_ink, font::UI)
    };
    let head = row![
        session_mark(mark, color),
        container(title).width(Length::Fill).clip(true),
        container(metadata).width(Length::Shrink),
        Space::new().width(session_row_tail(SESSION_ROW_SPACING)),
    ]
    .spacing(SESSION_ROW_SPACING)
    .align_y(Alignment::Center);
    // 行の中に隙間を作らない。配下の行の縦線が隙間で切れる——縦の余裕は
    // 行の詰め物([`session_tree::BUTTON_PAD_Y`])と書体の行送りで足りている。
    let mut content = column![head].spacing(0);
    for (index, agent) in session.agents.iter().enumerate() {
        let last = index + 1 == session.agents.len();
        let branch_row = row![
            Space::new().width(Length::Fixed(session_tree::AGENT_INDENT)),
            session_mark(
                agent_mark(agent.idle_sec, animation_phase),
                agent_mark_color(agent.idle_sec)
            ),
            container(ellipsized::text(
                &agent.name,
                12.0,
                agent_title_color(agent.idle_sec)
            ))
            .width(Length::Fill)
            .clip(true),
            container(agent_metadata(
                agent.model.as_deref(),
                agent.effort.as_deref(),
                agent.context_used,
                columns,
            ))
            .width(Length::Shrink),
            Space::new().width(session_row_tail(AGENT_ROW_SPACING)),
        ]
        .spacing(AGENT_ROW_SPACING)
        // 丈を決め打つ。枝は行の高さいっぱいに引く絵なので、行が縮む側
        // (`Shrink`)だと線の丈が字の高さに引きずられて上下の行と繋がらない。
        .height(Length::Fixed(session_tree::AGENT_ROW))
        .align_y(Alignment::Center);
        // 配下の枝も行の上に重ねる。**canvas で重ねない**——行ごとに mesh の層が立ち、
        // 5K の面を行数×3 回なで直して打鍵が遅れた(`session_tree` の頭注)。
        content = content.push(stack![
            branch_row,
            session_tree::Tree::agent(session_tree_ink(), last)
        ]);
    }
    let target = session.target.clone();
    let clickable = own_tab(&target);
    let mut item = button(content)
        .padding([session_tree::BUTTON_PAD_Y, session_tree::BUTTON_PAD_X])
        .width(Length::Fill);
    if clickable {
        item = item.on_press(Message::Pressed(target.clone()));
    }
    let item = item.style(move |_, status| session_button_style(clickable, status));
    // 枝は**行の上に重ねる**。行の中に置くと詰め物のぶん上下が余り、隣の行の線と
    // 繋がらない(`drop_marker` と同じ手で、寸法は1画素も動かさない)。枝の Widget は
    // `update` を実装していないので、重ねても押し心地は変わらない。
    let item: Element<'_, Message> = match session_guide(session) {
        Some(guide) => stack![item, guide].into(),
        None => item.into(),
    };
    if !clickable {
        return item;
    }
    // ホバーは落とし先の印。素のシェルのパネルにも付ける——外すと「そこには
    // 落とせないパネル」が一覧に混ざる。
    let area = mouse_area(item)
        .on_enter(Message::Hovered(target.clone()))
        .on_exit(Message::Unhovered(target.clone()));
    if !draggable {
        return area.into();
    }
    // 掴んで動かすと窓順そのものが入れ替わる(⌃Tab の巡回順もこれに従う)。
    // 押しただけで動かさなければ、下の button の選択がそのまま通る。
    drag::drag(area, Message::Grabbed(target), Message::Dropped)
}

/// 使用量の欄の字。桝を測る側と描く側で書式がずれないよう1箇所に置く。
fn context_label(used: f64) -> String {
    format!("{used:.0}%")
}

/// 属性の字送り。既定書体(Moralerspace Argon)は等幅で、`size(12)` のとき
/// ASCII は全字ちょうど 7.2px(ttf の `hmtx`/`head` を実測)。桁数×この値で桝が出る。
const METADATA_ADVANCE: f32 = 7.2;
const METADATA_SIZE: f32 = 12.0;

/// 一覧の属性(モデル・effort・使用量)の桝。
///
/// 行ごとの実寸で詰めると、字数が違うだけで右隣の列がずれる——`opus`/`fable`、
/// `high`/`xhigh`、`7%`/`27%` の差がそのまま後続をずらす。一覧全体を1度舐めて桝を
/// 決め、**値を持たない行も桝ぶんの幅を取る**——そこだけ詰めると右隣がずれて
/// 元の木阿弥。桝はパネルと配下と群をまたいで1組(群ごとに測ると境目でずれる)。
///
/// 寄せは**3列とも右(けつ)揃え**。寄せの指定は [`agent_metadata`] の `ALIGN` 1点に
/// 集めてある。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AttrColumns {
    model: usize,
    effort: usize,
    context: usize,
}

impl AttrColumns {
    /// 一覧(パネルとその配下)を舐めて桝を決める。
    fn of(sessions: &[SessionRow]) -> Self {
        let mut columns = Self::default();
        for session in sessions {
            columns.put(
                session.model.as_deref(),
                session.effort.as_deref(),
                session.context_used,
            );
            for agent in &session.agents {
                columns.put(
                    agent.model.as_deref(),
                    agent.effort.as_deref(),
                    agent.context_used,
                );
            }
        }
        columns
    }

    /// モデル名・effort・使用量はいずれもASCIIなので、字数がそのまま桁数になる。
    fn put(&mut self, model: Option<&str>, effort: Option<&str>, context_used: Option<f64>) {
        let cells = |value: Option<&str>| value.map_or(0, |value| value.chars().count());
        self.model = self.model.max(cells(model));
        self.effort = self.effort.max(cells(effort));
        self.context = self
            .context
            .max(context_used.map_or(0, |used| context_label(used).chars().count()));
    }
}

/// 桁数を実寸へ。
fn metadata_width(cells: usize) -> Length {
    Length::Fixed(cells as f32 * METADATA_ADVANCE)
}

fn agent_metadata<'a>(
    model: Option<&'a str>,
    effort: Option<&'a str>,
    context_used: Option<f64>,
    columns: AttrColumns,
) -> iced::widget::Row<'a, Message> {
    use iced::widget::text::{Alignment as TextAlignment, Wrapping};

    let cell = |content: String, cells: usize, color: iced::Color, align: TextAlignment| {
        text(content)
            .size(METADATA_SIZE)
            .color(color)
            .width(metadata_width(cells))
            .align_x(align)
            // 桝はちょうど字幅なので、丸めで1字ぶん溢れて折り返すのを禁じる。
            .wrapping(Wrapping::None)
    };

    // **モデルと effort の寄せはここ1点。** で右詰めにした。試しの温度感なので、
    // 戻すときはこの1行を `Left` へ倒すだけでよい(使用量は数なので常に右詰め)。
    const ALIGN: TextAlignment = TextAlignment::Right;

    let mut metadata = row![].spacing(6).align_y(Alignment::Center);
    // 桝が立っている列は、この行が値を持たなくても幅だけ取る。
    if columns.model > 0 {
        let name = model.unwrap_or_default();
        metadata = metadata.push(cell(
            name.to_string(),
            columns.model,
            palette::model(name),
            ALIGN,
        ));
    }
    if columns.effort > 0 {
        let name = effort.unwrap_or_default();
        metadata = metadata.push(cell(
            name.to_string(),
            columns.effort,
            palette::effort(name),
            ALIGN,
        ));
    }
    if columns.context > 0 {
        let label = context_used.map(context_label).unwrap_or_default();
        metadata = metadata.push(cell(
            label,
            columns.context,
            text_muted(),
            TextAlignment::Right,
        ));
    }
    metadata
}

/// パネルの見かけの状態。**緑を出してよいかまで含めて、ここで1度だけ決める。**
///
/// 生の [`SessionStatus`] と分けているのは、素のシェルのため。緑は「Claude が
/// 手を動かしている」の印なので、Claude の居ないパネルに点けると何も意味しない色に
/// なる。
///
/// **配下だけが動いているときは、親を止まっているものとして描く。** 一度は
/// 親にも沈めた緑の印を回したが、で取り下げた——「メインエージェントは
/// もうこのくるくるマークいらない。サブエージェントだけ緑でピコピコ動いて
/// いればいい」(2026-08-20)。木が生きていることは枝の行が自分で示す。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Liveness {
    /// Claude 本人の手が動いている。
    Working,
    /// 止まっていて、まだ見ていない。
    Unread,
    /// 止まっていて、見た。
    Read,
}

fn liveness(session: &SessionRow) -> Liveness {
    match session.status {
        // シェルのパネルに緑は点けない。出力が届いていたことだけを未読で残す。
        SessionStatus::Working if !session.claude => {
            if session.active {
                Liveness::Read
            } else {
                Liveness::Unread
            }
        }
        SessionStatus::Working => Liveness::Working,
        SessionStatus::Unread => Liveness::Unread,
        SessionStatus::Read => Liveness::Read,
    }
}

fn spinner_mark(phase: u8) -> &'static str {
    const FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
    FRAMES[usize::from(phase) % FRAMES.len()]
}

/// 黙っている木を「幽霊」と断じない下限。
///
/// 手が動いている木でも、道具の戻り待ち(Bash・WebFetch)で数十秒はふつうに空く。
/// ここを狭くすると走っている木にまで無音の札が点滅する飾りになる。
const AGENT_QUIET: f64 = 60.0;

/// 配下の行の印。
///
/// **道具の有無では決めない**——8体を並列で走らせている最中に全行が止まって
/// 見えた(`armature_core::monitor::Agent::live` の実測2026-08-13)。回すのをやめるのは
/// 名簿側の窓(`AGENT_BUSY_WINDOW` = 5分)を越えて黙っている木だけで、そこまで来た
/// 行は「考え込んでいる木」としても長すぎる。
fn agent_mark(idle_sec: f64, phase: u8) -> &'static str {
    if idle_sec > armature_core::monitor::AGENT_BUSY_WINDOW {
        "◦"
    } else {
        spinner_mark(phase)
    }
}

/// 配下の印の色。**3段**——動いている緑 / 黙っているグレー / 窓を越えた薄いグレー。
fn agent_mark_color(idle_sec: f64) -> iced::Color {
    if idle_sec > armature_core::monitor::AGENT_BUSY_WINDOW {
        text_faint()
    } else if agent_is_quiet(idle_sec) {
        text_muted()
    } else {
        accent_active()
    }
}

/// 黙っているかどうか。[`AGENT_QUIET`] を越えたら真。
///
/// 「動いているか」を当てにいかず、**測れた事実だけ**を出す
/// ——止められた木と考え込んでいる木は外から見分けられない。
///
/// 出し方は札から色へ移した。**「1分無音」の字は行の幅を
/// 食い、しかも数えるほどの情報ではない**——黙っているかどうかだけ分かればよい。
fn agent_is_quiet(idle_sec: f64) -> bool {
    !idle_sec.is_nan() && idle_sec >= AGENT_QUIET
}

/// 一覧の題の色。
///
/// 動いているパネルは緑。止まっているパネルは利用者が見たかどうかで分ける
/// ——見たパネルはグレーへ落とし、まだ見ていない「対応待ち」だけを黄で立たせる。
/// 既読を対応待ちより先に見るのは、`✳` が利用者の入力まで消えない印で、優先を逆に
/// すると見終えたパネルまで黄のまま居座るため。
fn session_title_color(session: &SessionRow, liveness: Liveness, seen: bool) -> iced::Color {
    if liveness == Liveness::Working {
        accent_active()
    } else if seen {
        text_faint()
    } else if session.awaiting {
        accent_attention()
    } else {
        text_primary()
    }
}

/// 木の枝(サブエージェント)の名の色。
///
/// **枝は常に「実行中」。** 一覧に枝が出るのは稼働中の配下だけ
/// ([`armature_core::monitor::live_agents`])。既読・対応待ちは tmux のペインを持つ親の
/// 性質で、ペインを持たない枝には立たない。左端の印と同じ [`accent_active`] を使う。
/// 配下の行の題の色。黙って久しい行は落とす——印だけだと、束の中で
/// どれが生きているのか一目で拾えない。
fn agent_title_color(idle_sec: f64) -> iced::Color {
    if idle_sec > armature_core::monitor::AGENT_BUSY_WINDOW || agent_is_quiet(idle_sec) {
        text_muted()
    } else {
        accent_active()
    }
}

/// 一覧の行。面を持つのは指を乗せている行だけ——前面のタブは題が光る([`session_view`])。
fn session_button_style(clickable: bool, status: button::Status) -> button::Style {
    let hovered = clickable && matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if hovered {
        Some(Background::Color(surface_raised()))
    } else {
        None
    };
    button::Style {
        background,
        text_color: text_primary(),
        border: Border {
            radius: palette::radius_control().into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}


/// セッション一覧の前面の行の題の滲み。
const SESSION_TITLE_GLOW: f32 = 0.6;


/// タブ列挙で分かる範囲(行の増減と前面)を既存の行へ写す。
/// 消えたタブの行は落とし、現れたタブは素の行(モデル等は空)で足す。
/// 重い情報は後追いの名簿が埋める。列挙は定コスト(約20ms)なのでキー経路に置ける。
fn reconcile_tabs(rows: &mut Vec<SessionRow>, tabs: &[Tab]) {
    rows.retain(|row| row.target.is_empty() || tabs.iter().any(|tab| tab.target == row.target));
    for tab in tabs {
        if !rows.iter().any(|row| row.target == tab.target) {
            rows.push(bare_row(tab.clone()));
        }
    }
    apply_tab_activity(rows, tabs);
    apply_awaiting(rows, tabs);
}

/// タブ列挙だけから作る素の行。
fn bare_row(tab: Tab) -> SessionRow {
    let awaiting = awaiting_reply(&tab.title);
    let title_for_mark = tab.title.clone();
    SessionRow {
        target: tab.target,
        title: if tab.title.is_empty() {
            "Claude Code".into()
        } else {
            tab.title
        },
        status: if tab.busy {
            SessionStatus::Working
        } else if tab.unread {
            SessionStatus::Unread
        } else {
            SessionStatus::Read
        },
        model: None,
        effort: None,
        context_used: None,
        agents: Vec::new(),
        active: tab.active,
        awaiting,
        // タブ列挙だけでは会話の有無が分からない。名乗りのマークがあれば
        // Claude のパネルで、無ければ判じない(名簿が届いた時点で上書きされる)。
        claude: awaiting || armature_core::monitor::title_state(&title_for_mark).is_some(),
    }
}

/// タブ列挙で分かる範囲(どのパネルが前面か)を既存の行へ写す。
///
/// 行には名簿由来のセッション(Cockpit の外で動く claude)も混ざるため、
/// 順番ではなく tmux の切替宛先(`target`)で対応づける。前面になれるのは
/// Cockpit のタブだけなので、対応するタブが無い行は前面ではない。
fn apply_tab_activity(rows: &mut [SessionRow], tabs: &[Tab]) {
    for row in rows.iter_mut() {
        row.active = !row.target.is_empty()
            && tabs
                .iter()
                .any(|tab| tab.active && tab.target == row.target);
    }
}

/// 名乗りの頭の印が「返事が出揃った」(`✳`)かどうか。
///
/// 回転中の点字(U+2800–U+28FF)と半月(U+25D0–U+25D3)は**手が動いている**印なので
/// ここでは拾わない。読むのは装飾記号のブロック(U+2700–U+27BF)——`✳` = U+2733 が
/// そこに居る。`strip_title_mark` が題から剥がしている範囲と同じにしてあるので、
/// Claude が印の字を替えても付いていける。
fn awaiting_reply(pane_title: &str) -> bool {
    pane_title
        .chars()
        .find(|c| !c.is_whitespace())
        .is_some_and(|c| ('\u{2700}'..='\u{27BF}').contains(&c))
}

/// 生のペイン名から「対応待ち」の印だけを既存の行へ写す。
///
/// 名簿経由の行は題が [`armature_core::monitor::strip_title_mark`] を通っていて印が
/// 落ちているため、印はタブ列挙の生の名乗りから採り直す。対応づけは順番ではなく
/// tmux の切替宛先(`target`)——[`apply_tab_activity`] と同じ理由。
fn apply_awaiting(rows: &mut [SessionRow], tabs: &[Tab]) {
    for row in rows.iter_mut() {
        if row.target.is_empty() {
            continue;
        }
        if let Some(tab) = tabs.iter().find(|tab| tab.target == row.target) {
            row.awaiting = awaiting_reply(&tab.title);
            // 名乗りに Claude の状態マークが在るなら、名簿を待たずに Claude のパネル。
            row.claude =
                row.claude || row.awaiting || armature_core::monitor::title_state(&tab.title).is_some();
        }
    }
}

/// 利用者が「見た」パネルを覚える。
///
/// 前面に出ていて手が止まっているパネルは、利用者がいまその出力を見ているので既読。
/// 逆に動き出したパネルは未確認へ戻す——次に止まったときは改めて対応が要るため。
/// 消えたタブは覚えたままにしない。
fn track_seen(seen: &mut std::collections::HashSet<String>, rows: &[SessionRow]) {
    for row in rows {
        if row.target.is_empty() {
            continue;
        }
        if row.status == SessionStatus::Working {
            seen.remove(&row.target);
        } else if row.active {
            seen.insert(row.target.clone());
        }
    }
    seen.retain(|target| rows.iter().any(|row| &row.target == target));
}

/// 名簿(`claude agents`)と ps で、タブの行に中身(モデル・思考量・文脈・配下・稼働)を足す。
/// **重い**(1回 0.6〜1.0 秒・Node のプロセス)ので裏の筋で回す。名簿は付加情報であって、
/// 取れなくてもタブそのものは消さない。
fn fetch_sessions(mut tabs: Vec<Tab>) -> Result<Vec<SessionRow>, String> {
    let running = armature_core::monitor::fetch_running().unwrap_or_default();
    if let Some(ps) = armature_core::monitor::ps_snapshot() {
        for tab in &mut tabs {
            tab.busy = running.iter().any(|session| {
                session.busy && ps.ttys.get(&session.pid).is_some_and(|tty| tty == &tab.tty)
            });
        }
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64());
    let sessions = armature_core::monitor::build_sessions(&running, &tabs, now)
        .ok_or_else(|| {
            tr!("Couldn't read the Claude Code sessions", "Claude Codeのセッションを取得できない")
                .to_string()
        })?;
    let mut rows: Vec<_> = sessions
        .into_iter()
        .map(|session| {
            let status = if session.busy {
                SessionStatus::Working
            } else {
                match session.tab_state {
                    armature_core::monitor::TabState::Working => SessionStatus::Working,
                    armature_core::monitor::TabState::Unread => SessionStatus::Unread,
                    armature_core::monitor::TabState::Read => SessionStatus::Read,
                }
            };
            let agents = session
                .agents
                .into_iter()
                .map(|agent| AgentRow {
                    name: if agent.name.is_empty() {
                        "subagent".into()
                    } else {
                        agent.name
                    },
                    model: agent.model,
                    effort: agent.effort,
                    context_used: agent.context_left.map(|left| 100.0 - left),
                    idle_sec: agent.idle_sec,
                })
                .collect::<Vec<_>>();
            let agents_present = !agents.is_empty();
            // Claude のパネルかどうかは、値を行へ移す前に測っておく。
            let claude = session.session_id.is_some() || session.model.is_some() || agents_present;
            SessionRow {
                target: session.target.clone(),
                title: if session.title.is_empty() {
                    "Claude Code".into()
                } else {
                    session.title
                },
                status,
                model: session.model,
                effort: session.effort,
                context_used: session.context_left.map(|left| 100.0 - left),
                // Bash等の道具名やactivityは親行へ出さない。動いている配下だけを
                // 詳細な木として見せる。
                agents,
                active: session.active,
                // 名簿は名乗りの状態マークを剥がして渡してくるので、印そのものは
                // 生のペイン名から採り直す。
                awaiting: false,
                claude,
            }
        })
        .collect();
    apply_awaiting(&mut rows, &tabs);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 試験用の空の行。見たい欄だけ `..blank_row()` で差し替える。
    fn blank_row() -> SessionRow {
        SessionRow {
            target: "d:1".into(),
            title: "パネル".into(),
            status: SessionStatus::Read,
            model: None,
            effort: None,
            context_used: None,
            agents: Vec::new(),
            active: false,
            awaiting: false,
            claude: true,
        }
    }

    /// 名簿は取得を始めた時点のタブで組まれている。着弾したら、その後に届いたタブの一覧を
    /// 重ねる——閉じたタブを生き返らせず、まだ知らない新しいタブも落とさず、前面も戻さない。
    #[test]
    fn a_late_roster_is_laid_over_the_latest_tabs() {
        let tab = |target: &str, active| Tab {
            target: target.into(),
            active,
            ..Default::default()
        };
        let mut stale_roster = vec![bare_row(tab("a", true)), bare_row(tab("b", false))];
        stale_roster[1].model = Some("opus".into());
        reconcile_tabs(&mut stale_roster, &[tab("b", true), tab("c", false)]);
        assert_eq!(
            stale_roster.iter().map(|row| row.target.as_str()).collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert!(stale_roster[0].active);
        assert!(!stale_roster[1].active);
        assert_eq!(stale_roster[0].model.as_deref(), Some("opus"));
    }

    #[test]
    fn the_active_seat_follows_the_tab_switch_without_the_roster() {
        let tab = |target: &str, active| armature_core::monitor::LocalTab {
            target: target.into(),
            active,
            ..Default::default()
        };
        let row = |target: &str, active| SessionRow {
            claude: true,
            target: target.into(),
            title: "Claude Code".into(),
            status: SessionStatus::Read,
            model: None,
            effort: None,
            context_used: None,
            agents: Vec::new(),
            active,
            awaiting: false,
        };
        // 名簿由来の行(Cockpit外・target空)が混ざっても、宛先で正しく対応づく。
        let mut rows = vec![row("d:1", true), row("", false), row("d:2", false)];
        apply_tab_activity(&mut rows, &[tab("d:1", false), tab("d:2", true)]);
        assert!(!rows[0].active);
        assert!(!rows[1].active);
        assert!(rows[2].active);
    }

    #[test]
    fn tab_rows_come_and_go_with_the_tabs_themselves() {
        let tab = |target: &str, active| armature_core::monitor::LocalTab {
            target: target.into(),
            active,
            ..Default::default()
        };
        let mut rows = vec![bare_row(tab("d:1", true)), bare_row(tab("d:2", false))];
        // d:2 が閉じ、d:3 が開いた。
        reconcile_tabs(&mut rows, &[tab("d:1", false), tab("d:3", true)]);
        let targets: Vec<&str> = rows.iter().map(|row| row.target.as_str()).collect();
        assert_eq!(targets, ["d:1", "d:3"]);
        assert!(!rows[0].active);
        assert!(rows[1].active);
    }

    #[test]
    fn the_reply_mark_is_the_only_glyph_that_means_waiting_for_you() {
        // `✳`(U+2733)= 返事が出揃った。回っている印は手が動いている側。
        assert!(awaiting_reply("✳ 返事が出揃った"));
        assert!(awaiting_reply("  ✳ 前に空白があっても"));
        assert!(!awaiting_reply("⠋ 考え中"), "点字の回転は working");
        assert!(!awaiting_reply("◐ 考え中"), "半月の回転も working");
        assert!(
            !awaiting_reply("素の題"),
            "印の無い名乗りは対応待ちではない"
        );
        assert!(!awaiting_reply(""));
    }

    #[test]
    fn a_stopped_seat_stays_gold_until_you_look_at_it() {
        let tab = |target: &str, title: &str, busy, active| armature_core::monitor::LocalTab {
            target: target.into(),
            title: title.into(),
            busy,
            active,
            ..Default::default()
        };
        let running = |active| tab("d:1", "⠋ 走っている", true, active);
        let stopped = |active| tab("d:2", "✳ 返事が出揃った", false, active);
        let color =
            |rows: &[SessionRow], seen: &std::collections::HashSet<String>, index: usize| {
                let row = &rows[index];
                session_title_color(row, liveness(row), seen.contains(&row.target))
            };

        let mut rows = vec![bare_row(running(true)), bare_row(stopped(false))];
        let mut seen = std::collections::HashSet::new();
        track_seen(&mut seen, &rows);
        assert_eq!(color(&rows, &seen, 0), accent_active(), "実行中は緑");
        assert_eq!(color(&rows, &seen, 1), accent_attention(), "止まっていて未確認は黄");

        // 利用者が止まっているパネルへ切り替えた=見た。
        reconcile_tabs(&mut rows, &[running(false), stopped(true)]);
        track_seen(&mut seen, &rows);
        assert_eq!(color(&rows, &seen, 1), text_faint(), "既読はグレー");

        // 返事を入れて再び動き出すと既読は外れ、次に止まればまた黄へ戻る。
        rows[1].status = SessionStatus::Working;
        track_seen(&mut seen, &rows);
        rows[1].status = SessionStatus::Read;
        assert_eq!(color(&rows, &seen, 1), accent_attention());

        // 閉じたタブは覚えたままにしない。
        reconcile_tabs(&mut rows, &[running(true)]);
        track_seen(&mut seen, &rows);
        assert!(seen.is_empty());
    }

    #[test]
    fn a_running_subagent_wears_the_same_green_as_its_spinner() {
        // 枝に並ぶのは稼働中だけなので、名も左端の印と同じ緑で揃える。
        assert_eq!(agent_title_color(0.0), accent_active());
        assert_ne!(agent_title_color(0.0), text_primary());
    }

    /// 止められた木と考え込んでいる木は外から見分けられない(子はプロセスでは
    /// ないので `ps` に出ない)。判定を固めず、黙っている長さを出して利用者に渡す
    /// 。
    #[test]
    fn a_silent_subagent_turns_grey_instead_of_wearing_a_label() {
        // 道具の戻り待ちの空白では落とさない。ここを狭めると走っている木が翳る。
        assert!(!agent_is_quiet(0.0));
        assert!(!agent_is_quiet(59.0));
        assert!(!agent_is_quiet(f64::NAN));
        assert!(agent_is_quiet(60.0));
        assert!(agent_is_quiet(7200.0));
        // 黙った瞬間に緑を降りる。字は足さない。
        assert_eq!(agent_title_color(59.0), accent_active());
        assert_eq!(agent_title_color(60.0), text_muted());
        assert_eq!(agent_mark_color(59.0), accent_active());
        assert_eq!(agent_mark_color(60.0), text_muted());
    }

    /// 印を止めるのは名簿の窓(5分)を越えた行だけ。道具の有無で決めると、
    /// 8体を並列で走らせている最中に全行が止まって見える(実測2026-08-13)。
    #[test]
    fn the_spinner_keeps_turning_until_the_roster_window_runs_out() {
        let busy = armature_core::monitor::AGENT_BUSY_WINDOW;
        assert_eq!(agent_mark(0.0, 3), spinner_mark(3));
        assert_eq!(agent_mark(busy, 3), spinner_mark(3), "窓の内側は回し続ける");
        assert_eq!(agent_mark(busy + 1.0, 3), "◦");
        // 窓の内側でも1分黙っていればグレー。緑は「いま書いている」だけの色。
        assert_eq!(agent_mark_color(busy), text_muted());
        assert_eq!(agent_mark_color(busy + 1.0), text_faint());
        assert_eq!(agent_title_color(busy + 1.0), text_muted());
        assert!(agent_is_quiet(busy + 1.0));
    }

    #[test]
    fn the_working_spinner_turns_and_cycles() {
        assert_ne!(spinner_mark(0), spinner_mark(1));
        assert_eq!(spinner_mark(0), spinner_mark(8));
    }

    /// 素のシェルに緑を点けない。緑は「Claude が手を
    /// 動かしている」の印で、Claude の居ないパネルでは何も意味しない。
    #[test]
    fn a_plain_shell_never_turns_green_even_while_it_is_busy() {
        let shell = |active| SessionRow {
            claude: false,
            status: SessionStatus::Working,
            active,
            ..blank_row()
        };
        assert_eq!(
            liveness(&shell(false)),
            Liveness::Unread,
            "見ていないパネルは未読"
        );
        assert_eq!(liveness(&shell(true)), Liveness::Read, "見ているパネルは既読");
        assert_eq!(
            liveness(&SessionRow {
                claude: true,
                status: SessionStatus::Working,
                ..blank_row()
            }),
            Liveness::Working,
            "Claude のパネルはこれまでどおり緑"
        );
    }

    /// 配下だけが動いているとき、親は止まっているものとして描く。
    #[test]
    fn a_seat_whose_children_are_running_still_reads_as_stopped() {
        let agent = |idle_sec| AgentRow {
            name: "worker".into(),
            model: None,
            effort: None,
            context_used: None,
            idle_sec,
        };
        let delegated = SessionRow {
            claude: true,
            status: SessionStatus::Read,
            agents: vec![agent(1.0)],
            ..blank_row()
        };
        assert_eq!(liveness(&delegated), Liveness::Read, "親は止まっている扱い");
        // 親の題も灰のまま。動いているのは配下で、親は指示待ち。
        assert_eq!(
            session_title_color(&delegated, Liveness::Read, true),
            text_faint()
        );
        // 枝の行だけが緑を持つ。
        assert_eq!(agent_title_color(1.0), accent_active());
        assert_eq!(agent_mark_color(1.0), accent_active());
    }

    /// 属性の桝は、一覧に居るいちばん長い値で決まる。
    ///
    /// 利用者の実機は `opus`/`fable`(4字/5字)・`high`/`xhigh`(4字/5字)だが、
    /// `palette::model` は `sonnet`(6字)を、`palette::effort` は `medium`(6字)を
    /// 持ち、どちらも未知の名を `_` で通す。最長を決め打ちにすると切れるので、
    /// 一覧を舐めて決める。
    #[test]
    fn the_attribute_columns_take_the_widest_value_in_the_whole_list() {
        let session = |model: &str, effort: &str, used: f64, agents: Vec<AgentRow>| SessionRow {
            claude: true,
            target: "d:1".into(),
            title: "Claude Code".into(),
            status: SessionStatus::Read,
            model: Some(model.into()),
            effort: Some(effort.into()),
            context_used: Some(used),
            agents,
            active: false,
            awaiting: false,
        };

        let columns = AttrColumns::of(&[
            session("opus", "high", 7.0, Vec::new()),
            session("sonnet", "medium", 100.0, Vec::new()),
        ]);
        assert_eq!(columns.model, "sonnet".len());
        assert_eq!(columns.effort, "medium".len());
        assert_eq!(columns.context, "100%".len());

        // 配下(サブエージェント)の値も同じ桝に入る——親だけ測ると枝でずれる。
        let branch = AttrColumns::of(&[session(
            "opus",
            "high",
            7.0,
            vec![AgentRow {
                name: "worker".into(),
                model: Some("sonnet".into()),
                effort: Some("xhigh".into()),
                context_used: Some(100.0),
                idle_sec: 0.0,
            }],
        )]);
        assert_eq!(branch.model, "sonnet".len());
        assert_eq!(branch.effort, "xhigh".len());
        assert_eq!(branch.context, "100%".len());
    }

    /// 桝は「桁数 × 字送り」で実寸になる。最長ケースが切れないことを押さえる。
    #[test]
    fn the_widest_attribute_values_fit_in_their_column() {
        // 既定書体は等幅で、size(12) のとき ASCII は全字 7.2px。
        assert!((METADATA_ADVANCE - 7.2).abs() < f32::EPSILON);
        for longest in ["sonnet", "medium", "100%"] {
            let cells = longest.chars().count();
            assert_eq!(
                metadata_width(cells),
                Length::Fixed(cells as f32 * METADATA_ADVANCE),
                "{longest} の桝"
            );
        }
        // 使用量の書式は測る側と描く側で同じ関数を通る(ずれると列が切れる)。
        assert_eq!(context_label(100.0), "100%");
        assert_eq!(context_label(7.4), "7%");
    }

    /// パネルの行と配下の行で属性の右端が同じ桁に落ちる。
    ///
    /// `spacing` は最後の要素との間にも入るので、行末の余白を素の 6px で揃えると
    /// パネル(7)と配下(5)で 2px ずれる。
    #[test]
    fn the_attribute_block_ends_at_the_same_column_on_both_rows() {
        let right_edge = |spacing: f32| match session_row_tail(spacing) {
            Length::Fixed(tail) => tail + spacing,
            other => panic!("行末は実寸で置く: {other:?}"),
        };
        assert_eq!(right_edge(SESSION_ROW_SPACING), SESSION_ROW_TAIL);
        assert_eq!(right_edge(AGENT_ROW_SPACING), SESSION_ROW_TAIL);
    }

    /// 一覧の行が面を持つのは指を乗せたときだけ。前面のタブは題が光る。
    #[test]
    fn session_rows_only_lift_under_the_pointer() {
        let hovered = session_button_style(true, button::Status::Hovered);
        let idle = session_button_style(true, button::Status::Active);

        assert_eq!(
            hovered.background,
            Some(Background::Color(surface_raised()))
        );
        assert_eq!(idle.background, None);
        // 押せない行はホバーで反応しない。
        assert_eq!(
            session_button_style(false, button::Status::Hovered).background,
            None
        );
    }
}
