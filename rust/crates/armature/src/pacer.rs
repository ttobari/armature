//! 描き直しの間合い。頁を読んでいる(ブラウザが中央の前面にいる)間は、
//! 明滅の刻みと端末の起こしを間引く——どちらも窓全体の描き直しを1回ずつ連れてくるので、
//! 見えていない端末のために頁の描画を削らない。

use std::time::Duration;

/// 頁を読んでいる(ブラウザが中央の前面にいる)間の印。`Cockpit::subscription` が
/// 毎回の用事の後に立て直す。
static READING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_reading(reading: bool) {
    READING.store(reading, std::sync::atomic::Ordering::Relaxed);
}

#[must_use]
pub fn reading() -> bool {
    READING.load(std::sync::atomic::Ordering::Relaxed)
}

/// 明滅の刻み。セッション一覧(`panels::Sessions`)の回転の印がこの刻みで進む。
/// 平時は 125ms、頁を読んでいる間は 250ms。
#[must_use]
pub fn ornament_interval() -> Duration {
    ornament_interval_for(reading())
}

#[must_use]
pub fn ornament_interval_for(reading: bool) -> Duration {
    if reading {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(125)
    }
}

/// 端末の起こし(alacritty の Wakeup)を流す間隔の上限。Claude Code の画面は
/// 秒に 30 回書き換わるので、そのまま通すと窓全体が 30fps で描き直る。平時は 66ms
/// (15fps・打った字は前の起こしから 66ms 以上空いていれば即)。頁を読んでいる間は
/// 端末が頁の裏に隠れているので 250ms。
#[must_use]
pub fn terminal_wakeup_interval() -> Duration {
    terminal_wakeup_interval_for(reading())
}

#[must_use]
pub fn terminal_wakeup_interval_for(reading: bool) -> Duration {
    if reading {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(66)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 明滅は平時 8fps・読んでいる間 4fps。
    #[test]
    fn the_blink_slows_down_while_reading() {
        assert_eq!(ornament_interval_for(false), Duration::from_millis(125));
        assert_eq!(ornament_interval_for(true), Duration::from_millis(250));
    }

    /// 端末の起こしは平時 15fps まで、読んでいる間は明滅と同じ 4fps まで。
    #[test]
    fn terminal_wakeups_slow_down_while_reading() {
        assert_eq!(terminal_wakeup_interval_for(false), Duration::from_millis(66));
        assert_eq!(terminal_wakeup_interval_for(true), ornament_interval_for(true));
        set_reading(true);
        assert_eq!(terminal_wakeup_interval(), Duration::from_millis(250));
        set_reading(false);
        assert_eq!(terminal_wakeup_interval(), Duration::from_millis(66));
    }
}
