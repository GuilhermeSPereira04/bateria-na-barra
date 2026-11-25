//! CPU, GPU, RAM and top processes. Stub: Task 9 samples them.
#![allow(dead_code)] // filled by Task 9

pub struct ProcRow {
    pub name: String,
    /// Percent of the whole machine.
    pub cpu: f32,
    /// Bytes.
    pub ram: u64,
}

#[derive(Default)]
pub struct Stats {
    pub cpu: f32,
    pub gpu: Option<f32>,
    pub ram_used: u64,
    pub ram_total: u64,
    pub top: Vec<ProcRow>,
}
