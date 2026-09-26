use crate::actions::Action;
use crate::backend;
use crate::bindings::{Binding, BindingAction, BindingsLayout, InputKind};
use crate::font::TermFont;
use crate::settings::{FontSettings, Settings, ThemeSettings};
use crate::theme::{ColorPalette, Theme};
use crate::AlacrittyEvent;
use iced::futures::stream::BoxStream;
use iced::futures::{SinkExt, StreamExt};
use iced::widget::canvas::Cache;
use iced::Subscription;
use std::hash::{Hash, Hasher};
use std::io::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{self, UnboundedReceiver};
use tokio::sync::Mutex;

/// 起こし(`AlacrittyEvent::Wakeup`)を流す間隔の下限(ms)。0 なら間引かない。
///
/// PTY から字が届くたびに alacritty は Wakeup を送り、それが1件ずつ iced の用事になって
/// 窓全体を描き直す。Claude Code のように秒に 30 回書き換わる画面では、窓も 30fps で
/// 描き直る。ここで下限を置くと、間隔内に重なった Wakeup は最後の1件だけを期限に流す
/// (字は落ちない——PTY の読み手は止まらず grid へ書き続け、Wakeup は「描き直せ」の
/// 合図でしかない)。最初の1件は即座に流すので、打った字の遅れは前の起こしから
/// 間隔内に打ったときだけ。
static WAKEUP_INTERVAL_MS: AtomicU64 = AtomicU64::new(0);

/// 起こしの間引き幅を置く。`Duration::ZERO` で間引かない(素の iced_term と同じ)。
pub fn set_wakeup_interval(interval: Duration) {
    WAKEUP_INTERVAL_MS.store(interval.as_millis().min(u128::from(u64::MAX)) as u64, Ordering::Relaxed);
}

/// いまの起こしの間引き幅。
#[must_use]
pub fn wakeup_interval() -> Duration {
    Duration::from_millis(WAKEUP_INTERVAL_MS.load(Ordering::Relaxed))
}

/// Wakeup の間引き。`on_wakeup` が「今すぐ流す」か「溜める」かを決め、溜めたぶんは
/// `deadline` の時刻に `fire` で流す。時計は外から渡す(試験で実時間を待たない)。
#[derive(Debug, Default)]
struct Throttle {
    last_sent: Option<Instant>,
    pending: bool,
}

impl Throttle {
    /// Wakeup が来た。真なら今すぐ流す。偽なら溜めた(期限は `deadline`)。
    fn on_wakeup(&mut self, now: Instant, interval: Duration) -> bool {
        if interval.is_zero() {
            self.last_sent = Some(now);
            self.pending = false;
            return true;
        }
        match self.last_sent {
            Some(last) if now.duration_since(last) < interval => {
                self.pending = true;
                false
            },
            _ => {
                self.last_sent = Some(now);
                true
            },
        }
    }

    /// 溜めた Wakeup を流す時刻。溜めていなければ `None`。
    fn deadline(&self, interval: Duration) -> Option<Instant> {
        self.pending.then(|| self.last_sent.map_or_else(Instant::now, |last| last + interval))
    }

    /// 溜めた Wakeup を流した。
    fn fire(&mut self, now: Instant) {
        self.pending = false;
        self.last_sent = Some(now);
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    BackendCall(u64, backend::Command),
}

#[derive(Debug, Clone)]
pub enum Command {
    ChangeTheme(Box<ColorPalette>),
    ChangeFont(FontSettings),
    AddBindings(Vec<(Binding<InputKind>, BindingAction)>),
    ProxyToBackend(backend::Command),
}

pub struct Terminal {
    pub id: u64,
    widget_id: iced::widget::Id,
    pub(crate) font: TermFont,
    pub(crate) theme: Theme,
    pub(crate) cache: Cache,
    pub(crate) bindings: BindingsLayout,
    pub(crate) backend: backend::Backend,
    backend_event_rx: Arc<Mutex<UnboundedReceiver<AlacrittyEvent>>>,
}

impl Terminal {
    pub fn new(id: u64, settings: Settings) -> Result<Self> {
        let (backend_event_tx, backend_event_rx) = mpsc::unbounded_channel();
        let theme = Theme::new(settings.theme);
        let font = TermFont::new(settings.font);

        Ok(Self {
            id,
            widget_id: iced::widget::Id::unique(),
            font,
            theme,
            bindings: BindingsLayout::default(),
            cache: Cache::default(),
            backend: backend::Backend::new(
                id,
                backend_event_tx,
                settings.backend,
            )?,
            backend_event_rx: Arc::new(Mutex::new(backend_event_rx)),
        })
    }

    pub fn widget_id(&self) -> &iced::widget::Id {
        &self.widget_id
    }

    pub fn subscription(&self) -> Subscription<Event> {
        let data = TerminalSubscriptionData {
            id: self.id,
            event_receiver: self.backend_event_rx.clone(),
        };

        Subscription::run_with(data, terminal_subscription_stream)
    }

    pub fn handle(&mut self, cmd: Command) -> Action {
        let mut action = Action::default();

        match cmd {
            Command::ChangeTheme(color_pallete) => {
                self.theme = Theme::new(ThemeSettings::new(color_pallete));
            },
            Command::ChangeFont(font_settings) => {
                self.font = TermFont::new(font_settings);
            },
            Command::AddBindings(bindings) => {
                self.bindings.add_bindings(bindings);
            },
            Command::ProxyToBackend(cmd) => {
                action = self.backend.handle(cmd);
            },
        };

        self.sync_and_redraw();
        action
    }

