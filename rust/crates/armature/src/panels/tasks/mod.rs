//! The task list: one md file per task in the Armature folder (`~/Armature/tasks`), with bundles
//! (tasks that hold others). Click it or press ⌘R to give it the keys:
//!
//! - `j` / `k` move, `l` / `h` (or Enter) go into / out of a bundle
//! - `⌃L` or `⇧L` starts a Claude tab on the selected task
//! - `d` twice marks it done, `x` picks it up and Enter puts it down elsewhere
//! - `m` / `e` change the model / effort the next tab starts with
//!
//! New tasks are not typed here: ask Claude. The panel's note ([`Panel::note`]) tells every
//! Claude tab where the tasks live and how a task file is written.

mod list;
mod task_tree;

use std::time::{Duration, Instant};

use iced::widget::container;
use iced::{Length, Subscription, Task};
use iced::keyboard::{Key, Modifiers, key::Named};

use crate::{Element, Host, KeyPress, Notice, Panel, tr};

use list::{CompleteSpec, MoveSpec, SettingsSpec};

/// How often the folder is read again (a file Claude wrote shows up within this).
const REFRESH: Duration = armature_core::board::POLL_INTERVAL;
/// Two `d`s within this count as `dd`. It does not wait longer: a list is looked at for a long
/// time, and a forgotten first `d` should not turn a later one into "done".
const STROKE_WINDOW: Duration = Duration::from_millis(900);

/// The task list panel (`tasks` in `panels.conf`).
pub struct Tasks {
    list: list::Tasks,
    scroll_id: iced::widget::Id,
    /// When the first `d` of `dd` came.
    stroke: Option<Instant>,
    /// The task whose Claude tab is being started.
    launching: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    List(list::Message),
    Refresh,
    Expire,
    Loaded(Box<Result<armature_core::board::Board, String>>),
    SettingsSaved(SettingsSpec, Result<(), String>),
    CompleteFinished(CompleteSpec, Result<(), String>),
    MoveFinished(MoveSpec, Result<(), String>),
    Launched(Result<(), String>),
}

impl Tasks {
    #[must_use]
    pub fn new() -> Self {
        Self {
            list: list::Tasks::default(),
            scroll_id: iced::widget::Id::unique(),
            stroke: None,
            launching: None,
        }
    }

    fn load() -> Task<Message> {
        Task::perform(
            async {
                tokio::task::spawn_blocking(list::fetch)
                    .await
                    .map_err(|error| format!("task loader thread ended: {error}"))?
            },
            |result| Message::Loaded(Box::new(result)),
        )
    }

    /// Scrolls so the selected line is in view (the same rule as the browser tab list).
    fn follow(&self) -> Task<Message> {
        let Some(offset) = self.list.scroll_ratio() else {
            return Task::none();
        };
        iced::widget::operation::snap_to(
            self.scroll_id.clone(),
            iced::advanced::widget::operation::scrollable::RelativeOffset { x: 0.0, y: offset },
        )
    }

    fn save_pending_settings(&mut self) -> Task<Message> {
        match self.list.take_action() {
            Some(list::Action::SaveSettings(spec)) => {
                let reply = spec.clone();
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || list::persist_settings(&spec))
                            .await
                            .unwrap_or_else(|error| Err(format!("settings writer thread ended: {error}")))
                    },
                    move |result| Message::SettingsSaved(reply.clone(), result),
                )
            }
            None => Task::none(),
        }
    }

    fn launch(&mut self, host: &mut Host) -> Task<Message> {
        let Some(spec) = self.list.launch_spec() else {
            return Task::none();
        };
        self.list.launching(&spec.id);
        self.launching = Some(spec.id.clone());
        let mut arguments = Vec::new();
        if ["sonnet", "opus", "fable"].contains(&spec.model.as_str()) {
            arguments.extend(["--model".to_string(), spec.model.clone()]);
        }
        if ["low", "medium", "high", "xhigh", "max"].contains(&spec.effort.as_str()) {
            arguments.extend(["--effort".to_string(), spec.effort.clone()]);
        }
        // The first message is the last argument; `--` keeps a title that starts with `-` a message.
        arguments.push("--".to_string());
        arguments.push(opening(&spec.id, &spec.title));
        host.start_claude(tab_name(&spec.id, &spec.title), arguments)
            .map(Message::Launched)
    }

    fn complete(&mut self) -> Task<Message> {
        let now = Instant::now();
        if !is_double(self.stroke, now) {
            self.stroke = Some(now);
            return Task::none();
        }
        self.stroke = None;
        let Some(spec) = self.list.complete_spec() else {
            return Task::none();
        };
        self.list.completing(&spec);
        let reply = spec.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || list::persist_complete(&spec))
                    .await
                    .unwrap_or_else(|error| Err(format!("task writer thread ended: {error}")))
            },
            move |result| Message::CompleteFinished(reply.clone(), result),
        )
    }

    fn put_down(&mut self) -> Task<Message> {
        let Some(spec) = self.list.commit_move() else {
            return Task::none();
        };
        let reply = spec.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || list::persist_move(&spec))
                    .await
                    .unwrap_or_else(|error| Err(format!("task writer thread ended: {error}")))
            },
            move |result| Message::MoveFinished(reply.clone(), result),
        )
    }
}

