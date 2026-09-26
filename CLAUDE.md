# Working on Armature

Armature is a plain frame around Claude Code: the people who use it add panels of their own. This file is the map for
Claude Code when it adds or changes a panel.

Every panel in the window — the clock, the task list, the session list, all of them — is a
`armature::Panel`. The panels Armature comes with (`src/panels/`) are written exactly like one of yours,
so they are also the examples to read.

## Layout of the repository

```
rust/
  crates/armature/        the app (Iced 0.14 + wgpu). A library; the binary only calls App::run
    src/lib.rs           the window: terminal, browser page, key routing (see "How the app is wired")
    src/panel.rs         the Panel trait, Host (what a panel can ask for), Notice (what it hears)
    src/panels/          the panels Armature comes with: clock, calendar, tasks, sessions,
                         browser_tabs, music — each a plain Panel
    src/layout.rs        panels.conf: which panel sits where
    src/notes.rs         what every Claude tab is told (Armature's part + each panel's Panel::note)
    src/palette.rs       the colors (the six color themes) and corner radii
    src/terminal.rs      Armature's own tmux, and how Claude tabs are started
    src/main.rs          the plain Armature binary
    assets/              bundled pages and images (welcome.html …)
    vendor/              a patched copy of iced_term
  crates/armature-core/   the layer below (Claude Code sessions, task files and their order, calendar,
                       paths, lang)
scripts/
  install.sh           checks the tools, builds, puts the app in Applications and opens it
  make-app.sh          builds the app bundle into dist/ (and signs it); REUSE_FROM=<an Armature.app>
                       takes tmux and the fonts from it instead of building and fetching them
  test.sh              the tests, with make-app.sh's settings (one build of the dependencies)
  fetch-fonts.sh       the bundled fonts into dist/fonts (in an app: Contents/Resources/fonts)
  build-tmux.sh        builds the bundled tmux from source
  check-min-os.sh      stops a build whose binaries weak-link C functions newer than macOS 15.0
  release.sh           signed, notarized DMG (carries its own source, unpacked to ~/Armature/source)
```

## Two ways to add a panel

### 1. Your own crate (keeps your code out of this repository)

Make a crate next to this checkout:

```toml
# my-armature/Cargo.toml
[package]
name = "my-armature"
version = "0.1.0"
edition = "2024"

[dependencies]
armature = { path = "../armature/rust/crates/armature" }
```

```rust
// my-armature/src/main.rs
use armature::iced::Task;
use armature::iced::widget::{button, column, text};
use armature::{App, Element, Host, Panel, tr};

#[derive(Default)]
struct Counter {
    count: u32,
    note: String,
}

#[derive(Clone, Debug)]
enum Message {
    Up,
    Docs,
    Opened(Result<(), String>),
}

impl Panel for Counter {
    const KEY: &'static str = "counter"; // the name in panels.conf
    type Message = Message;

    fn view(&self) -> Element<'_, Message> {
        column![
            text(self.count),
            button("+1").on_press(Message::Up),
            button(tr!("Docs", "資料")).on_press(Message::Docs),
            text(&self.note),
        ]
        .spacing(6)
        .into()
    }

    fn update(&mut self, message: Message, host: &mut Host) -> Task<Message> {
        match message {
            Message::Up => self.count += 1,
            // `host.open_url(…);` alone is enough; `.map` also hears how it went.
            Message::Docs => return host.open_url("https://docs.rs/iced").map(Message::Opened),
            Message::Opened(outcome) => self.note = outcome.err().unwrap_or_default(),
        }
        Task::none()
    }

    fn on_exit(&mut self) {
        // Save what you need to keep: Armature ends the process right after this.
    }
}

fn main() -> Result<(), armature::Error> {
    App::new()
        .name("My Armature", "my-armature") // optional: folders and tmux of its own
        .panel(Counter::default())
        .run()
}
```

Build the app bundle with this repository's script, pointed at your crate:

