//! Speed test against Cloudflare's public endpoints: ping, download and
//! upload, over WinHTTP on a worker thread.
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, PCWSTR, w};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Ping,
    Download,
    Upload,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub enum SpeedState {
    #[default]
    Idle,
    Running { phase: Phase, ping_ms: Option<u32>, down: Option<f32>, up: Option<f32> },
    /// `at` is the local time (hh, mm) the test finished.
    Done { ping_ms: u32, down: f32, up: f32, at: (u16, u16) },
    Error(String),
}

/// The HTTP calls the test needs. The callbacks get the bytes moved since
/// the last call and return `false` to abort.
pub trait Http {
    fn get(&mut self, path: &str, sink: &mut dyn FnMut(usize) -> bool) -> Result<(), ()>;
    fn post(&mut self, path: &str, bytes: usize, tick: &mut dyn FnMut(usize) -> bool) -> Result<(), ()>;
}

const HOST: PCWSTR = w!("speed.cloudflare.com");
const PING_PATH: &str = "/__down?bytes=0";
const DOWN_PATH: &str = "/__down?bytes=25000000";
const UP_PATH: &str = "/__up";
const UP_BYTES: usize = 10_000_000;
const PINGS: usize = 5;
/// Read/write block: cancellation is checked after each one.
const BLOCK: usize = 64 * 1024;
const REPORT_EVERY: Duration = Duration::from_millis(250);
const OFFLINE: &str = "Sem conexão";

/// Megabits per second for `bytes` moved in `elapsed` (0 for no time).
pub fn mbps(bytes: u64, elapsed: Duration) -> f32 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 { 0.0 } else { (bytes as f64 * 8.0 / 1e6 / secs) as f32 }
}

/// Runs the whole test on `client`, reporting progress and the outcome.
/// Returns without a final report once `cancel` is set.
pub fn run_with(client: &mut dyn Http, cancel: &AtomicBool, report: &mut dyn FnMut(SpeedState)) {
    let stopped = || cancel.load(Ordering::Relaxed);
    let running = |phase, ping_ms, down, up| SpeedState::Running { phase, ping_ms, down, up };
    report(running(Phase::Ping, None, None, None));
    let mut pings = Vec::with_capacity(PINGS);
    for _ in 0..PINGS {
        let start = Instant::now();
        let result = client.get(PING_PATH, &mut |_| !stopped());
        if stopped() {
            return;
        }
        if result.is_err() {
            return report(SpeedState::Error(OFFLINE.into()));
        }
        pings.push(start.elapsed());
    }
    pings.sort();
    let ping_ms = u32::try_from(pings[PINGS / 2].as_millis()).unwrap_or(u32::MAX);

    report(running(Phase::Download, Some(ping_ms), None, None));
    let mut shown = |v| report(running(Phase::Download, Some(ping_ms), Some(v), None));
    let down = match transfer(cancel, &mut shown, |sink| client.get(DOWN_PATH, sink)) {
        None => return,
        Some(Err(())) => return report(SpeedState::Error(OFFLINE.into())),
        Some(Ok(v)) => v,
    };

    report(running(Phase::Upload, Some(ping_ms), Some(down), None));
    let mut shown = |v| report(running(Phase::Upload, Some(ping_ms), Some(down), Some(v)));
    let up = match transfer(cancel, &mut shown, |tick| client.post(UP_PATH, UP_BYTES, tick)) {
        None => return,
        Some(Err(())) => return report(SpeedState::Error(OFFLINE.into())),
        Some(Ok(v)) => v,
    };

    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    report(SpeedState::Done { ping_ms, down, up, at: (t.wHour, t.wMinute) });
}

