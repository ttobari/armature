use crate::backend::{
    Backend, Command, LinkAction, MouseButton, RenderableContent,
};
use crate::bindings::{BindingAction, BindingsLayout, InputKind};
use crate::terminal::{Event, Terminal};
use crate::theme::TerminalStyle;
use alacritty_terminal::index::Point as TerminalGridPoint;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::{cell, TermMode};
use alacritty_terminal::vte::ansi::{self as ansi, NamedColor};
use iced::alignment::Vertical;
use iced::font::{Style as FontStyle, Weight as FontWeight};
use iced::mouse::{Cursor, ScrollDelta};
use iced::widget::canvas::Path;
use iced::widget::container;
use iced::{
    Color, Element, Font, Length, Point, Rectangle, Size, Theme, Vector,
};
use iced_core::clipboard::Kind as ClipboardKind;
use iced_core::input_method::{self, InputMethod};
use iced_core::keyboard::{Key, Modifiers};
use iced_core::mouse::{self, Click};
use iced_core::text::{Alignment, LineHeight, Shaping, Wrapping};
use iced_core::widget::operation::{self, Focusable};
use iced_graphics::core::widget::{tree, Tree};
use iced_graphics::core::Widget;
use iced_graphics::geometry::Stroke;

pub struct TerminalView<'a> {
    term: &'a Terminal,
}

impl<'a> TerminalView<'a> {
    pub fn show(term: &'a Terminal) -> Element<'a, Event> {
        container(Self { term })
            .width(Length::Fill)
            .height(Length::Fill)
            // 端末の高さはセル高の倍数に丸まる(layout の cell_aligned_height)。
            // この包みが Fill なので、外側のコンテナに align を掛けても丸めの
            // 余りの置き場は変えられない——ここで下詰めにして、余りを目立つ
            // 下端(入力欄の直下)ではなく上端へ送る。
            .align_y(iced::Alignment::End)
            .style(|_| term.theme.container_style())
            .into()
    }

    pub fn focus<Message: 'static>(
        id: iced::widget::Id,
    ) -> iced::Task<Message> {
        iced::widget::operation::focus(id)
    }

    fn is_cursor_in_layout(
        &self,
        cursor: Cursor,
        layout: iced_graphics::core::Layout<'_>,
    ) -> bool {
        if let Some(cursor_position) = cursor.position() {
            let layout_position = layout.position();
            let layout_size = layout.bounds();
            let is_triggered = cursor_position.x >= layout_position.x
                && cursor_position.y >= layout_position.y
                && cursor_position.x < (layout_position.x + layout_size.width)
                && cursor_position.y < (layout_position.y + layout_size.height);

            return is_triggered;
        }

        false
    }

    fn is_cursor_hovered_hyperlink(&self, state: &TerminalViewState) -> bool {
        let content = self.term.backend.renderable_content();
        if let Some(hyperlink_range) = &content.hovered_hyperlink {
            return hyperlink_range.contains(&state.mouse_position_on_grid);
        }

        false
    }

    fn handle_resize(
        &mut self,
        state: &mut TerminalViewState,
        layout: iced_graphics::core::Layout<'_>,
        shell: &mut iced_graphics::core::Shell<'_, Event>,
    ) {
        let layout_size = layout.bounds().size();
        if state.size != layout_size {
            state.size = layout_size;
            let cmd = Command::Resize(
                Some(layout_size),
                Some(self.term.font.measure),
            );
            shell.publish(Event::BackendCall(self.term.id, cmd));
        }
    }

    fn handle_focus(
        &self,
        event: &iced_core::Event,
        state: &mut TerminalViewState,
        is_cursor_in_layout: bool,
    ) {
        use iced::Event::Mouse;
        use iced_core::mouse::{Button::Left, Event::ButtonPressed};

        if let Mouse(ButtonPressed(Left)) = event {
            state.focus = is_cursor_in_layout;
        }
    }

    fn handle_mouse_event(
        &self,
        state: &mut TerminalViewState,
        layout_position: Point,
        cursor_position: Point,
        event: &iced::mouse::Event,
    ) -> Vec<Command> {
        let mut commands = Vec::new();
        let terminal_content = self.term.backend.renderable_content();
        let terminal_mode = terminal_content.terminal_mode;

        match event {
            iced_core::mouse::Event::ButtonPressed(
                iced_core::mouse::Button::Left,
            ) => {
                if !state.is_focused() {
                    return Vec::default();
                }

                Self::handle_left_button_pressed(
                    state,
                    &terminal_mode,
                    cursor_position,
                    layout_position,
                    &mut commands,
                );
            },
            iced_core::mouse::Event::CursorMoved { position } => {
                if !state.is_focused() {
                    return Vec::default();
                }

                Self::handle_cursor_moved(
                    state,
                    self.term.backend.renderable_content(),
                    position,
                    layout_position,
                    &mut commands,
                );
            },
            iced_core::mouse::Event::ButtonReleased(
                iced_core::mouse::Button::Left,
            ) => {
                if !state.is_focused() {
                    return Vec::default();
                }

                Self::handle_button_released(
                    state,
                    &terminal_mode,
                    &self.term.bindings,
                    &mut commands,
                );
            },
            iced::mouse::Event::WheelScrolled { delta } => {
                Self::handle_wheel_scrolled(
                    state,
                    *delta,
                    &self.term.font.measure,
                    &mut commands,
                );
            },
            _ => {},
        }

        commands
    }

    fn handle_left_button_pressed(
        state: &mut TerminalViewState,
        terminal_mode: &TermMode,
        cursor_position: Point,
        layout_position: Point,
        commands: &mut Vec<Command>,
    ) {
        let cmd = if Self::mouse_reports_to_pty(
            *terminal_mode,
            state.keyboard_modifiers,
        ) {
            Command::MouseReport(
                MouseButton::LeftButton,
                state.keyboard_modifiers,
                state.mouse_position_on_grid,
                true,
            )
        } else {
            let current_click = Click::new(
                cursor_position,
                mouse::Button::Left,
                state.last_click,
            );
            let selection_type = match current_click.kind() {
                mouse::click::Kind::Single => SelectionType::Simple,
                mouse::click::Kind::Double => SelectionType::Semantic,
                mouse::click::Kind::Triple => SelectionType::Lines,
            };
            state.last_click = Some(current_click);
            // **押した時点では選択を始めない**。
            //
            // 押すたびに `SelectStart` を送っていたので、置き場を決めるためだけの
            // 一叩きでも選択が生まれていた。しかも trackpad の tap-to-click は
            // 一度の叩きが2回3回の押下として届くことがあり、Iced の判定
            // (300ms・6px 以内は続きの叩き)がそれを Double / Triple と読む
            // ——`SelectionType::Lines` は行まるごと、つまり画面いっぱいの選択になる。
            //
            // 始めるのは指が実際に動いてから。押し離しただけなら選択は消す。
            state.pending_selection = Some((
                selection_type,
                (
                    cursor_position.x - layout_position.x,
                    cursor_position.y - layout_position.y,
                ),
            ));
            state.is_dragged = true;
            state.drag_moved = false;
            return;
        };
        commands.push(cmd);
        state.is_dragged = true;
        state.drag_moved = false;
    }