```sh
MANIFEST=~/dev/my-armature/Cargo.toml PACKAGE=my-armature BINARY_NAME=my-armature \
APP_NAME="My Armature" BUNDLE_ID=com.example.my-armature DIST=~/dev/my-armature/dist \
    ~/dev/armature/scripts/make-app.sh
```

`App::new()` has Armature's own panels; yours are added after them. Leave one of Armature's out with
`.without("music")`, or replace it by adding yours under the same key (`"clock"`, `"tasks"` …) —
it takes that panel's place. `App::bare()` starts with none at all.

What a `Panel` gets (see `src/panel.rs`):

- `KEY`: its name in `panels.conf`. A new panel starts hidden when a `panels.conf` already
  exists; add its key to the file (or ask Claude to).
- `FILLS`: take the rest of the column's height (like the task list) instead of the content's.
- `SIDE`: the column it joins. `Side` may grow, so match it with a `_` arm.
- `PADDED`: `false` drops the inner margin, to draw up to its edges (a picture, a map).
- `SHOWN`: `false` leaves it out of the arrangement Armature starts with (like the calendar).
- `view`: the content.
- `update`: its own messages, plus a `Host` to ask Armature for things — `open_url` (`http`/`https`
  only; the browser panel, or the default browser when it's off), `open_claude(prompt)`,
  `start_claude(title, args)`, `select_tab(target)`, `move_tabs(order)`,
  `select_browser_tab(index)`, `release_keys()`. Each returns an `Outcome`: drop it, or
  `.map(…)` it into one of your messages and return that task to hear `Ok(())` or the reason it
  failed. (In a `match` arm that returns nothing, write `{ host.open_url(url); }`.)
- `notice`: what Armature knows, as it changes — `Notice::Tabs` (the terminal's tabs),
  `Notice::BrowserTabs`, `Notice::Keys` (the panel got or lost the keyboard), `Notice::Dismiss`
  (close your center view). More may come: match with a `_` arm.
- `KEYS`, `SHORTCUT`, `key`: a panel with `KEYS = true` takes the keyboard when it is clicked (or
  with ⌘ + its `SHORTCUT` letter, or ⌘L); keys then go to `key` until the terminal takes them
  back. Return `None` for a key you don't use. The task list is the example.
- `center`: something to show in the middle of the window instead of the terminal (the
  calendar's six months). Esc and other screens send `Notice::Dismiss`.
- `note`: what Claude should know about the panel — the files it reads, how to change what it
  shows. The notes of the panels on screen go to every Claude tab Armature starts, so a user can
  just ask Claude to "add a task" or "put this in my panel". The task list's note is the example.
- `subscription`: timers and watchers. Runs only while the panel is shown.
- `on_exit`: called once on every panel when Armature quits (close button, ⌘Q, Quit in the Dock,
  before a restart). `Drop` does not run after it.
- Messages need `Clone + Debug + Send`, not `Sync`.
- Colors: `armature::palette` (`text_primary()`, `accent_active()` …) follows the color theme the
  user picked in ⌘,.
- Text: `armature::tr!("English", "日本語")` for everything the user reads.
- Files: keep your own under `armature::paths::state()`.

A build and plain Armature are kept apart by `App::name(name, slug)`: the task folder `~/<name>`
(with its `panels.conf`), the state folder `~/Library/Application Support/<slug>`, and the tmux
server `tmux -L <slug>`. Without a name they share all three; `panels.conf` then keeps the keys
one build doesn't know when the other rewrites it, so neither drops the other's panels. To share
only the tasks, set `TASKS_HOME` in `<state folder>/config.env`.

Armature never changes its own process environment: children (tmux, claude, shells) get theirs
through `armature_core::proc::child_env`, so your `main` and your panels may start threads freely.

Armature turns on only the parts of tokio it uses (`rt`, `rt-multi-thread`, `sync`, `time`,
`macros`). A panel that uses more of it (`tokio::process`, `fs`, `net`, `io-util` …) lists those
features in its own crate's `tokio` dependency.

Updating Armature is `git pull` in this checkout, then building your crate again.

### 2. A panel in this repository

1. Write `src/panels/<panel>.rs` (or a folder, like `src/panels/tasks/`): a type that implements
   `Panel`, its messages, and the pure logic with tests. Take colors from `palette.rs`.
2. Add `mod <panel>;` and `pub use` it in `src/panels/mod.rs`.
3. Put it in `App::new()` in `lib.rs` (`.builtin(panels::Mine::new())`), and add its key to the
   default arrangement in `layout.rs` (`DEFAULT_LEFT` / `DEFAULT_RIGHT`) if it should be shown
   from the start.
4. A macOS permission it needs goes into the Info.plist in `scripts/make-app.sh`.

Nothing else in Armature needs to know about it.

## How the app is wired (`lib.rs`)

| Where | What |
|---|---|
| `struct Cockpit` | the window's state: the terminal, the browser page, the panels (`custom`), which one holds the keys |
| `Cockpit::new` | starting values and the first loads |
| `enum Message` | everything that can happen; a panel's own messages travel as `Custom(index, payload)` |
| `Cockpit::update` | what each message does, then `tell_changes` sends the panels what changed (`Notice`). Heavy work (files, commands, network) goes to `Task::perform` + `tokio::task::spawn_blocking` and comes back as another message. **Never wait in update or view** — the same holds in a panel |
| `Cockpit::view` | the window: left column, center (terminal / browser / a panel's `center` / settings / help), right column |
| `Cockpit::column_view` / `panel_view` | one column from `panels.conf`, one panel |
| `Cockpit::subscription` | Armature's timers, and each shown panel's `subscription` |
| `Shortcut` / `shortcut_for_key` / `pick_shortcut` | Armature's own ⌘ keys; a panel's `SHORTCUT`; every other key to the panel holding the keys |

## Language

Every string the user reads is written in both languages where it is used:
`tr!("Tasks", "タスク")`, or `tr!(format!(…), format!(…))`. The language is picked in `⌘,`
(the first time, from the Mac's preferred language). Log lines and internal errors can stay
English-only. Tests that check text pin the language for their own thread:
`let _ja = armature_core::lang::scoped(armature_core::lang::Lang::Ja);`.

Task files are read with English or Japanese keys (`status: done` = `状態: 完了`); new ones are
written in the current language.

## Rules

- **Take paths from `armature_core::paths`**: `paths::state()` for the app's state,
  `paths::tasks_dir()` for tasks. Don't write fixed paths under the home folder or another tool's
  folders.
- **Don't use Claude Code's login token.** Free / Pro / Max OAuth tokens are only for Claude Code
  and Claude.ai (Anthropic's terms). For usage limits, use the `rate_limits` Claude Code passes to
  the statusline.
- Keep the look plain: iced's own widgets, colors and radii from `palette.rs` (which follow the
  color theme). There are no shaders or post-processing passes; a panel that wants a different
  look draws it in its own `view`.
- tmux: only the app's own socket (`-L <slug>`, `-L armature` for plain Armature). Never touch the
  user's tmux.
- Don't change the process environment (`set_var` / `remove_var`). Start child processes through
  `armature_core::proc::child_env`, which drops the parent session's markers and sets `PATH`, the
  locale and the colors for the child only.
- A panel that needs a macOS permission (camera, controlling Music …) needs its usage text in the
  Info.plist written by `scripts/make-app.sh`.
- Run `cargo test --manifest-path rust/Cargo.toml --workspace` before you're done.

## Building

```sh
scripts/test.sh                                          # tests (release, sharing the app's build)
scripts/make-app.sh                                       # dist/Armature.app
scripts/install.sh                                        # the above, into Applications, opened
```

## Panels that would be fun to add

- Weather (Open-Meteo needs no key)
- Today's note (one Markdown file per day)
- A Git diff list
- An RSS feed
- Usage limits (from the statusline's `rate_limits`)
