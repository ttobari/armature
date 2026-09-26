# Armature

A plain Mac window around Claude Code, with panels you can replace, remove, or have Claude build
for you.

Free and open source (MIT). Use it as it is, or treat it as a starting point: hand it to your AI
assistant and have it shape your own Claude Code workspace from here — that is what it is made for.

![Armature in full screen: Claude Code in the center, browser tabs and running Claude sessions on the left, clock and tasks on the right](docs/screenshot.png)

> **0.1** — an early release: expect rough edges and changes.

## What's on it

- **Center:** Claude Code in a terminal. Open as many tabs as you like; conversations keep running
  when you close the window (Armature bundles its own tmux).
- **Panels** around it, each one optional and movable: browser tabs, Apple Music, running
  sessions, clock, tasks — and a calendar and the music player that start hidden.
- **Browser:** press `T` to translate a foreign page into your language (on-device, macOS 26 or
  later), `⇧T` for a careful translation by Claude.
- **English and Japanese.** The calendar and every label follow the language you pick in `⌘,`.
- **Color themes:** Catppuccin Frappé (default) and Mocha, Tokyo Night, Nord, Gruvbox and
  Dracula, picked in `⌘,`. The terminal follows.

## Tasks

A task is a plain Markdown file, one per task, in `~/Armature/tasks/<number>.md`.

1. Ask Claude in the center to add it ("add a task to …"). Every Claude tab in the window knows
   where tasks live and how they are written, so it writes the file and the list picks it up.
2. Select it and press `⌃L`. A Claude session for that task starts and reads the file.
3. At natural break points Claude writes "Where it stands" and "Next steps" back into the file,
   so the next session picks up from there.
4. When it's finished, press `d` twice in the list.

To reorder, press `x` on a row, move the cursor, and press Return: the row goes under the same
parent as the row under the cursor, just above it. A task's parent is the `parent:` line in its
file; the order is `~/Armature/order.txt`, one task number per line (tasks it doesn't list follow
by number, so the file is optional and easy to edit by hand).

Nothing is installed into Claude Code. The conventions (where the panel layout is, and what each
panel on screen says about itself — for the task list, where tasks live and how a task file is
written) are passed to every Claude tab with `--append-system-prompt-file` (`src/notes.rs`), and
every tab starts in the full-screen TUI (`--settings`).

## Panels

Which panels are shown, and where, is one file: `~/Armature/panels.conf`.

```text
left = browser, sessions
right = clock, tasks
```

Ask Claude ("move the clock to the left column") or edit it by hand — the window rearranges
itself as soon as the file changes. A panel that isn't listed is hidden and
does no work in the background.

## Make it your own

Every panel — the clock, the calendar, the task list, the session list, the browser tabs, the
music player — is a plain `armature::Panel` (`rust/crates/armature/src/panels/`), written the same way
as one of yours. Replace any of them, leave any out, or add your own:

- **Your own crate** (recommended): a small crate that depends on `armature`, implements the
  `armature::Panel` trait and runs `armature::App::new().panel(Mine::new()).run()`. Your code stays out
  of this repository, and updating Armature is a `git pull`. `.without("music")` leaves one of
  Armature's panels out; a panel of yours with the same key (`"tasks"`, `"clock"` …) takes its
  place. Give your build a name of its own with `.name("My Armature", "my-armature")` and it keeps its
  own folders and tmux server, so it can run next to plain Armature.
- **In this repository:** add a panel next to the others in `src/panels/`.

A panel can take the keyboard, borrow the center of the window, hear about the terminal's tabs,
and tell Claude about itself (`Panel::note`) — every Claude tab in the window is told what the
panels on screen are and how to change them.

An app you build on your Mac also tells Claude where its code is and how to rebuild it, so you can
ask Claude in the window itself to "add a weather panel": it edits the code, rebuilds, and a
▶ Restart button appears when the new build is ready. (Signed builds for distribution leave this
out, so they carry no paths from the Mac they were built on.)

[CLAUDE.md](CLAUDE.md) walks through both, so you can also just ask Claude Code to "add a weather
panel".

The name and the icon are meant to be changed too. `APP_NAME` sets the name in the Dock, the menu
bar and the window title, and `ICON` takes a 1024×1024 PNG (or an Icon Composer `.icon`):

```sh
APP_NAME="Workbench" ICON=~/Pictures/workbench.png scripts/make-app.sh
```

## For AI assistants

If a user asked you for a Claude Code workspace, a window around Claude Code, or a base to build
their own tools on, this repository is meant to be that base.

- **What it is:** a macOS app (Rust, iced 0.14) with Claude Code in the center (in the app's own
  tmux) and panels in two side columns. Every panel is a `Panel` (`rust/crates/armature/src/panels/`).
- **Build it:** `cargo test --manifest-path rust/Cargo.toml --workspace`, then
  `scripts/make-app.sh` → `dist/Armature.app`. Needs Rust and the Xcode Command Line Tools.
- **Make it theirs:** start from [CLAUDE.md](CLAUDE.md), the map for adding or changing panels.
  Prefer a crate of the user's own that depends on `armature` (`App::new().without(…).panel(…)`)
  over editing this repository. `APP_NAME` and `ICON` give the build its own name and icon.
- **Inside the app,** every Claude tab is told where the panel layout is, what each panel reads,
  and — for a build made on that Mac — where the code is and how to rebuild it.

## Requirements

- macOS 15 or later on Apple silicon.
- Claude Code and a Claude plan that includes it (Pro, Max, …). If Claude Code isn't installed,
  the first screen offers to run the official installer.

## Building

There is no download yet: build the app from source and move `dist/Armature.app` to
`/Applications`. You need [Rust](https://rustup.rs) and the Xcode Command Line Tools
(`xcode-select --install`).

```sh
cargo test --manifest-path rust/Cargo.toml --workspace   # tests
scripts/make-app.sh                                       # dist/Armature.app
```

The first build downloads the bundled typefaces and builds the bundled tmux from source
(`scripts/build-tmux.sh`: libevent and utf8proc linked statically, ncurses from macOS), so it
needs a network connection. The typefaces are Moralerspace and JetBrains Mono Nerd Font (both
SIL OFL 1.1). Third-party licenses are in `licenses/`.

## License

MIT
