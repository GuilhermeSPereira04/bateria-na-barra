//! Speed test state. Stub: Task 11 runs the test.
#![allow(dead_code)] // filled by Task 11

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Ping,
    Download,
    Upload,
}

#[derive(Clone, Default)]
pub enum SpeedState {
    #[default]
    Idle,
    Running { phase: Phase, ping_ms: Option<u32>, down: Option<f32>, up: Option<f32> },
    /// `at` is the local time (hh, mm) the test finished.
    Done { ping_ms: u32, down: f32, up: f32, at: (u16, u16) },
    Error(String),
}