impl Default for Tasks {
    fn default() -> Self {
        Self::new()
    }
}

impl Panel for Tasks {
    const KEY: &'static str = "tasks";
    const FILLS: bool = true;
    const PADDED: bool = false;
    const KEYS: bool = true;
    const SHORTCUT: Option<char> = Some('r');
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        container(self.list.view(self.scroll_id.clone()).map(Message::List))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn update(&mut self, message: Message, _host: &mut Host) -> Task<Message> {
        match message {
            Message::List(message) => {
                self.list.update(message);
                self.save_pending_settings()
            }
            Message::Refresh => Self::load(),
            Message::Expire => {
                self.list.expire_status(Instant::now());
                Task::none()
            }
            Message::Loaded(result) => {
                match *result {
                    Ok(board) => self.list.set_board(board),
                    Err(error) => self.list.failed(error),
                }
                Task::none()
            }
            Message::SettingsSaved(spec, result) => {
                self.list.settings_saved(&spec, result);
                Task::none()
            }
            Message::CompleteFinished(spec, result) => {
                self.list.completed(&spec, result);
                Task::none()
            }
            Message::MoveFinished(spec, result) => {
                let moved = result.is_ok();
                self.list.moved(&spec, result);
                if moved { Self::load() } else { Task::none() }
            }
            Message::Launched(result) => {
                // The keys stay with the list: picking the next task is what it is for.
                let id = self.launching.take().unwrap_or_default();
                self.list.launched(result.map(|()| {
                    tr!(format!("Started {id}"), format!("{id} を起動した"))
                }));
                Task::none()
            }
        }
    }

    fn key(&mut self, key: &KeyPress, host: &mut Host) -> Option<Task<Message>> {
        let plain = key.modifiers.is_empty();
        let letter = key.latin();
        if letter == Some('l') && (key.modifiers == Modifiers::CTRL || key.modifiers == Modifiers::SHIFT) {
            return Some(self.launch(host));
        }
        if !plain {
            return None;
        }
        if key.key == Key::Named(Named::Enter) || letter == Some('l') {
            // While a line is held, Enter / l puts it down.
            if self.list.is_moving() {
                return Some(self.put_down());
            }
            self.list.enter_bundle();
            return Some(self.follow());
        }
        match letter? {
            'j' => self.list.move_by(1),
            'k' => self.list.move_by(-1),
            'h' => self.list.exit_bundle(),
            'd' => return Some(self.complete()),
            'x' => {
                self.list.toggle_grab();
                return Some(Task::none());
            }
            'm' => {
                self.list.cycle_model();
                return Some(self.save_pending_settings());
            }
            'e' => {
                self.list.cycle_effort();
                return Some(self.save_pending_settings());
            }
            _ => return None,
        }
        Some(self.follow())
    }