    fn handle_cursor_moved(
        state: &mut TerminalViewState,
        terminal_content: &RenderableContent,
        position: &Point,
        layout_position: Point,
        commands: &mut Vec<Command>,
    ) {
        let cursor_x = position.x - layout_position.x;
        let cursor_y = position.y - layout_position.y;
        let was_on_grid = state.mouse_position_on_grid;
        state.mouse_position_on_grid = Backend::selection_point(
            cursor_x,
            cursor_y,
            &terminal_content.terminal_size,
            terminal_content.grid.display_offset(),
        );

        // Handle command or selection update based on terminal mode and modifiers
        if state.is_dragged {
            // **引きずったかどうかは升目で見る。** 画素で見ると、trackpad の
            // 一叩きに必ず混ざる 1px 未満のブレまで「引きずった」に数えてしまい、
            // 離したときの ⌘クリック(リンクを開く)が毎回弾かれていた
            // (2026-08-23 実測: 6回の ⌘クリック全部が drag=true)。
            // 同じ升の中で指が震えただけなら、それは叩きであって引きずりではない。
            if state.mouse_position_on_grid != was_on_grid {
                state.drag_moved = true;
            }
            // 指が動いた。ここで初めて選択を起こす。
            if let Some((selection_type, origin)) = state.pending_selection.take()
            {
                commands.push(Command::SelectStart(selection_type, origin));
            }
            let terminal_mode = terminal_content.terminal_mode;
            let cmd = if terminal_mode.intersects(TermMode::MOUSE_MOTION)
                && Self::mouse_reports_to_pty(
                    terminal_mode,
                    state.keyboard_modifiers,
                ) {
                Command::MouseReport(
                    MouseButton::LeftMove,
                    state.keyboard_modifiers,
                    state.mouse_position_on_grid,
                    true,
                )
            } else {
                Command::SelectUpdate((cursor_x, cursor_y))
            };
            commands.push(cmd);
        }

        // Handle link hover if applicable
        if state.keyboard_modifiers == Modifiers::COMMAND {
            commands.push(Command::ProcessLink(
                LinkAction::Hover,
                state.mouse_position_on_grid,
            ));
        }
    }

    fn handle_button_released(
        state: &mut TerminalViewState,
        terminal_mode: &TermMode,
        bindings: &BindingsLayout, // Use the actual type of your bindings here
        commands: &mut Vec<Command>,
    ) {
        state.is_dragged = false;
        let drag_moved = std::mem::take(&mut state.drag_moved);
        // 動かずに離した=ただの一叩き。始めかけの選択を捨て、前に残っていた
        // 選択も消す。**選択が残っていると ⌘C がそれを掴む。**
        if state.pending_selection.take().is_some() && !drag_moved {
            commands.push(Command::SelectClear);
        }

        if Self::mouse_reports_to_pty(*terminal_mode, state.keyboard_modifiers)
        {
            commands.push(Command::MouseReport(
                MouseButton::LeftButton,
                state.keyboard_modifiers,
                state.mouse_position_on_grid,
                false,
            ));
        }

        let link_action = bindings.get_action(
            InputKind::Mouse(iced_core::mouse::Button::Left),
            state.keyboard_modifiers,
            *terminal_mode,
        );
        // ⌘クリックがどこで落ちたかを追う細工(2026-08-23)。原因が割れたら外す。
        if state.keyboard_modifiers.command() {
            log_link_click(
                drag_moved,
                state.keyboard_modifiers,
                &link_action,
                state.mouse_position_on_grid,
            );
        }
        if !drag_moved && link_action == BindingAction::LinkOpen {
            // **叩いた場所のリンクをその場で引き直す。** `Open` は直前の `Hover`
            // が残したものを開くだけなので、⌘ を押してから指を動かさずに叩くと
            // 何も起きなかった。
            // Hover は「カーソルが動いた」か「修飾キーが変わった」でしか走らず、
            // どちらも起きない叩き方がある。ここで先に引けば取りこぼさない。
            commands.push(Command::ProcessLink(
                LinkAction::Hover,
                state.mouse_position_on_grid,
            ));
            commands.push(Command::ProcessLink(
                LinkAction::Open,
                state.mouse_position_on_grid,
            ));
        }
    }

    fn mouse_reports_to_pty(
        terminal_mode: TermMode,
        modifiers: Modifiers,
    ) -> bool {
        // **⌘ を持った叩きは中のプログラムへ渡さない。** ⌘クリックは端末アプリ
        // 自身の身振り(リンクを開く)で、iTerm2 も Terminal.app もそう扱う。
        // 渡していたせいで、中の Claude Code も同じ叩きを受けて `open <url>` を
        // 走らせ、パネルのブラウザと Safari の両方が開いていた。
        //
        // ⇧ を除くのは今までどおり——選択やスクロールを中へ渡さないための穴。
        terminal_mode.intersects(TermMode::MOUSE_MODE)
            && !modifiers.contains(Modifiers::SHIFT)
            && !modifiers.contains(Modifiers::COMMAND)
    }

    fn handle_wheel_scrolled(
        state: &mut TerminalViewState,
        delta: ScrollDelta,
        font_measure: &Size<f32>,
        commands: &mut Vec<Command>,
    ) {
        match delta {
            ScrollDelta::Lines { y, .. } => {
                let lines = y.signum() * y.abs().round();
                commands.push(Command::Scroll(lines as i32));
            },
            ScrollDelta::Pixels { y, .. } => {
                state.scroll_pixels -= y;
                let line_height = font_measure.height; // Assume this method exists and gives the height of a line
                let lines = (state.scroll_pixels / line_height).trunc();
                state.scroll_pixels %= line_height;
                if lines != 0.0 {
                    commands.push(Command::Scroll(lines as i32));
                }
            },
        }
    }

    fn handle_keyboard_event(
        &self,
        state: &mut TerminalViewState,
        clipboard: &mut dyn iced_graphics::core::Clipboard,
        event: &iced::keyboard::Event,
    ) -> Option<Command> {
        let mut binding_action = BindingAction::Ignore;
        let last_content = self.term.backend.renderable_content();
        match event {
            iced::keyboard::Event::ModifiersChanged(m) => {
                state.keyboard_modifiers = *m;
                let action = if state.keyboard_modifiers == Modifiers::COMMAND {
                    LinkAction::Hover
                } else {
                    LinkAction::Clear
                };
                return Some(Command::ProcessLink(
                    action,
                    state.mouse_position_on_grid,
                ));
            },
            iced::keyboard::Event::KeyPressed {
                key,
                text,
                modifiers,
                ..
            } => {
                binding_action = keyboard_binding_action_for_event(
                    &self.term.bindings,
                    last_content.terminal_mode,
                    event,
                );
                // **⌘ を持った打鍵は字にしない。** macOS では ⌘ 付きはアプリの
                // 鍵であって入力ではない。割当の無い ⌘ の組は素通しで字として
                // 打ち込まれていて、⇧⌘R(再起動)を押すと端末に `R` が残った。
                // ここで塞げば、これから足す ⌘ の鍵も同じ事故を起こさない。
                // ⌥ は塞がない——macOS では別の字を出す正当な修飾で、
                // Claude Code の ⌥⏎(改行)もここを通る。
                if binding_action == BindingAction::Ignore
                    && matches!(key, Key::Character(_))
                    && !modifiers.contains(Modifiers::COMMAND)
                {
                    if let Some(content) = text {
                        return Some(Command::Write(
                            content.as_bytes().to_vec(),
                        ));
                    }
                }
            },
            _ => {},
        }

        if let Some(command) = binding_write_command(&binding_action) {
            return Some(command);
        }
        match binding_action {
            BindingAction::Paste => {
                // **ここでは貼らない。**このパネルは指の下にいるときしか鍵を受け取らない
                // ——`handle_focus` は左押しの位置だけで焦点を決めるので、暦や一覧を
                // 一度押しただけで端末は鍵を失い、以後 ⌘V が黙っていた。焦点に依らない
                // 窓側(`Cockpit::paste_into_terminal`)が読んで PTY へ流す。
                //
                // それでも割当だけは持つ。外すと `v` が字として打ち込まれる。
            },
            BindingAction::Copy => {
                // **空の選択で上書きしない**。
                // 素で書くと、何も選んでいない ⌘C が空文字で丸ごと消す。
                let selected = self.term.backend.selectable_content();
                if !selected.is_empty() {
                    clipboard.write(ClipboardKind::Standard, selected);
                }
            },
            _ => {},
        };

        None
    }
}