    fn sync_and_redraw(&mut self) {
        self.sync_font();
        self.backend.sync();
        self.redraw();
    }

    fn sync_font(&mut self) {
        self.font.sync();
        self.backend
            .handle(backend::Command::Resize(None, Some(self.font.measure)));
    }

    fn redraw(&mut self) {
        self.cache.clear();
    }
}

#[derive(Clone)]
struct TerminalSubscriptionData {
    id: u64,
    event_receiver: Arc<Mutex<UnboundedReceiver<AlacrittyEvent>>>,
}

impl Hash for TerminalSubscriptionData {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

fn terminal_subscription_stream(
    data: &TerminalSubscriptionData,
) -> BoxStream<'static, Event> {
    let id = data.id;
    let event_receiver = data.event_receiver.clone();
    iced::stream::channel(1000, async move |mut output| {
        let mut shutdown = false;
        let mut throttle = Throttle::default();
        // 受け口はこの stream が生きている間ずっと握る(他に読む者はいない)。
        let mut event_receiver = event_receiver.lock().await;
        loop {
            let deadline = throttle.deadline(wakeup_interval());
            let hold = async {
                match deadline {
                    Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                received = event_receiver.recv() => match received {
                    // 点滅の変化・マウス印の汚れは、この器では起こしと同じ「描き直せ」でしかない
                    // (`backend.rs` の ProcessAlacrittyEvent は Exit / Title / PtyWrite 以外を
                    // 読み捨て、Terminal::handle が sync_and_redraw を掛けるだけ)。Claude Code は
                    // 1 回の描画ごとに DECSCUSR を送るので、起こしだけ間引いても毎秒 5 件残る
                    // (2026-09-05 messages.log 実測)。同じ間引きに乗せ、期限には Wakeup を1件流す。
                    Some(
                        AlacrittyEvent::Wakeup
                        | AlacrittyEvent::CursorBlinkingChange
                        | AlacrittyEvent::MouseCursorDirty,
                    ) => {
                        if !throttle.on_wakeup(Instant::now(), wakeup_interval()) {
                            continue;
                        }
                        send_event(&mut output, id, AlacrittyEvent::Wakeup).await;
                    },
                    Some(event) => {
                        if let AlacrittyEvent::Exit = event {
                            shutdown = true
                        };
                        send_event(&mut output, id, event).await;
                    },
                    None => {
                        if !shutdown {
                            panic!("iced_term stream {}: terminal event channel closed unexpected", id);
                        }
                        // 閉じた受け口を回し続けない(以前は None を空回りで読み続けていた)。
                        std::future::pending::<()>().await;
                    },
                },
                () = hold => {
                    throttle.fire(Instant::now());
                    send_event(&mut output, id, AlacrittyEvent::Wakeup).await;
                },
            }
        }
    })
    .boxed()
}

async fn send_event(
    output: &mut iced::futures::channel::mpsc::Sender<Event>,
    id: u64,
    event: AlacrittyEvent,
) {
    output
        .send(Event::BackendCall(id, backend::Command::ProcessAlacrittyEvent(event)))
        .await
        .unwrap_or_else(|_| {
            panic!("iced_term stream {}: sending BackendEventReceived event is failed", id)
        });
}

#[cfg(test)]
mod throttle_tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_millis(66);

    /// 最初の起こしは即座に流す。
    #[test]
    fn the_first_wakeup_passes_at_once() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        assert!(throttle.on_wakeup(now, INTERVAL));
        assert_eq!(throttle.deadline(INTERVAL), None);
    }

    /// 間隔内に重なった起こしは溜め、期限は前に流した時刻から間隔ぶん先。何件来ても1件。
    #[test]
    fn wakeups_inside_the_interval_are_held_until_the_deadline() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        assert!(throttle.on_wakeup(now, INTERVAL));
        assert!(!throttle.on_wakeup(now + Duration::from_millis(10), INTERVAL));
        assert!(!throttle.on_wakeup(now + Duration::from_millis(20), INTERVAL));
        assert_eq!(throttle.deadline(INTERVAL), Some(now + INTERVAL));
        throttle.fire(now + INTERVAL);
        assert_eq!(throttle.deadline(INTERVAL), None);
        // 流した直後の起こしはまた溜まり、期限は今度の送信から数える。
        assert!(!throttle.on_wakeup(now + INTERVAL + Duration::from_millis(5), INTERVAL));
        assert_eq!(throttle.deadline(INTERVAL), Some(now + INTERVAL + INTERVAL));
    }

    /// 間隔より空いて来た起こしは即座に流す(打った字を待たせない)。
    #[test]
    fn a_wakeup_after_a_quiet_spell_passes_at_once() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        assert!(throttle.on_wakeup(now, INTERVAL));
        assert!(throttle.on_wakeup(now + INTERVAL, INTERVAL));
        assert_eq!(throttle.deadline(INTERVAL), None);
    }

    /// 間引き幅 0 は素通し。
    #[test]
    fn a_zero_interval_passes_everything() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        assert!(throttle.on_wakeup(now, Duration::ZERO));
        assert!(throttle.on_wakeup(now + Duration::from_millis(1), Duration::ZERO));
        assert_eq!(throttle.deadline(Duration::ZERO), None);
    }

    /// 口に置いた幅がそのまま読める。
    #[test]
    fn the_interval_round_trips_through_the_setter() {
        set_wakeup_interval(Duration::from_millis(250));
        assert_eq!(wakeup_interval(), Duration::from_millis(250));
        set_wakeup_interval(Duration::ZERO);
        assert_eq!(wakeup_interval(), Duration::ZERO);
    }
}