    fn notice(&mut self, notice: &Notice, _host: &mut Host) -> Task<Message> {
        match notice {
            // Given the keys (a click, ⌘R): read the folder now and bring the selection into view.
            Notice::Keys(true) => Task::batch([
                Self::load(),
                iced::widget::operation::scroll_to(
                    self.scroll_id.clone(),
                    iced::widget::scrollable::AbsoluteOffset {
                        x: None,
                        y: Some(self.list.scroll_offset()),
                    },
                ),
            ]),
            _ => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(REFRESH).map(|_| Message::Refresh),
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Expire),
        ])
    }

    fn note(&self) -> Option<String> {
        Some(
            tr!(NOTE_EN, NOTE_JA)
                .replace("{tasks_dir}", &armature_core::paths::tasks_dir().to_string_lossy()),
        )
    }
}

/// A second `d` within the window of the first is `dd`.
fn is_double(first: Option<Instant>, now: Instant) -> bool {
    first.is_some_and(|first| now.duration_since(first) <= STROKE_WINDOW)
}

/// The tab's name: the task's number and title.
fn tab_name(id: &str, title: &str) -> String {
    let title = title.trim();
    if title.is_empty() { id.to_string() } else { format!("{id} {title}") }
}

/// The first message of a task's Claude tab: which task, and where its file is.
fn opening(id: &str, title: &str) -> String {
    let file = armature_core::paths::tasks_dir().join(format!("{id}.md"));
    let file = file.display();
    let title = title.trim();
    match (title.is_empty(), armature_core::lang::current()) {
        (true, armature_core::lang::Lang::En) => format!("Start task {id}. File: {file}"),
        (false, armature_core::lang::Lang::En) => format!("Start task {id} \"{title}\". File: {file}"),
        (true, armature_core::lang::Lang::Ja) => format!("タスク {id} を始めて。ファイル: {file}"),
        (false, armature_core::lang::Lang::Ja) => format!("タスク {id}「{title}」を始めて。ファイル: {file}"),
    }
}

/// What every Claude tab is told about the task list. `{tasks_dir}` is filled in.
const NOTE_EN: &str = "\
## Tasks

The task list in the window shows one md file per task in `{tasks_dir}`; it rereads the folder by itself.

### Task file format

```
---
id: 12
parent: 3
status: active
created: 2026-09-24
updated: 2026-09-24
---

# Title

Body (what to do, what has been decided)

## Where it stands

## Next steps
```

- `parent` is the ID of the task this one belongs to (a task with children shows as a bundle). Leave it empty if there is none.
- `status` is `active` or `done`.
- Some files use the Japanese keys instead (`ID`, `親`, `状態`, `作成日`, `更新日`; status `進行` / `完了`, sections `## 現在地` / `## 次にやること`). Keep whichever a file already uses; for a new file, follow the other files in the folder.

### Rules

1. When asked to add a task, write one file numbered one above the highest ID among the md files in the folder, with `status: active` and today's date. Put what the user said into the title and body; don't ask follow-up questions unless the title would be unclear.
2. If this session started from a task, read that file first. If it has next steps, start there. If not, read the body and check with the user what to do before starting.
3. While working on a task, at natural break points (a piece of work is done, the user seems about to leave, the conversation is getting long), rewrite \"## Where it stands\" and \"## Next steps\" so that another session can carry on from those two sections alone. Add them at the end if they are missing. Then set `updated` to today. When the user says \"save\", do this right away.
4. Change `status` only when the user asks.
";

const NOTE_JA: &str = "\
## タスク

窓のタスクの一覧は、`{tasks_dir}` に 1 件 1 枚で置かれた md を出している。一覧はこのフォルダを自分で読み直す。

### タスクのファイルの形

```
---
ID: 12
親: 3
状態: 進行
作成日: 2026-09-24
更新日: 2026-09-24
---

# 題

本文(何をしたいか・決まったこと)

## 現在地

## 次にやること
```

- `親` は所属するタスクの ID(子を持つタスクは束として出る)。無所属なら空。
- `状態` は `進行` か `完了`。
- 英語の欄で書かれたファイルもある(`id`・`parent`・`status`・`created`・`updated`、状態は `active` / `done`、節は `## Where it stands` / `## Next steps`)。ファイルが使っている方に合わせる。新しく作るときは、フォルダのほかのファイルに合わせる。

### 約束