/// Times one transfer, passing its speed so far to `progress` every
/// `REPORT_EVERY`. `None` when cancelled, else the final speed.
fn transfer(
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
    call: impl FnOnce(&mut dyn FnMut(usize) -> bool) -> Result<(), ()>,
) -> Option<Result<f32, ()>> {
    let start = Instant::now();
    let mut last = start;
    let mut bytes = 0u64;
    let result = call(&mut |n| {
        bytes += n as u64;
        let now = Instant::now();
        if now - last >= REPORT_EVERY {
            last = now;
            progress(mbps(bytes, now - start));
        }
        !cancel.load(Ordering::Relaxed)
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(result.map(|()| mbps(bytes, start.elapsed())))
}

/// Runs the test on a new thread. Every report goes to `tx` and is announced
/// to `hwnd_owner` by `WM_PANEL_ASYNC` (`ASYNC_SPEED`). Setting `cancel`
/// ends the thread within a block.
pub fn start(hwnd_owner: isize, cancel: Arc<AtomicBool>, tx: Sender<SpeedState>) {
    std::thread::spawn(move || {
        let mut report = |state| {
            if tx.send(state).is_ok() {
                let (msg, source) = (crate::panel::WM_PANEL_ASYNC, WPARAM(crate::panel::ASYNC_SPEED));
                let _ = unsafe { PostMessageW(Some(HWND(hwnd_owner as _)), msg, source, LPARAM(0)) };
            }
        };
        match WinHttp::open() {
            Ok(mut client) => run_with(&mut client, &cancel, &mut report),
            Err(()) => report(SpeedState::Error(OFFLINE.into())),
        }
    });
}

/// A WinHTTP handle, closed on drop.
struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> Result<Self, ()> {
        if raw.is_null() { Err(()) } else { Ok(Handle(raw)) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// Synchronous HTTPS to `HOST`, one request handle per call.
struct WinHttp {
    // Fields drop in order: the connection closes before the session.
    connect: Handle,
    _session: Handle,
}

impl WinHttp {
    fn open() -> Result<Self, ()> {
        let session = Handle::new(unsafe {
            WinHttpOpen(w!("battery-bar"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0)
        })?;
        // Resolve, connect, send, receive (ms): a dead link fails instead of hanging.
        let _ = unsafe { WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 15_000) };
        let connect = Handle::new(unsafe { WinHttpConnect(session.0, HOST, 443, 0) })?;
        Ok(WinHttp { connect, _session: session })
    }

    fn request(&self, verb: PCWSTR, path: &str) -> Result<Handle, ()> {
        let path = HSTRING::from(path);
        let accept = std::ptr::null(); // WINHTTP_DEFAULT_ACCEPT_TYPES
        Handle::new(unsafe {
            WinHttpOpenRequest(self.connect.0, verb, &path, PCWSTR::null(), PCWSTR::null(), accept, WINHTTP_FLAG_SECURE)
        })
    }
}

/// Waits for the response and accepts only a 2xx status.
fn response_ok(req: &Handle) -> Result<(), ()> {
    unsafe { WinHttpReceiveResponse(req.0, std::ptr::null_mut()) }.map_err(|_| ())?;
    let mut code = 0u32;
    let mut len = size_of::<u32>() as u32;
    let level = WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER;
    let buf = Some((&raw mut code).cast::<c_void>());
    unsafe { WinHttpQueryHeaders(req.0, level, PCWSTR::null(), buf, &mut len, std::ptr::null_mut()) }.map_err(|_| ())?;
    if (200..300).contains(&code) { Ok(()) } else { Err(()) }
}

impl Http for WinHttp {
    fn get(&mut self, path: &str, sink: &mut dyn FnMut(usize) -> bool) -> Result<(), ()> {
        let req = self.request(w!("GET"), path)?;
        unsafe { WinHttpSendRequest(req.0, None, None, 0, 0, 0) }.map_err(|_| ())?;
        response_ok(&req)?;
        let mut buf = vec![0u8; BLOCK];
        loop {
            let mut read = 0u32;
            unsafe { WinHttpReadData(req.0, buf.as_mut_ptr().cast(), BLOCK as u32, &mut read) }.map_err(|_| ())?;
            if read == 0 {
                return Ok(());
            }
            if !sink(read as usize) {
                return Err(());
            }
        }
    }

    fn post(&mut self, path: &str, bytes: usize, tick: &mut dyn FnMut(usize) -> bool) -> Result<(), ()> {
        let req = self.request(w!("POST"), path)?;
        let total = u32::try_from(bytes).map_err(|_| ())?;
        unsafe { WinHttpSendRequest(req.0, None, None, 0, total, 0) }.map_err(|_| ())?;
        let buf = vec![0u8; BLOCK];
        let mut left = bytes;
        while left > 0 {
            let mut written = 0u32;
            let n = left.min(BLOCK) as u32;
            unsafe { WinHttpWriteData(req.0, Some(buf.as_ptr().cast()), n, &mut written) }.map_err(|_| ())?;
            if written == 0 {
                return Err(());
            }
            left = left.saturating_sub(written as usize);
            if !tick(written as usize) {
                return Err(());
            }
        }
        response_ok(&req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Delivers 64 KB per callback: `blocks` of them per call, or without end
    /// when `None`. `offline` fails the first call; `cancel_after` sets the
    /// flag once that many blocks were delivered.
    struct Fake<'a> {
        offline: bool,
        blocks: Option<usize>,
        calls: usize,
        delivered: usize,
        cancel_after: Option<(usize, &'a AtomicBool)>,
    }

    impl Fake<'_> {
        fn new(blocks: Option<usize>) -> Self {
            Fake { offline: false, blocks, calls: 0, delivered: 0, cancel_after: None }
        }

        fn feed(&mut self, cb: &mut dyn FnMut(usize) -> bool) -> Result<(), ()> {
            self.calls += 1;
            if self.offline {
                return Err(());
            }
            // Some time passes, so the measured speed is finite and non-zero.
            std::thread::sleep(Duration::from_millis(1));
            let mut i = 0;
            while self.blocks.is_none_or(|n| i < n) {
                i += 1;
                self.delivered += 1;
                if let Some((n, flag)) = self.cancel_after
                    && self.delivered == n
                {
                    flag.store(true, Ordering::Relaxed);
                }
                if !cb(65_536) {
                    return Err(());
                }
                assert!(self.delivered < 10_000, "kept going after cancel");
            }
            Ok(())
        }
    }

    impl Http for Fake<'_> {
        fn get(&mut self, _path: &str, sink: &mut dyn FnMut(usize) -> bool) -> Result<(), ()> {
            self.feed(sink)
        }
        fn post(&mut self, _path: &str, _bytes: usize, tick: &mut dyn FnMut(usize) -> bool) -> Result<(), ()> {
            self.feed(tick)
        }
    }

    fn run(client: &mut Fake, cancel: &AtomicBool) -> Vec<SpeedState> {
        let mut reports = Vec::new();
        run_with(client, cancel, &mut |s| reports.push(s));
        reports
    }

    #[test]
    fn mbps_of_one_second() {
        assert_eq!(mbps(12_500_000, Duration::from_secs(1)), 100.0);
        assert_eq!(mbps(12_500_000, Duration::ZERO), 0.0);
    }

    #[test]
    fn offline_reports_error() {
        let cancel = AtomicBool::new(false);
        let mut fake = Fake { offline: true, ..Fake::new(Some(1)) };
        let reports = run(&mut fake, &cancel);
        assert_eq!(reports.last(), Some(&SpeedState::Error("Sem conexão".into())));
        assert_eq!(fake.calls, 1);
    }

    #[test]
    fn cancel_stops_quickly() {
        let cancel = AtomicBool::new(false);
        let mut fake = Fake { cancel_after: Some((3, &cancel)), ..Fake::new(None) };
        let reports = run(&mut fake, &cancel);
        assert!(!matches!(reports.last(), Some(SpeedState::Done { .. } | SpeedState::Error(_))));
        assert_eq!(fake.delivered, 3, "a block delivered after cancel");
    }

    #[test]
    fn full_run_reports_done() {
        let cancel = AtomicBool::new(false);
        let mut fake = Fake::new(Some(4));
        let reports = run(&mut fake, &cancel);
        let Some(SpeedState::Done { down, up, .. }) = reports.last() else { panic!("last report not Done") };
        assert!(*down > 0.0 && *up > 0.0, "down {down} up {up}");
        assert!(reports.iter().any(|s| matches!(s, SpeedState::Running { phase: Phase::Download, .. })));
        assert_eq!(fake.calls, 7, "5 pings, a download and an upload");
    }

    /// One tiny real request (no payload) to check the WinHTTP plumbing.
    /// Run by hand only: `cargo test winhttp_ping_once -- --ignored`.
    #[test]
    #[ignore]
    fn winhttp_ping_once() {
        let Ok(mut client) = WinHttp::open() else { panic!("WinHttpOpen failed") };
        assert_eq!(client.get(PING_PATH, &mut |_| true), Ok(()));
    }
}
