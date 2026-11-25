//! Battery readers, one per device. Each `read_once` is blocking and runs on
//! its own background thread (see `spawn_all`).
pub mod adb;
pub mod dualsense;
pub mod zaopin;

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::model::{DeviceKind, ReadOutcome};

/// Posted to the UI window whenever a reader has sent a new outcome.
pub const WM_READING: u32 = WM_APP + 1;

fn interval(kind: DeviceKind) -> Duration {
    match kind {
        DeviceKind::Mouse | DeviceKind::Controller => Duration::from_secs(60),
        DeviceKind::Phone => Duration::from_secs(180),
    }
}

/// HID readers keep one `HidApi` per thread, created lazily and retried on failure.
fn hid_read(api: &mut Option<hidapi::HidApi>, read: fn(&mut hidapi::HidApi) -> ReadOutcome) -> ReadOutcome {
    if api.is_none() {
        *api = hidapi::HidApi::new().ok();
    }
    match api {
        Some(api) => read(api),
        None => ReadOutcome::Failed,
    }
}

/// Starts one thread per device. Each reads, sends `(kind, outcome)` on `tx`,
/// wakes the UI with `WM_READING`, then waits for its interval or for a
/// "refresh now" signal on the returned sender (same order as `model::ALL`).
pub fn spawn_all(hwnd_ui: isize, tx: Sender<(DeviceKind, ReadOutcome)>) -> Vec<Sender<()>> {
    crate::model::ALL
        .iter()
        .map(|&kind| {
            let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut api: Option<hidapi::HidApi> = None;
                // Mouse only: show what the official Zaopin app shows.
                let mut smoother = zaopin::Smoother::new();
                let mut last = Instant::now();
                loop {
                    let outcome = match kind {
                        DeviceKind::Mouse => match hid_read(&mut api, zaopin::read_once) {
                            ReadOutcome::Ok(raw) => {
                                let now = Instant::now();
                                let shown = smoother.update(raw, now - last);
                                last = now;
                                ReadOutcome::Ok(shown)
                            }
                            other => other,
                        },
                        DeviceKind::Controller => hid_read(&mut api, dualsense::read_once),
                        DeviceKind::Phone => adb::read_once(),
                    };
                    if tx.send((kind, outcome)).is_err() {
                        return;
                    }
                    unsafe {
                        let _ = PostMessageW(Some(HWND(hwnd_ui as _)), WM_READING, WPARAM(0), LPARAM(0));
                    }
                    match refresh_rx.recv_timeout(interval(kind)) {
                        Ok(()) => while refresh_rx.try_recv().is_ok() {}, // coalesce repeated clicks
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            });
            refresh_tx
        })
        .collect()
}