1. タスクを足してと頼まれたら、置き場の md の中で最大の ID に 1 を足した番号で、`状態: 進行`・今日の日付のファイルを 1 枚作る。利用者の言ったことを題と本文にする。題が決まらないときだけ聞き返す。
2. このセッションがタスクから始まったなら、最初にそのファイルを読む。「次にやること」があればそこから始める。無ければ本文を読み、何をするかを利用者に確かめてから始める。
3. タスクの作業中は、区切りのいいところ(ひと仕事終えた・利用者が離れそう・会話が長くなった)で「## 現在地」と「## 次にやること」を書き直す。別のセッションがそこだけを読んで続きから始められる粒度で書く。節が無ければ末尾に作る。書き直したら `更新日` を今日にする。利用者が「保存」「save」と言ったら、すぐにこれをやる。
4. `状態` は利用者に頼まれたときだけ書き換える。
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_names_the_sections_the_list_reads() {
        assert!(NOTE_JA.contains("## 現在地"));
        assert!(NOTE_JA.contains("## 次にやること"));
        assert!(NOTE_EN.contains("## Where it stands"));
        assert!(NOTE_EN.contains("## Next steps"));
        for note in [NOTE_EN, NOTE_JA] {
            assert!(note.contains("{tasks_dir}"));
        }
    }

    #[test]
    fn the_opening_line_points_at_the_task_file() {
        let _en = armature_core::lang::scoped(armature_core::lang::Lang::En);
        let line = opening("12", "Send the quote");
        assert!(line.starts_with("Start task 12 \"Send the quote\"."));
        assert!(line.ends_with("12.md"));
        assert!(opening("7", "  ").starts_with("Start task 7."));
        drop(_en);
        let _ja = armature_core::lang::scoped(armature_core::lang::Lang::Ja);
        let line = opening("12", "見積もりを出す");
        assert!(line.starts_with("タスク 12「見積もりを出す」を始めて。"));
        assert!(line.ends_with("12.md"));
    }

    #[test]
    fn two_ds_within_the_window_complete_the_task() {
        let first = Instant::now();
        assert!(!is_double(None, first));
        assert!(is_double(Some(first), first + STROKE_WINDOW - Duration::from_millis(1)));
        assert!(!is_double(Some(first), first + STROKE_WINDOW + Duration::from_millis(1)));
    }

    /// The keys are read from the physical key: with kana input on, `d` still arrives as `d`.
    #[test]
    fn keys_are_read_from_the_physical_key() {
        use iced::keyboard::key::{Code, Physical};
        let press = |text: &str, code, modifiers| KeyPress {
            key: Key::Character(text.into()),
            physical: Physical::Code(code),
            modifiers,
        };
        assert!(press("し", Code::KeyD, Modifiers::empty()).is('d'));
        assert!(!press("d", Code::KeyD, Modifiers::COMMAND).is('d'));
    }

    /// ⌃L and ⇧L start the selected task; plain `l` goes into a bundle instead.
    #[test]
    fn control_l_and_shift_l_start_the_task() {
        use iced::keyboard::key::{Code, Physical};
        let mut panel = Tasks::new();
        let mut host = Host::default();
        let l = |modifiers| KeyPress {
            key: Key::Character("l".into()),
            physical: Physical::Code(Code::KeyL),
            modifiers,
        };
        // An empty list has nothing to start, but the key is still the panel's.
        assert!(panel.key(&l(Modifiers::CTRL), &mut host).is_some());
        assert!(panel.key(&l(Modifiers::SHIFT), &mut host).is_some());
        assert!(panel.key(&l(Modifiers::COMMAND), &mut host).is_none(), "⌘L is Armature's");
        assert!(panel.key(&l(Modifiers::empty()), &mut host).is_some());
        let unknown = KeyPress {
            key: Key::Character("q".into()),
            physical: Physical::Code(Code::KeyQ),
            modifiers: Modifiers::empty(),
        };
        assert!(panel.key(&unknown, &mut host).is_none(), "a key the list doesn't use is left alone");
    }

    #[test]
    fn a_task_tab_is_named_by_number_and_title() {
        assert_eq!(tab_name("901", "盤の起動"), "901 盤の起動");
        assert_eq!(tab_name("7", "  "), "7");
    }
}
