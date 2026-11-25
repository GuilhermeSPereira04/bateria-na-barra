//! Pure timing rules for the top panel: edge-hold trigger and close rule.

use std::time::{Duration, Instant};

/// How long the cursor must rest on the top edge to open the panel.
pub const HOLD_TO_OPEN: Duration = Duration::from_millis(1500);
/// How long the cursor must stay outside the panel before it closes.
pub const LEAVE_GRACE: Duration = Duration::from_millis(500);

/// Fires once when the cursor rests on the top edge for `HOLD_TO_OPEN`.
pub struct EdgeTrigger {
    since: Option<Instant>,
    fired: bool,
}

impl EdgeTrigger {
    pub fn new() -> Self {
        Self { since: None, fired: false }
    }

    /// Returns true exactly once per visit to the edge.
    pub fn update(&mut self, at_top_edge: bool, button_down: bool, now: Instant) -> bool {
        if !at_top_edge {
            self.since = None;
            self.fired = false;
            return false;
        }
        if button_down {
            self.since = None;
            return false;
        }
        if self.fired {
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if now.duration_since(since) >= HOLD_TO_OPEN {
            self.fired = true;
            self.since = None;
            return true;
        }
        false
    }
}

/// Conditions that keep the panel open regardless of the cursor.
#[derive(Clone, Copy, Default)]
pub struct Pauses {
    pub search_text: bool,
    pub speedtest_running: bool,
    pub popup_open: bool,
}

impl Pauses {
    fn any(&self) -> bool {
        self.search_text || self.speedtest_running || self.popup_open
    }
}

/// Decides when the panel should close after the cursor leaves it.
pub struct CloseRule {
    armed: bool,
    outside_since: Option<Instant>,
}

impl CloseRule {
    pub fn new() -> Self {
        Self { armed: false, outside_since: None }
    }

    /// Returns true when the panel should close.
    pub fn update(&mut self, inside: bool, pauses: Pauses, now: Instant) -> bool {
        if inside {
            self.armed = true;
            self.outside_since = None;
            return false;
        }
        if !self.armed || pauses.any() {
            self.outside_since = None;
            return false;
        }
        let since = *self.outside_since.get_or_insert(now);
        now.duration_since(since) >= LEAVE_GRACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn fires_after_hold() {
        let t0 = Instant::now();
        let mut e = EdgeTrigger::new();
        assert!(!e.update(true, false, t0));
        assert!(!e.update(true, false, t0 + ms(1499)));
        assert!(e.update(true, false, t0 + ms(1500)));
    }

    #[test]
    fn fires_once_per_visit() {
        let t0 = Instant::now();
        let mut e = EdgeTrigger::new();
        e.update(true, false, t0);
        assert!(e.update(true, false, t0 + ms(1500)));
        assert!(!e.update(true, false, t0 + ms(3000)));
        assert!(!e.update(false, false, t0 + ms(3100)));
        assert!(!e.update(true, false, t0 + ms(3200)));
        assert!(e.update(true, false, t0 + ms(4700)));
    }

    #[test]
    fn leaving_resets() {
        let t0 = Instant::now();
        let mut e = EdgeTrigger::new();
        e.update(true, false, t0);
        e.update(false, false, t0 + ms(1000));
        e.update(true, false, t0 + ms(1100));
        assert!(!e.update(true, false, t0 + ms(2000)));
        assert!(e.update(true, false, t0 + ms(2600)));
    }

    #[test]
    fn held_button_never_fires() {
        let t0 = Instant::now();
        let mut e = EdgeTrigger::new();
        for s in 0..=5 {
            assert!(!e.update(true, true, t0 + Duration::from_secs(s)));
        }
        assert!(!e.update(true, false, t0 + ms(5000)));
        assert!(e.update(true, false, t0 + ms(6500)));
    }

    #[test]
    fn close_not_armed_before_entering() {
        let t0 = Instant::now();
        let mut c = CloseRule::new();
        for s in 0..=5 {
            assert!(!c.update(false, Pauses::default(), t0 + Duration::from_secs(s)));
        }
    }

    #[test]
    fn close_after_grace() {
        let t0 = Instant::now();
        let mut c = CloseRule::new();
        assert!(!c.update(true, Pauses::default(), t0));
        assert!(!c.update(false, Pauses::default(), t0 + ms(1000)));
        assert!(!c.update(false, Pauses::default(), t0 + ms(1400)));
        assert!(c.update(false, Pauses::default(), t0 + ms(1500)));
    }

    #[test]
    fn reentering_cancels_close() {
        let t0 = Instant::now();
        let mut c = CloseRule::new();
        c.update(true, Pauses::default(), t0);
        c.update(false, Pauses::default(), t0 + ms(1000));
        c.update(true, Pauses::default(), t0 + ms(1300));
        c.update(false, Pauses::default(), t0 + ms(2000));
        assert!(!c.update(false, Pauses::default(), t0 + ms(2400)));
    }

    #[test]
    fn pauses_block_close() {
        let t0 = Instant::now();
        let mut c = CloseRule::new();
        let p = Pauses { search_text: true, ..Pauses::default() };
        c.update(true, Pauses::default(), t0);
        assert!(!c.update(false, p, t0 + ms(1000)));
        assert!(!c.update(false, p, t0 + ms(4000)));
        assert!(!c.update(false, Pauses::default(), t0 + ms(4100)));
        assert!(!c.update(false, Pauses::default(), t0 + ms(4500)));
        assert!(c.update(false, Pauses::default(), t0 + ms(4600)));
    }
}