fn keyboard_binding_action(
    bindings: &BindingsLayout,
    terminal_mode: TermMode,
    key: &Key,
    modifiers: Modifiers,
) -> BindingAction {
    let input = match key {
        Key::Character(value) => InputKind::Char(value.to_ascii_lowercase()),
        Key::Named(code) => InputKind::KeyCode(*code),
        _ => return BindingAction::Ignore,
    };
    bindings.get_action(input, modifiers, terminal_mode)
}

fn keyboard_binding_action_for_event(
    bindings: &BindingsLayout,
    terminal_mode: TermMode,
    event: &iced::keyboard::Event,
) -> BindingAction {
    let iced::keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return BindingAction::Ignore;
    };
    keyboard_binding_action(bindings, terminal_mode, key, *modifiers)
}

fn binding_write_command(action: &BindingAction) -> Option<Command> {
    match action {
        BindingAction::Char(character) => {
            let mut buffer = [0; 4];
            Some(Command::Write(
                character.encode_utf8(&mut buffer).as_bytes().to_vec(),
            ))
        },
        BindingAction::Esc(sequence) => {
            Some(Command::Write(sequence.as_bytes().to_vec()))
        },
        _ => None,
    }
}

impl Widget<Event, Theme, iced::Renderer> for TerminalView<'_> {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<TerminalViewState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(TerminalViewState::new())
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &iced_core::layout::Limits,
    ) -> iced_core::layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);
        // PTYはセル単位でしかresizeできない。端数を含む高さをそのまま渡すと、
        // backendは行数をfloorし、描画だけが上詰めになって下端へ数px残る。
        // このlayout境界でセル高の倍数へ丸める。**丸めの余りをどちらへ置くかは
        // ここでは決められない**——iced の layout::positioned は子の move_to を
        // padding位置で上書きする。置き場所は親コンテナの align_y が決める
        // (cockpit は End=下詰めで、余りを目立たない上端へ送る)。
        let (height, _remainder) =
            cell_aligned_height(size.height, self.term.font.measure.height);
        iced::advanced::layout::Node::new(Size::new(size.width, height))
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: iced_core::Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn operation::Operation,
    ) {
        let state = tree.state.downcast_mut::<TerminalViewState>();
        let wid = self.term.widget_id();
        operation.focusable(Some(wid), layout.bounds(), state);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout,
        _cursor: Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<TerminalViewState>();
        let content = self.term.backend.renderable_content();
        let term_size = content.terminal_size;
        let cell_width = term_size.cell_width as f32;
        let cell_height = term_size.cell_height as f32;
        let font_size = self.term.font.size;
        let font_scale_factor = self.term.font.scale_factor;
        let layout_offset_x = layout.position().x;
        let layout_offset_y = layout.position().y;
        let preedit_span = state.preedit.as_ref().map(|preedit| {
            let point = content.grid.cursor.point;
            PreeditSpan::new(
                point.line.0,
                point.column.0,
                preedit_cells(&preedit.content),
            )
        });

        let geom =
            self.term
                .cache
                .draw(renderer, layout.bounds().size(), |frame| {
                    // Precompute constants used in the inner loop
                    let display_offset = content.grid.display_offset() as f32;
                    let cell_size = Size::new(cell_width, cell_height);
                    // We use the background pallete color as a default
                    // because the widget global background color must be the same
                    let default_bg = self
                        .term
                        .theme
                        .get_color(ansi::Color::Named(NamedColor::Background));

                    let mut last_line: Option<i32> = None;
                    let mut bg_batch_rect = BackgroundRect::default();

                    // 塗り物はここに溜めて、背景を全部流し終えてから描く。
                    // **セルごとに即描きしてはいけない**——背景は行ごとにバッチされ
                    // あとからまとめてフラッシュされるので、先に置いた矩形が上塗りで
                    // 消える(2026-08-31 実測: ロゴが背景色の塊になった)。
                    let mut block_fills: Vec<(Point, Size, Color)> = Vec::new();
                    for indexed in content.grid.display_iter() {
                        // Compute per-cell geometry cheaply
                        let line = indexed.point.line.0;
                        let col = indexed.point.column.0 as f32;

                        // Resolve position point for this cell
                        let x = col * cell_width;
                        let y = ((line as f32) + display_offset) * cell_height;
                        // Resolve colors for this cell
                        let mut fg = self.term.theme.get_color(indexed.fg);
                        let mut bg = self.term.theme.get_bg_color(indexed.bg);

                        // If the new line was detected,
                        // need to flush pending background rect and init the new one
                        if last_line != Some(line) {
                            if bg_batch_rect.can_flush() {
                                let line = last_line.unwrap_or(line);
                                frame.fill(
                                    &bg_batch_rect.build(line),
                                    bg_batch_rect.color,
                                );
                            }

                            last_line = Some(line);
                            bg_batch_rect = BackgroundRect::default()
                                .with_cell_height(cell_height)
                                .with_display_offset(display_offset)
                                .with_layout_offset_y(0.0);
                        }

                        // Handle dim, inverse, and selected text
                        if indexed.cell.flags.intersects(
                            cell::Flags::DIM | cell::Flags::DIM_BOLD,
                        ) {
                            fg.a *= 0.7;
                        }
                        if indexed.cell.flags.contains(cell::Flags::INVERSE)
                            || content
                                .selectable_range
                                .is_some_and(|r| r.contains(indexed.point))
                        {
                            std::mem::swap(&mut fg, &mut bg);
                        }

                        // Batch draw backgrounds: skip default background (container already paints it)
                        if bg != default_bg {
                            if bg_batch_rect.can_extend(bg, x) {
                                // Same color and contiguous: extend current run
                                bg_batch_rect.extend(cell_width);
                            } else {
                                // New colored run (or non-contiguous): flush previous run if any
                                if bg_batch_rect.can_flush() {
                                    frame.fill(
                                        &bg_batch_rect.build(line),
                                        bg_batch_rect.color,
                                    );
                                }

                                // Start a new run but do not draw yet; wait for potential extensions
                                bg_batch_rect = BackgroundRect::default()
                                    .with_cell_height(cell_height)
                                    .with_display_offset(display_offset)
                                    .with_layout_offset_y(0.0)
                                    .activate()
                                    .with_color(bg)
                                    .with_start_x(x)
                                    .with_width(cell_width);
                            }
                        } else if bg_batch_rect.can_flush() {
                            // Background returns to default, flush current background rect and init the new one
                            frame.fill(
                                &bg_batch_rect.build(line),
                                bg_batch_rect.color,
                            );

                            bg_batch_rect = BackgroundRect::default()
                                .with_cell_height(cell_height)
                                .with_display_offset(display_offset)
                                .with_layout_offset_y(0.0);
                        }

                        // Draw hovered hyperlink underline (rare; keep per-cell for correctness)
                        if content.hovered_hyperlink.as_ref().is_some_and(
                            |range| {
                                range.contains(&indexed.point)
                                    && range
                                        .contains(&state.mouse_position_on_grid)
                            },
                        ) || indexed
                            .cell
                            .flags
                            .contains(cell::Flags::UNDERLINE)
                        {
                            let underline_height = y + cell_size.height;
                            let underline = Path::line(
                                Point::new(x, underline_height),
                                Point::new(
                                    x + cell_size.width,
                                    underline_height,
                                ),
                            );
                            frame.stroke(
                                &underline,
                                Stroke::default()
                                    .with_width(font_size * 0.15)
                                    .with_color(fg),
                            );
                        }

                        // 塗り物(█▀▄▛▜▝ 等)は字形ではなく桝を直接塗る。
                        // 字形で置くと、桝の高さ(本文書体の行間込み)と字の高さ
                        // (別書体の em)の差が行ごとの横縞になる——Claude Code の
                        // 起動ロゴが割れて見えていた(2026-08-31)。背景と同じ面へ
                        // 置くので、重ね順は背景の上・文字の下で確定する。
                        if let Some((rects, alpha)) = block_element_fill(indexed.c)
                        {
                            let mut color = fg;
                            color.a *= alpha;
                            for [rx, ry, rw, rh] in rects {
                                block_fills.push((
                                    Point::new(
                                        x + rx * cell_size.width,
                                        y + ry * cell_size.height,
                                    ),
                                    Size::new(
                                        rw * cell_size.width,
                                        rh * cell_size.height,
                                    ),
                                    color,
                                ));
                            }
                        }

                        // Handle cursor rendering
                        if content.grid.cursor.point == indexed.point
                            && content
                                .terminal_mode
                                .contains(TermMode::SHOW_CURSOR)
                        {
                            let cursor_color =
                                self.term.theme.get_color(content.cursor.fg);
                            // 桝を塗りつぶす太いカーソルではなく、桝の左端に立つ
                            // 細い縦棒。太さは桝の幅に比例
                            // させる——固定値にすると、字を大きくしたときだけ
                            // 髪の毛のように細くなる。
                            let caret_width =
                                (cell_size.width * 0.16).clamp(1.0, 2.5);
                            let cursor_rect = Path::rectangle(
                                Point::new(x, y),
                                Size::new(caret_width, cell_size.height),
                            );
                            frame.fill(&cursor_rect, cursor_color);
                        }
                    }

                    // Flush any remaining background run at the end
                    if bg_batch_rect.can_flush() {
                        frame.fill(
                            &bg_batch_rect.build(last_line.unwrap_or(0)),
                            bg_batch_rect.color,
                        );
                    }

                    // 背景を全部置き終えてから塗り物を重ねる(上の溜め場の弁)。
                    for (point, size, color) in block_fills {
                        frame.fill(&Path::rectangle(point, size), color);
                    }

                    // Iced標準のIMEオーバーレイはmacOSの選択中preeditを背景色で塗り、
                    // 日本語が下線だけに見える。端末のカーソル位置へ変換中の文字列を
                    // 自前で描き、PTYへ送るのはCommitが来た時だけにする。
                    if let Some(preedit) = state.preedit.as_ref() {
                        let point = content.grid.cursor.point;
                        let x = point.column.0 as f32 * cell_width;
                        let y = (point.line.0 as f32 + display_offset)
                            * cell_height;
                        let cells =
                            preedit_cells(&preedit.content).max(1) as f32;
                        let width = (cells * cell_width)
                            .min((layout.bounds().width - x).max(cell_width));
                        let background = self.term.theme.get_color(
                            ansi::Color::Named(NamedColor::Background),
                        );
                        let foreground = self.term.theme.get_color(
                            ansi::Color::Named(NamedColor::Foreground),
                        );
                        frame.fill(
                            &Path::rectangle(
                                Point::new(x, y),
                                Size::new(width, cell_height),
                            ),
                            background,
                        );
                        frame.stroke(
                            &Path::line(
                                Point::new(x, y + cell_height - 1.0),
                                Point::new(x + width, y + cell_height - 1.0),
                            ),
                            Stroke::default()
                                .with_width(1.0)
                                .with_color(foreground),
                        );
                    }
                });

        use iced::advanced::graphics::geometry::Renderer as _;
        use iced_core::Renderer as _;
        renderer.with_translation(
            Vector::new(layout_offset_x, layout_offset_y),
            |renderer| {
                renderer.draw_geometry(geom);
            },
        );

        // 文字をCanvasへ保存すると、起動時に名前付きフォントの解決が間に合わなかった
        // 一回の代替字形が、その後もキャッシュへ残り続ける。背景・罫線・カーソルだけを
        // Canvasへ置き、文字は毎回rendererへ渡して再起動時の字形を固定しない。
        for indexed in content.grid.display_iter() {
            if !is_renderable_glyph(indexed.c, indexed.cell.flags) {
                continue;
            }

            let line = indexed.point.line.0;
            let column = indexed.point.column.0;
            let span = glyph_cell_span(indexed.cell.flags);
            // preedit の不透明な面は Canvas、端末文字は renderer に載るため、
            // 描画API上は後から来る端末文字の方が前へ出る。未確定文字が占める
            // セルだけ端末文字を描かず、通常のオーバーレイと同じ重ね順にする。
            // 内容や DIM 属性からプレイスホルダーを推測せず、矩形の交差だけを見る。
            if preedit_span
                .is_some_and(|preedit| preedit.covers(line, column, span))
            {
                continue;
            }
            let mut foreground = self.term.theme.get_color(indexed.fg);
            let mut background = self.term.theme.get_bg_color(indexed.bg);
            if indexed
                .cell
                .flags
                .intersects(cell::Flags::DIM | cell::Flags::DIM_BOLD)
            {
                foreground.a *= 0.7;
            }
            if indexed.cell.flags.contains(cell::Flags::INVERSE)
                || content
                    .selectable_range
                    .is_some_and(|range| range.contains(indexed.point))
            {
                std::mem::swap(&mut foreground, &mut background);
            }
            // カーソルの下の字は**消さない**。
            //
            // ここは桝を塗りつぶす太いカーソルの名残——字を地の色で描いて
            // カーソルの面に沈めることで、反転した見た目を作っていた。桝の左端に
            // 立つ細い縦棒へ変えた今は、沈める面が無いので字がただ消える。
            let mut font =
                terminal_symbol_font(indexed.c, self.term.font.font_type);
            if indexed
                .cell
                .flags
                .intersects(cell::Flags::BOLD | cell::Flags::DIM_BOLD)
            {
                font.weight = FontWeight::Bold;
            }
            if indexed.cell.flags.contains(cell::Flags::ITALIC) {
                font.style = FontStyle::Italic;
            }
            draw_terminal_glyph(
                renderer,
                indexed.c,
                line,
                column,
                span,
                foreground,
                font,
                layout_offset_x,
                layout_offset_y,
                content.grid.display_offset(),
                cell_width,
                cell_height,
                font_size,
                font_scale_factor,
                *viewport,
            );
        }

        if let Some(preedit) = state.preedit.as_ref() {
            let point = content.grid.cursor.point;
            let foreground = self
                .term
                .theme
                .get_color(ansi::Color::Named(NamedColor::Foreground));
            let mut column_offset = 0;
            for character in preedit.content.chars() {
                if matches!(character, '\n' | '\r') {
                    continue;
                }
                let span = unicode_cell_span(character);
                draw_terminal_glyph(
                    renderer,
                    character,
                    point.line.0,
                    point.column.0 + column_offset,
                    span,
                    foreground,
                    terminal_symbol_font(character, self.term.font.font_type),
                    layout_offset_x,
                    layout_offset_y,
                    content.grid.display_offset(),
                    cell_width,
                    cell_height,
                    font_size,
                    font_scale_factor,
                    *viewport,
                );
                column_offset += span;
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced_core::Event,
        layout: iced_graphics::core::Layout<'_>,
        cursor: Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn iced_graphics::core::Clipboard,
        shell: &mut iced_graphics::core::Shell<'_, Event>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<TerminalViewState>();
        self.handle_resize(state, layout, shell);

        let is_cursor_in_layout = self.is_cursor_in_layout(cursor, layout);
        self.handle_focus(event, state, is_cursor_in_layout);

        let commands = match event {
            iced::Event::Mouse(mouse_event) if is_cursor_in_layout => self
                .handle_mouse_event(
                    state,
                    layout.position(),
                    cursor.position().unwrap(),
                    mouse_event,
                ),
            iced::Event::Keyboard(keyboard_event) => {
                if !state.is_focused() {
                    return;
                }

                self.handle_keyboard_event(state, clipboard, keyboard_event)
                    .into_iter()
                    .collect()
            },
            iced::Event::InputMethod(input_method_event)
                if state.is_focused() =>
            {
                shell.capture_event();
                let commands = match input_method_event {
                    input_method::Event::Preedit(content, selection) => {
                        state.preedit = (!content.is_empty()).then(|| {
                            input_method::Preedit {
                                content: content.clone(),
                                selection: selection.clone(),
                                text_size: Some(iced_core::Pixels(
                                    self.term.font.size,
                                )),
                            }
                        });
                        Vec::new()
                    },
                    input_method::Event::Commit(content) => {
                        state.preedit = None;
                        vec![Command::Write(content.as_bytes().to_vec())]
                    },
                    input_method::Event::Closed => {
                        state.preedit = None;
                        Vec::new()
                    },
                    input_method::Event::Opened => Vec::new(),
                };
                self.term.cache.clear();
                shell.request_redraw();
                commands
            },
            _ => Vec::new(),
        };

        if state.is_focused()
            && matches!(
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(_))
            )
        {
            let content = self.term.backend.renderable_content();
            let point = content.grid.cursor.point;
            let terminal_size = content.terminal_size;
            let x = layout.position().x
                + point.column.0 as f32 * terminal_size.cell_width as f32;
            let y = layout.position().y
                + (point.line.0 as f32 + content.grid.display_offset() as f32)
                    * terminal_size.cell_height as f32;
            shell.request_input_method(&InputMethod::<&str>::Enabled {
                cursor: Rectangle::new(
                    Point::new(x, y),
                    Size::new(
                        terminal_size.cell_width as f32,
                        terminal_size.cell_height as f32,
                    ),
                ),
                purpose: input_method::Purpose::Terminal,
                // 変換中文字列は端末キャンバス側で描く。Iced標準オーバーレイへ
                // 渡すとmacOSで選択範囲が下線だけになる。
                preedit: None,
            });
        }

        if !commands.is_empty() {
            shell.capture_event();
        }

        for cmd in commands {
            shell.publish(Event::BackendCall(self.term.id, cmd));
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: iced_core::Layout<'_>,
        cursor: iced_core::mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> iced_core::mouse::Interaction {
        let state = tree.state.downcast_ref::<TerminalViewState>();
        let mut cursor_mode = iced_core::mouse::Interaction::Idle;
        let terminal_mode =
            self.term.backend.renderable_content().terminal_mode;
        if self.is_cursor_in_layout(cursor, layout)
            && !terminal_mode.contains(TermMode::SGR_MOUSE)
        {
            cursor_mode = iced_core::mouse::Interaction::Text;
        }

        if self.is_cursor_hovered_hyperlink(state) {
            cursor_mode = iced_core::mouse::Interaction::Pointer;
        }

        cursor_mode
    }
}

fn cell_aligned_height(
    layout_height: f32,
    measured_cell_height: f32,
) -> (f32, f32) {
    let cell_height = (measured_cell_height as u16).max(1) as f32;
    let lines = (layout_height / cell_height).floor();
    if lines < 1.0 {
        return (layout_height, 0.0);
    }
    let height = lines * cell_height;
    (height, layout_height - height)
}

fn preedit_cells(content: &str) -> usize {
    content.chars().map(unicode_cell_span).sum()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PreeditSpan {
    line: i32,
    start: usize,
    end: usize,
}

impl PreeditSpan {
    fn new(line: i32, start: usize, cells: usize) -> Self {
        Self {
            line,
            start,
            end: start.saturating_add(cells.max(1)),
        }
    }

    fn covers(self, line: i32, column: usize, cells: usize) -> bool {
        line == self.line
            && column < self.end
            && column.saturating_add(cells) > self.start
    }
}

fn unicode_cell_span(character: char) -> usize {
    if character.is_ascii() {
        1
    } else {
        2
    }
}

fn glyph_cell_span(flags: cell::Flags) -> usize {
    if flags.contains(cell::Flags::WIDE_CHAR) {
        2
    } else {
        1
    }
}

fn is_renderable_glyph(character: char, flags: cell::Flags) -> bool {
    !matches!(character, ' ' | '\t')
        && !flags.intersects(
            cell::Flags::WIDE_CHAR_SPACER
                | cell::Flags::LEADING_WIDE_CHAR_SPACER,
        )
}

/// 塗り物(Block Elements・U+2580〜U+259F)を、セルに対する比率の矩形へ写す。
///
/// **字形で描くと縦に隙間が残る。** セルの高さは本文の書体を `line_height` 込みで
/// 測った値なのに、これらの字は別書体(Menlo)の em ぶんしか塗らないので、その差が
/// 行ごとの横縞になって現れる(Claude Code の起動ロゴが割れて見えていた・2026-08-31)。
/// これらは「隣のセルと繋がって1枚の面になる」前提で組まれた字なので、字形に頼らず
/// セルを直接塗る。返すのは `[x, y, w, h]` の比率(0.0〜1.0)と、網掛けの濃度。
fn block_element_fill(character: char) -> Option<(Vec<[f32; 4]>, f32)> {
    let code = character as u32;
    let eighths = |n: u32| n as f32 / 8.0;
    // **罫線(U+2500〜U+257F)はここで扱わない。** 横線だけ矩形にしたら、Menlo のまま
    // 残る縦線・角(│┌┐├┼)と太さも縦位置も合わず、表の枠がずれた(2026-08-31 実測)。
    // 罫線は縦横と角が噛み合って1枚の枠になるので、**全部を矩形で描くか、全部を字形に
    // 任せるかの二択**——片側だけ差し替えると必ず接合が壊れる。字形のままで割れは
    // 目立っていないので、当面は触らない。
    let rects = match code {
        // █ 全面
        0x2588 => vec![[0.0, 0.0, 1.0, 1.0]],
        // ▁▂▃▄▅▆▇ 下から n/8
        0x2581..=0x2587 => {
            let h = eighths(code - 0x2580);
            vec![[0.0, 1.0 - h, 1.0, h]]
        }
        // ▉▊▋▌▍▎▏ 左から n/8(U+2589 が 7/8、U+258F が 1/8)
        0x2589..=0x258F => {
            let w = eighths(0x2590 - code);
            vec![[0.0, 0.0, w, 1.0]]
        }
        0x2580 => vec![[0.0, 0.0, 1.0, 0.5]], // ▀ 上半分
        0x2590 => vec![[0.5, 0.0, 0.5, 1.0]], // ▐ 右半分
        0x2594 => vec![[0.0, 0.0, 1.0, 0.125]], // ▔ 上 1/8
        0x2595 => vec![[0.875, 0.0, 0.125, 1.0]], // ▕ 右 1/8
        // ░▒▓ 網掛けは全面を薄く塗って表す
        0x2591..=0x2593 => vec![[0.0, 0.0, 1.0, 1.0]],
        // ▖▗▘▙▚▛▜▝▞▟ 四分割。左上1・右上2・左下4・右下8 の組み合わせ
        0x2596..=0x259F => {
            let quadrants = match code {
                0x2596 => 0b0100,
                0x2597 => 0b1000,
                0x2598 => 0b0001,
                0x2599 => 0b1101,
                0x259A => 0b1001,
                0x259B => 0b0111,
                0x259C => 0b1011,
                0x259D => 0b0010,
                0x259E => 0b0110,
                _ => 0b1110,
            };
            let mut out = Vec::new();
            for (bit, x, y) in [
                (0b0001, 0.0, 0.0),
                (0b0010, 0.5, 0.0),
                (0b0100, 0.0, 0.5),
                (0b1000, 0.5, 0.5),
            ] {
                if quadrants & bit != 0 {
                    out.push([x, y, 0.5, 0.5]);
                }
            }
            out
        }
        _ => return None,
    };
    let alpha = match code {
        0x2591 => 0.25,
        0x2592 => 0.5,
        0x2593 => 0.75,
        _ => 1.0,
    };
    Some((rects, alpha))
}

#[allow(clippy::too_many_arguments)]
fn draw_terminal_glyph(
    renderer: &mut iced::Renderer,
    character: char,
    line: i32,
    column: usize,
    span: usize,
    foreground: Color,
    font: Font,
    layout_offset_x: f32,
    layout_offset_y: f32,
    display_offset: usize,
    cell_width: f32,
    cell_height: f32,
    font_size: f32,
    font_scale_factor: f32,
    viewport: Rectangle,
) {
    use iced_core::text::Renderer as _;
    // 塗り物は Canvas 側(背景と同じ面)で矩形として塗ってある。ここで字形を重ねると
    // 二重になるので抜ける。**renderer.fill_quad で描いてはいけない**——quad は
    // geometry より先のレイヤーに積まれ、あとから来る背景に覆われて消える
    // (2026-08-31 実測: ロゴが背景色の塊になった)。
    if block_element_fill(character).is_some() {
        return;
    }
    let x = layout_offset_x
        + column as f32 * cell_width
        + span as f32 * cell_width / 2.0;
    let y = layout_offset_y
        + (line as f32 + display_offset as f32) * cell_height
        + cell_height / 2.0;
    renderer.fill_text(
        iced_core::Text {
            content: character.to_string(),
            bounds: Size::INFINITE,
            size: iced_core::Pixels(font_size),
            line_height: LineHeight::Relative(font_scale_factor),
            font,
            align_x: Alignment::Center,
            align_y: Vertical::Center,
            shaping: Shaping::Advanced,
            wrapping: Wrapping::None,
        },
        Point::new(x, y),
        foreground,
        viewport,
    );
}

fn terminal_symbol_font(character: char, fallback: Font) -> Font {
    match character {
        '\u{23fa}' => Font::with_name("STIX Two Math"),
        '\u{2500}'..='\u{259f}' => Font::with_name("Menlo"),
        '\u{3000}'..='\u{30ff}'
        | '\u{3400}'..='\u{9fff}'
        | '\u{f900}'..='\u{faff}'
        | '\u{ff00}'..='\u{ffef}' => {
            let mut font = Font::with_name("Hiragino Sans");
            font.weight = fallback.weight;
            font.style = fallback.style;
            font
        },
        _ => fallback,
    }
}

impl<'a> From<TerminalView<'a>> for Element<'a, Event, Theme, iced::Renderer> {
    fn from(widget: TerminalView<'a>) -> Self {
        Self::new(widget)
    }
}

#[derive(Debug, Clone)]
struct TerminalViewState {
    focus: bool,
    is_dragged: bool,
    drag_moved: bool,
    last_click: Option<mouse::Click>,
    /// 押したが、まだ指が動いていない選択。動いたら起こし、動かずに離したら捨てる。
    pending_selection: Option<(SelectionType, (f32, f32))>,
    scroll_pixels: f32,
    keyboard_modifiers: Modifiers,
    size: Size<f32>,
    mouse_position_on_grid: TerminalGridPoint,
    preedit: Option<input_method::Preedit>,
}

impl TerminalViewState {
    fn new() -> Self {
        Self {
            focus: false,
            is_dragged: false,
            drag_moved: false,
            last_click: None,
            pending_selection: None,
            scroll_pixels: 0.0,
            keyboard_modifiers: Modifiers::empty(),
            size: Size::from([0.0, 0.0]),
            mouse_position_on_grid: TerminalGridPoint::default(),
            preedit: None,
        }
    }
}

impl Default for TerminalViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl operation::Focusable for TerminalViewState {
    fn is_focused(&self) -> bool {
        self.focus
    }

    fn focus(&mut self) {
        self.focus = true;
    }

    fn unfocus(&mut self) {
        self.focus = false;
    }
}

#[derive(Default)]
struct BackgroundRect {
    display_offset: f32,
    cell_height: f32,
    layout_offset_y: f32,
    is_active: bool,
    color: Color,
    start_x: f32,
    width: f32,
}

impl BackgroundRect {
    fn with_display_offset(mut self, value: f32) -> Self {
        self.display_offset = value;
        self
    }

    fn with_cell_height(mut self, value: f32) -> Self {
        self.cell_height = value;
        self
    }

    fn with_layout_offset_y(mut self, value: f32) -> Self {
        self.layout_offset_y = value;
        self
    }

    fn with_width(mut self, value: f32) -> Self {
        self.width = value;
        self
    }

    fn with_start_x(mut self, value: f32) -> Self {
        self.start_x = value;
        self
    }

    fn with_color(mut self, value: Color) -> Self {
        self.color = value;
        self
    }

    fn activate(mut self) -> Self {
        self.is_active = true;
        self
    }

    fn build(&self, line: i32) -> Path {
        let flush_y = self.layout_offset_y
            + ((line as f32 + self.display_offset) * self.cell_height);
        Path::rectangle(
            Point::new(self.start_x, flush_y),
            Size::new(self.width, self.cell_height),
        )
    }

    fn can_flush(&self) -> bool {
        self.is_active && self.width > 0.0
    }

    fn can_extend(&self, bg: Color, x: f32) -> bool {
        self.is_active
            && bg == self.color
            && (self.start_x + self.width - x).abs() < f32::EPSILON
    }

    fn extend(&mut self, value: f32) {
        self.width += value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_layout_moves_only_the_fractional_rows_to_the_top() {
        assert_eq!(cell_aligned_height(101.0, 14.8), (98.0, 3.0));
        assert_eq!(cell_aligned_height(98.0, 14.8), (98.0, 0.0));
        assert_eq!(cell_aligned_height(10.0, 14.8), (10.0, 0.0));
        for height in 80..130 {
            let (grid, top) = cell_aligned_height(height as f32, 14.8);
            assert_eq!(
                grid + top,
                height as f32,
                "last PTY row meets the bottom"
            );
        }
    }

    #[test]
    fn focused_plain_escape_becomes_one_backend_write_byte() {
        use iced_core::keyboard::key::Named;

        let bindings = BindingsLayout::new();
        let event = iced::keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            modified_key: Key::Named(Named::Escape),
            physical_key: iced_core::keyboard::key::Physical::Code(
                iced_core::keyboard::key::Code::Escape,
            ),
            location: iced_core::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        };
        let action = keyboard_binding_action_for_event(
            &bindings,
            TermMode::empty(),
            &event,
        );
        let Some(Command::Write(bytes)) = binding_write_command(&action) else {
            panic!("focused Escape must reach the backend as Write");
        };
        assert_eq!(bytes, [0x1b]);
    }

    #[test]
    fn japanese_preedit_uses_two_terminal_cells_per_character() {
        assert_eq!(preedit_cells("日本a"), 5);
    }

    #[test]
    fn preedit_occludes_only_the_terminal_cells_behind_it() {
        let preedit = PreeditSpan::new(4, 8, 4);
        assert!(preedit.covers(4, 8, 1));
        assert!(preedit.covers(4, 11, 1));
        assert!(!preedit.covers(4, 7, 1));
        assert!(!preedit.covers(4, 12, 1));
        assert!(!preedit.covers(5, 8, 1));
    }

    #[test]
    fn wide_glyphs_are_centered_across_two_terminal_cells() {
        assert_eq!(glyph_cell_span(cell::Flags::empty()), 1);
        assert_eq!(glyph_cell_span(cell::Flags::WIDE_CHAR), 2);
    }

    #[test]
    fn every_visible_glyph_family_bypasses_the_canvas_cache() {
        for character in ['A', '日', '▐', '❯'] {
            assert!(is_renderable_glyph(character, cell::Flags::empty()));
        }
        assert!(!is_renderable_glyph(' ', cell::Flags::empty()));
        assert!(!is_renderable_glyph('本', cell::Flags::WIDE_CHAR_SPACER));
    }

    #[test]
    fn claude_record_mark_uses_a_monochrome_symbol_font() {
        let fallback = Font::MONOSPACE;
        assert_eq!(
            terminal_symbol_font('\u{23fa}', fallback),
            Font::with_name("STIX Two Math")
        );
        assert_eq!(terminal_symbol_font('A', fallback), fallback);
        assert_eq!(
            terminal_symbol_font('日', fallback).family,
            Font::with_name("Hiragino Sans").family
        );
    }

    mod handle_left_button_pressed_tests {
        use super::*;
        use alacritty_terminal::index::{Column, Line};

        #[test]
        fn handles_mouse_mode_with_left_click() {
            let mut state = TerminalViewState::new();
            let terminal_mode = TermMode::MOUSE_MODE;
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_left_button_pressed(
                &mut state,
                &terminal_mode,
                cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftButton,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0),
                    },
                    true,
                )
            ));
            assert!(state.is_dragged);
        }

        #[test]
        fn a_press_only_arms_the_selection_it_does_not_start_one() {
            let terminal_mode = TermMode::MOUSE_MODE | TermMode::SGR_MOUSE;
            let cursor_position = Point { x: 200.0, y: 150.0 };
            let layout_position = Point { x: 50.0, y: 50.0 };

            let cases = vec![
                SelectionType::Simple,
                SelectionType::Semantic,
                SelectionType::Lines,
            ];

            for _selection_type in cases {
                let mut state = TerminalViewState::new();
                state.keyboard_modifiers = Modifiers::SHIFT;
                let mut commands = Vec::new();

                TerminalView::handle_left_button_pressed(
                    &mut state,
                    &terminal_mode,
                    cursor_position,
                    layout_position,
                    &mut commands,
                );

                // 押しただけでは何も送らない。選択は指が動いてから起こす。
                assert!(commands.is_empty(), "押した時点で選択が起きている");
                assert!(state.is_dragged);
                assert!(matches!(
                    state.pending_selection,
                    Some((_selection_type, (150.0, 100.0)))
                ));
            }
        }

        /// 指が動いたところで初めて選択が起きる。
        #[test]
        fn moving_while_held_starts_the_armed_selection() {
            let mut state = TerminalViewState::new();
            state.is_dragged = true;
            state.pending_selection =
                Some((SelectionType::Simple, (150.0, 100.0)));
            let mut commands = Vec::new();
            let terminal_content = RenderableContent::default();
            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &Point { x: 210.0, y: 160.0 },
                Point { x: 50.0, y: 50.0 },
                &mut commands,
            );
            assert!(matches!(
                commands.first(),
                Some(Command::SelectStart(SelectionType::Simple, (150.0, 100.0)))
            ));
            assert!(state.pending_selection.is_none());
        }

        /// 動かさずに離したら、始めかけの選択も前の選択も落とす。
        /// **選択が残っていると ⌘C がそれを掴む。**
        #[test]
        fn a_tap_without_movement_clears_the_selection() {
            let mut state = TerminalViewState::new();
            state.is_dragged = true;
            state.drag_moved = false;
            state.pending_selection =
                Some((SelectionType::Lines, (150.0, 100.0)));
            let mut commands = Vec::new();
            TerminalView::handle_button_released(
                &mut state,
                &TermMode::empty(),
                &BindingsLayout::default(),
                &mut commands,
            );
            assert!(
                commands
                    .iter()
                    .any(|command| matches!(command, Command::SelectClear)),
                "一叩きで選択が残っている"
            );
            assert!(state.pending_selection.is_none());
        }
    }

    mod handle_cursor_moved_tests {
        use alacritty_terminal::index::{Column, Line};

        use super::*;

        #[test]
        fn updates_mouse_position_on_grid() {
            let mut state = TerminalViewState::new();
            let terminal_content = RenderableContent::default();
            let mut commands = Vec::new();
            let cases = vec![
                (
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 1.0, y: 1.0 },
                    TerminalGridPoint {
                        line: Line(1),
                        column: Column(1),
                    },
                ),
                (
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 2.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(2),
                        column: Column(2),
                    },
                ),
                (
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 30.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(2),
                        column: Column(30),
                    },
                ),
                (
                    Point { x: 10.0, y: 0.0 },
                    Point { x: 30.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(2),
                        column: Column(20),
                    },
                ),
                (
                    Point { x: 10.0, y: 10.0 },
                    Point { x: 30.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(20),
                    },
                ),
            ];

            for (layout_position, cursor_position, expected) in cases {
                TerminalView::handle_cursor_moved(
                    &mut state,
                    &terminal_content,
                    &cursor_position,
                    layout_position,
                    &mut commands,
                );

                assert_eq!(state.mouse_position_on_grid, expected);
            }
        }

        #[test]
        fn generates_drag_update_command_when_dragged() {
            let mut state = TerminalViewState::new();
            state.is_dragged = true; // Simulate an ongoing drag operation
            let terminal_content = RenderableContent::default();
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::SelectUpdate((95.0, 145.0))
            ));
        }

        #[test]
        fn generates_drag_update_command_when_dragged_in_mouse_motion_mode() {
            let mut state = TerminalViewState::new();
            state.is_dragged = true; // Simulate an ongoing drag operation
            let mut terminal_content = RenderableContent::default();
            terminal_content.terminal_mode = TermMode::MOUSE_MOTION;
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftMove,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(49),
                        column: Column(79),
                    },
                    true,
                )
            ));
        }

        #[test]
        fn generates_drag_update_command_when_dragged_in_srg_mode_with_key_mods(
        ) {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::SHIFT;
            state.is_dragged = true; // Simulate an ongoing drag operation
            let mut terminal_content = RenderableContent::default();
            terminal_content.terminal_mode =
                TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE;
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::SelectUpdate((95.0, 145.0))
            ));
        }

        #[test]
        fn generates_drag_update_and_link_open() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            state.is_dragged = true; // Simulate an ongoing drag operation
            let mut terminal_content = RenderableContent::default();
            terminal_content.terminal_mode = TermMode::SGR_MOUSE;
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 2);
            assert!(matches!(
                commands[0],
                Command::SelectUpdate((95.0, 145.0))
            ));
            assert!(matches!(
                commands[1],
                Command::ProcessLink(
                    LinkAction::Hover,
                    TerminalGridPoint {
                        line: Line(49),
                        column: Column(79),
                    },
                )
            ));
        }
    }

    mod handle_button_released_tests {
        use super::*;
        use alacritty_terminal::index::{Column, Line};

        #[test]
        fn mouse_mode_activated() {
            let mut state = TerminalViewState::new();
            let terminal_mode = TermMode::MOUSE_MODE;
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_button_released(
                &mut state,
                &terminal_mode,
                &bindings,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftButton,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0)
                    },
                    false
                )
            ));
        }

        #[test]
        fn link_open_on_button_release() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            let terminal_mode = TermMode::MOUSE_MODE;
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();

            TerminalView::handle_button_released(
                &mut state,
                &terminal_mode,
                &bindings,
                &mut commands,
            );

            // ⌘ を持った叩きは MOUSE_MODE でも中のプログラムへ渡さない
            // (渡すと中の Claude Code も `open <url>` を走らせ、パネルのブラウザと
            // Safari の両方が開く)。開くのは端末アプリ自身の身振りだけ。
            assert!(
                !commands
                    .iter()
                    .any(|command| matches!(command, Command::MouseReport(..)))
            );
            assert_eq!(commands.len(), 2);
            assert!(matches!(
                commands[0],
                Command::ProcessLink(
                    LinkAction::Hover,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0)
                    }
                ),
            ));
            assert!(matches!(
                commands[1],
                Command::ProcessLink(
                    LinkAction::Open,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0)
                    }
                ),
            ));
        }

        #[test]
        fn link_open_on_button_release_in_non_mouse_mode() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            state.mouse_position_on_grid = TerminalGridPoint {
                line: Line(4),
                column: Column(10),
            };
            let terminal_mode = TermMode::empty(); // Assume SGR_MOUSE mode doesn't affect link opening
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();

            TerminalView::handle_button_released(
                &mut state,
                &terminal_mode,
                &bindings,
                &mut commands,
            );

            // 叩いた升のリンクをその場で引き直してから開くので、Hover と Open の2手。
            assert_eq!(commands.len(), 2);
            assert!(matches!(
                commands[0],
                Command::ProcessLink(
                    LinkAction::Hover,
                    TerminalGridPoint {
                        line: Line(4),
                        column: Column(10)
                    }
                ),
            ));
            assert!(matches!(
                commands[1],
                Command::ProcessLink(
                    LinkAction::Open,
                    TerminalGridPoint {
                        line: Line(4),
                        column: Column(10)
                    }
                ),
            ));
        }

        #[test]
        fn command_drag_does_not_open_a_link() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            state.is_dragged = true;
            state.drag_moved = true;
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();

            TerminalView::handle_button_released(
                &mut state,
                &TermMode::empty(),
                &bindings,
                &mut commands,
            );

            assert!(commands.is_empty(), "Cmd+dragはlink clickではない");
        }

        #[test]
        fn shift_release_does_not_leak_a_mouse_report_to_tmux() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::SHIFT;
            state.is_dragged = true;
            state.drag_moved = true;
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();

            TerminalView::handle_button_released(
                &mut state,
                &TermMode::MOUSE_MODE,
                &bindings,
                &mut commands,
            );

            assert!(
                commands.is_empty(),
                "Shift+dragはlocal selectionで完結する"
            );
        }
    }

    mod handle_wheel_scrolled_tests {
        use super::*;
        use crate::font::TermFont;
        use crate::settings::FontSettings;

        #[test]
        fn scroll_with_lines_downward() {
            let mut state = TerminalViewState::new();
            let font = TermFont::new(FontSettings::default());
            let mut commands = Vec::new();

            TerminalView::handle_wheel_scrolled(
                &mut state,
                ScrollDelta::Lines { y: 3.0, x: 0.0 }, // Scroll down 3 lines
                &font.measure,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(commands[0], Command::Scroll(3)));
        }

        #[test]
        fn scroll_with_lines_upward() {
            let mut state = TerminalViewState::new();
            let font = TermFont::new(FontSettings::default());
            let mut commands = Vec::new();

            TerminalView::handle_wheel_scrolled(
                &mut state,
                ScrollDelta::Lines { y: -2.0, x: 0.0 },
                &font.measure,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(commands[0], Command::Scroll(-2)));
        }

        #[test]
        fn scroll_with_pixels_accumulating_downward() {
            let mut state = TerminalViewState::new();
            let font = TermFont::new(FontSettings::default());
            let mut commands = Vec::new();

            TerminalView::handle_wheel_scrolled(
                &mut state,
                ScrollDelta::Pixels { y: 45.0, x: 0.0 },
                &font.measure,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(commands[0], Command::Scroll(-2)));
            assert_eq!(state.scroll_pixels, -8.600002);
        }

        #[test]
        fn scroll_with_pixels_accumulating_upward() {
            let mut state = TerminalViewState::new();
            let font = TermFont::new(FontSettings::default());
            let mut commands = Vec::new();

            TerminalView::handle_wheel_scrolled(
                &mut state,
                ScrollDelta::Pixels { y: -60.0, x: 0.0 },
                &font.measure,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(commands[0], Command::Scroll(3)));
            assert_eq!(state.scroll_pixels, 5.4000034);
        }
    }
}

/// ⌘クリックの通り道の記録(調べ終えたので、公開版では何も書かない)。
fn log_link_click(
    _drag_moved: bool,
    _modifiers: Modifiers,
    _action: &BindingAction,
    _point: alacritty_terminal::index::Point,
) {
}
