//! Device state: what each reader reported and what the strip should show.
#![allow(dead_code)]

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DeviceKind {
    Mouse,
    Controller,
    Phone,
}

/// Strip order: mouse, controller, phone.
pub const ALL: [DeviceKind; 3] = [DeviceKind::Mouse, DeviceKind::Controller, DeviceKind::Phone];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charge {
    Discharging,
    Charging,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reading {
    pub percent: u8,
    pub charge: Charge,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadOutcome {
    Ok(Reading),
    /// Device is there but its battery could not be read.
    Failed,
    /// Device not found.
    Offline,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ItemView {
    Value(Reading),
    Dash,
    Hidden,
}

/// How long a good reading keeps being shown after reads stop succeeding.
pub const HOLD: Duration = Duration::from_secs(600);

pub fn device_label(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Mouse => "Mouse Zaopin",
        DeviceKind::Controller => "Controle PS5",
        DeviceKind::Phone => "Celular",
    }
}

pub fn is_low(r: &Reading) -> bool {
    r.percent < 20
}

#[derive(Default)]
struct Slot {
    last_good: Option<(Reading, Instant)>,
    last_outcome: Option<ReadOutcome>,
}

#[derive(Default)]
pub struct Tracker {
    slots: HashMap<DeviceKind, Slot>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, kind: DeviceKind, outcome: ReadOutcome, now: Instant) {
        let slot = self.slots.entry(kind).or_default();
        if let ReadOutcome::Ok(reading) = outcome {
            slot.last_good = Some((reading, now));
        }
        slot.last_outcome = Some(outcome);
    }

    pub fn view(&self, kind: DeviceKind, now: Instant) -> ItemView {
        let Some(slot) = self.slots.get(&kind) else {
            return ItemView::Hidden;
        };
        if let Some(ReadOutcome::Ok(reading)) = slot.last_outcome {
            return ItemView::Value(reading);
        }
        if let Some((reading, at)) = slot.last_good
            && now.saturating_duration_since(at) <= HOLD
        {
            return ItemView::Value(reading);
        }
        match slot.last_outcome {
            Some(ReadOutcome::Failed) => ItemView::Dash,
            _ => ItemView::Hidden,
        }
    }

    pub fn tooltip(&self, now: Instant) -> String {
        ALL.iter()
            .map(|&kind| {
                let state = match self.view(kind, now) {
                    ItemView::Value(r) => match r.charge {
                        Charge::Discharging => format!("{}%", r.percent),
                        Charge::Charging => format!("{}% (carregando)", r.percent),
                        Charge::Full => format!("{}% (carregado)", r.percent),
                    },
                    ItemView::Dash => "sem leitura".to_string(),
                    ItemView::Hidden => "desconectado".to_string(),
                };
                format!("{}: {}", device_label(kind), state)
            })
            .collect::<Vec<_>>()
            .join("
")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn r(percent: u8, charge: Charge) -> Reading {
        Reading { percent, charge }
    }

    #[test]
    fn ok_shows_value() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Mouse, ReadOutcome::Ok(r(50, Charge::Discharging)), t0);
        assert_eq!(t.view(DeviceKind::Mouse, t0), ItemView::Value(r(50, Charge::Discharging)));
    }

    #[test]
    fn failed_after_ok_holds_value() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Phone, ReadOutcome::Ok(r(50, Charge::Discharging)), t0);
        t.update(DeviceKind::Phone, ReadOutcome::Failed, t0 + 5 * MIN);
        assert_eq!(t.view(DeviceKind::Phone, t0 + 5 * MIN), ItemView::Value(r(50, Charge::Discharging)));
    }

    #[test]
    fn failed_after_hold_shows_dash() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Phone, ReadOutcome::Ok(r(50, Charge::Discharging)), t0);
        t.update(DeviceKind::Phone, ReadOutcome::Failed, t0 + 11 * MIN);
        assert_eq!(t.view(DeviceKind::Phone, t0 + 11 * MIN), ItemView::Dash);
    }

    #[test]
    fn offline_after_hold_hides() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Phone, ReadOutcome::Ok(r(50, Charge::Discharging)), t0);
        t.update(DeviceKind::Phone, ReadOutcome::Offline, t0 + 11 * MIN);
        assert_eq!(t.view(DeviceKind::Phone, t0 + 11 * MIN), ItemView::Hidden);
    }

    #[test]
    fn offline_within_hold_keeps_value() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Phone, ReadOutcome::Ok(r(50, Charge::Discharging)), t0);
        t.update(DeviceKind::Phone, ReadOutcome::Offline, t0 + MIN);
        assert_eq!(t.view(DeviceKind::Phone, t0 + MIN), ItemView::Value(r(50, Charge::Discharging)));
    }

    #[test]
    fn held_value_expires_without_new_reads() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Phone, ReadOutcome::Ok(r(50, Charge::Discharging)), t0);
        t.update(DeviceKind::Phone, ReadOutcome::Offline, t0 + MIN);
        assert_eq!(t.view(DeviceKind::Phone, t0 + 11 * MIN), ItemView::Hidden);
    }

    #[test]
    fn failed_without_any_good_is_dash() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Mouse, ReadOutcome::Failed, t0);
        assert_eq!(t.view(DeviceKind::Mouse, t0), ItemView::Dash);
    }

    #[test]
    fn nothing_yet_is_hidden() {
        let t = Tracker::new();
        assert_eq!(t.view(DeviceKind::Controller, Instant::now()), ItemView::Hidden);
    }

    #[test]
    fn tooltip_lists_all() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Mouse, ReadOutcome::Failed, t0);
        t.update(DeviceKind::Controller, ReadOutcome::Ok(r(64, Charge::Charging)), t0);
        assert_eq!(
            t.tooltip(t0),
            "Mouse Zaopin: sem leitura\nControle PS5: 64% (carregando)\nCelular: desconectado"
        );
    }

    #[test]
    fn tooltip_full_and_plain() {
        let t0 = Instant::now();
        let mut t = Tracker::new();
        t.update(DeviceKind::Mouse, ReadOutcome::Ok(r(93, Charge::Discharging)), t0);
        t.update(DeviceKind::Phone, ReadOutcome::Ok(r(100, Charge::Full)), t0);
        assert_eq!(
            t.tooltip(t0),
            "Mouse Zaopin: 93%\nControle PS5: desconectado\nCelular: 100% (carregado)"
        );
    }

    #[test]
    fn low_threshold() {
        assert!(is_low(&r(19, Charge::Discharging)));
        assert!(!is_low(&r(20, Charge::Discharging)));
    }
}
