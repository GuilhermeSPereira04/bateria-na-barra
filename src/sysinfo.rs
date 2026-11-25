//! CPU, GPU, RAM and the processes that use them most, sampled once a second
//! on a worker thread while the panel shows the Resources tab.
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
    PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetProcessTimes, GetSystemTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{Owned, w};

#[derive(Clone, Debug, PartialEq)]
pub struct ProcRow {
    pub name: String,
    /// Percent of the whole machine.
    pub cpu: f32,
    /// Bytes.
    pub ram: u64,
}

#[derive(Clone, Default)]
pub struct Stats {
    pub cpu: f32,
    pub gpu: Option<f32>,
    pub ram_used: u64,
    pub ram_total: u64,
    /// From the `Sampler`: every process, summed by name. In the panel view:
    /// only the busiest few, sorted (see `ranked`).
    pub top: Vec<ProcRow>,
}

impl Stats {
    /// A copy for the panel with only the `n` busiest processes.
    pub fn ranked(&self, by_ram: bool, n: usize) -> Stats {
        Stats {
            cpu: self.cpu,
            gpu: self.gpu,
            ram_used: self.ram_used,
            ram_total: self.ram_total,
            top: top(self.top.clone(), by_ram, n),
        }
    }
}

/// Total CPU use between two `GetSystemTimes` readings, as (idle, kernel,
/// user); kernel time includes the idle time.
pub fn cpu_percent(prev: (u64, u64, u64), cur: (u64, u64, u64)) -> f32 {
    let idle = cur.0.saturating_sub(prev.0);
    let total = cur.1.saturating_sub(prev.1) + cur.2.saturating_sub(prev.2);
    if total == 0 {
        return 0.0;
    }
    (100.0 * total.saturating_sub(idle) as f64 / total as f64) as f32
}

/// `exe` without a trailing `.exe` (any case).
fn strip_exe(exe: &str) -> &str {
    let cut = exe.len().saturating_sub(4);
    match exe.get(cut..) {
        Some(ext) if cut > 0 && ext.eq_ignore_ascii_case(".exe") => &exe[..cut],
        _ => exe,
    }
}

/// One row per program, like Task Manager: rows whose names match (ignoring
/// case and `.exe`) are summed, and the idle pseudo-process is dropped.
pub fn aggregate(rows: Vec<ProcRow>) -> Vec<ProcRow> {
    let mut out: Vec<ProcRow> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let name = strip_exe(&row.name);
        let key = name.to_lowercase();
        if key == "idle" || key == "system idle process" {
            continue;
        }
        match index.get(&key) {
            Some(&i) => {
                out[i].cpu += row.cpu;
                out[i].ram += row.ram;
            }
            None => {
                index.insert(key, out.len());
                out.push(ProcRow { name: name.to_string(), cpu: row.cpu, ram: row.ram });
            }
        }
    }
    out
}

/// The `n` busiest rows, by CPU (then RAM) or by RAM (then CPU).
pub fn top(mut rows: Vec<ProcRow>, by_ram: bool, n: usize) -> Vec<ProcRow> {
    rows.sort_by(|a, b| {
        let cpu = b.cpu.total_cmp(&a.cpu);
        let ram = b.ram.cmp(&a.ram);
        if by_ram { ram.then(cpu) } else { cpu.then(ram) }
    });
    rows.truncate(n);
    rows
}

/// GPU use from `\GPU Engine(*)\Utilization Percentage` instances (named
/// `pid_…_engtype_<type>`): values are summed per engine type and the
/// busiest type wins, capped at 100.
pub fn gpu_percent<'a>(engines: impl IntoIterator<Item = (&'a str, f64)>) -> f32 {
    let mut per_type: HashMap<&str, f64> = HashMap::new();
    for (name, value) in engines {
        if let Some(i) = name.find("engtype_") {
            *per_type.entry(&name[i + "engtype_".len()..]).or_default() += value;
        }
    }
    per_type.values().fold(0.0_f64, |a, &b| a.max(b)).min(100.0) as f32
}

fn ticks(t: FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// (idle, kernel, user) summed over every logical core, in 100 ns units.
fn system_times() -> Option<(u64, u64, u64)> {
    let [mut idle, mut kernel, mut user] = [FILETIME::default(); 3];
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    Some((ticks(idle), ticks(kernel), ticks(user)))
}

/// (used, total) physical memory in bytes.
fn memory() -> (u64, u64) {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_err() {
        return (0, 0);
    }
    (m.ullTotalPhys.saturating_sub(m.ullAvailPhys), m.ullTotalPhys)
}

/// (creation time, CPU time, private bytes) of a process; `None` when it
/// cannot be opened or read (access denied, already gone).
fn process_info(pid: u32) -> Option<(u64, u64, u64)> {
    if pid == 0 {
        return None; // System Idle Process
    }
    // Owned closes the handle on every return.
    let process = unsafe { Owned::new(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?) };
    let [mut created, mut exited, mut kernel, mut user] = [FILETIME::default(); 4];
    unsafe { GetProcessTimes(*process, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
    let mut mem = PROCESS_MEMORY_COUNTERS_EX::default();
    unsafe { GetProcessMemoryInfo(*process, (&raw mut mem).cast(), size_of_val(&mem) as u32) }.ok()?;
    Some((ticks(created), ticks(kernel) + ticks(user), mem.PrivateUsage as u64))
}

/// The GPU engine counters. `Owned` calls `PdhCloseQuery` on drop.
struct Gpu {
    query: Owned<PDH_HQUERY>,
    counter: PDH_HCOUNTER,
}

impl Gpu {
    /// `None` without the counter (no GPU, or a Windows without it).
    fn open() -> Option<Self> {
        let mut query = PDH_HQUERY::default();
        if unsafe { PdhOpenQueryW(None, 0, &mut query) } != 0 {
            return None;
        }
        let query = unsafe { Owned::new(query) };
        let mut counter = PDH_HCOUNTER::default();
        let path = w!("\\GPU Engine(*)\\Utilization Percentage");
        if unsafe { PdhAddEnglishCounterW(*query, path, 0, &mut counter) } != 0 {
            return None;
        }
        unsafe { PdhCollectQueryData(*query) }; // the rate needs a first reading
        Some(Self { query, counter })
    }

    fn read(&mut self) -> Option<f32> {
        if unsafe { PdhCollectQueryData(*self.query) } != 0 {
            return None;
        }
        let (mut size, mut count) = (0u32, 0u32);
        // u64 keeps the items aligned; their names point into the same buffer.
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..3 {
            let items = (!buf.is_empty()).then(|| buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>());
            let status = unsafe { PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, items) };
            match status {
                // Engines come and go between calls: ask again with the new size.
                PDH_MORE_DATA => buf.resize((size as usize).div_ceil(8).max(1), 0),
                0 if !buf.is_empty() => {
                    let ptr = buf.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
                    let items = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
                    let engines: Vec<(String, f64)> = items
                        .iter()
                        .filter(|it| matches!(it.FmtValue.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA))
                        .filter_map(|it| {
                            let name = unsafe { it.szName.to_string() }.ok()?;
                            Some((name, unsafe { it.FmtValue.Anonymous.doubleValue }))
                        })
                        .collect();
                    return Some(gpu_percent(engines.iter().map(|(n, v)| (n.as_str(), *v))));
                }
                _ => return None,
            }
        }
        None
    }
}

/// Takes the readings. CPU figures are deltas since the previous `sample`
/// (or since `new`).
pub struct Sampler {
    system: Option<(u64, u64, u64)>,
    /// CPU time per process, keyed by (PID, creation time) so a reused PID
    /// does not inherit the old process's time.
    procs: HashMap<(u32, u64), u64>,
    gpu: Option<Gpu>,
}

impl Sampler {
    /// Takes the first readings the deltas start from.
    pub fn new() -> Self {
        let mut sampler = Self { system: system_times(), procs: HashMap::new(), gpu: Gpu::open() };
        sampler.processes(0);
        sampler
    }

    pub fn sample(&mut self) -> Stats {
        let now = system_times();
        let (cpu, total) = match (self.system, now) {
            (Some(prev), Some(cur)) => (cpu_percent(prev, cur), cur.1.saturating_sub(prev.1) + cur.2.saturating_sub(prev.2)),
            _ => (0.0, 0),
        };
        self.system = now;
        let top = aggregate(self.processes(total));
        let gpu = self.gpu.as_mut().and_then(Gpu::read);
        let (ram_used, ram_total) = memory();
        Stats { cpu, gpu, ram_used, ram_total, top }
    }

    /// Every process it can open. `total` is the system kernel + user time
    /// since the last call; it is summed over every logical core (elapsed
    /// time x cores), so a share of it is a share of the whole machine.
    fn processes(&mut self, total: u64) -> Vec<ProcRow> {
        let mut rows = Vec::new();
        let mut seen = HashMap::with_capacity(self.procs.len());
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else { return rows };
        let snapshot = unsafe { Owned::new(snapshot) };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = unsafe { Process32FirstW(*snapshot, &mut entry) }.is_ok();
        while more {
            if let Some((created, time, ram)) = process_info(entry.th32ProcessID) {
                let key = (entry.th32ProcessID, created);
                let cpu = match self.procs.get(&key) {
                    Some(&before) if total > 0 => (100.0 * time.saturating_sub(before) as f64 / total as f64) as f32,
                    _ => 0.0, // first seen: no delta yet
                };
                seen.insert(key, time);
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                rows.push(ProcRow { name: String::from_utf16_lossy(&entry.szExeFile[..len]), cpu, ram });
            }
            more = unsafe { Process32NextW(*snapshot, &mut entry) }.is_ok();
        }
        self.procs = seen;
        rows
    }
}

/// A `Sampler` on its own thread: a tick costs a process snapshot, an
/// `OpenProcess` per process and a PDH collection, too much for the UI
/// thread. Dropping it ends the thread, which drops the `Sampler`.
pub struct StatsWorker {
    requests: Sender<()>,
    results: Receiver<Stats>,
    pending: bool,
}

impl StatsWorker {
    /// Starts the thread; each sample is announced to `owner` by
    /// `WM_PANEL_ASYNC` (`ASYNC_STATS`).
    pub fn spawn(owner: HWND) -> Self {
        let (requests, request_rx) = mpsc::channel::<()>();
        let (result_tx, results) = mpsc::channel();
        let owner = owner.0 as isize; // HWND is not Send
        std::thread::spawn(move || {
            let mut sampler = Sampler::new();
            while request_rx.recv().is_ok() {
                if result_tx.send(sampler.sample()).is_err() {
                    break;
                }
                let (msg, source) = (crate::panel::WM_PANEL_ASYNC, WPARAM(crate::panel::ASYNC_STATS));
                let _ = unsafe { PostMessageW(Some(HWND(owner as _)), msg, source, LPARAM(0)) };
            }
        });
        Self { requests, results, pending: false }
    }

    /// Asks for a sample, unless the previous one is still being taken.
    pub fn request(&mut self) {
        if !self.pending {
            self.pending = self.requests.send(()).is_ok();
        }
    }

    /// The sample announced by `ASYNC_STATS`, if it arrived.
    pub fn take(&mut self) -> Option<Stats> {
        let stats = self.results.try_recv().ok()?;
        self.pending = false;
        Some(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, cpu: f32, ram: u64) -> ProcRow {
        ProcRow { name: name.into(), cpu, ram }
    }

    #[test]
    fn cpu_percent_from_deltas() {
        // idle +50, kernel +100 (idle included), user +100: 150 of 200 busy.
        assert_eq!(cpu_percent((100, 200, 100), (150, 300, 200)), 75.0);
        assert_eq!(cpu_percent((100, 200, 100), (100, 200, 100)), 0.0);
    }

    #[test]
    fn aggregate_sums_same_name() {
        let rows = aggregate(vec![row("chrome.exe", 10.0, 100), row("Chrome.exe", 5.0, 50)]);
        assert_eq!(rows, vec![row("chrome", 15.0, 150)]);
    }

    #[test]
    fn aggregate_drops_idle() {
        let rows = aggregate(vec![row("Idle", 90.0, 0), row("System Idle Process", 90.0, 0), row("code.exe", 1.0, 10)]);
        assert_eq!(rows, vec![row("code", 1.0, 10)]);
    }

    #[test]
    fn top_by_cpu_then_ram() {
        let rows: Vec<ProcRow> = (0..7).map(|i| row(&format!("p{i}"), i as f32, 100 - i as u64)).collect();
        let by_cpu = top(rows.clone(), false, 5);
        assert_eq!(by_cpu.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["p6", "p5", "p4", "p3", "p2"]);
        let by_ram = top(rows, true, 5);
        assert_eq!(by_ram.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["p0", "p1", "p2", "p3", "p4"]);
    }

    #[test]
    fn gpu_takes_the_busiest_engine_type() {
        let engines = [
            ("pid_1_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 30.0),
            ("pid_2_luid_0x0_0x1_phys_0_eng_0_engtype_3D", 25.0),
            ("pid_2_luid_0x0_0x1_phys_0_eng_5_engtype_VideoDecode", 40.0),
            ("pid_3_luid_0x0_0x1_phys_0_eng_1_engtype_Copy", 5.0),
        ];
        assert_eq!(gpu_percent(engines), 55.0);
        assert_eq!(gpu_percent([("pid_1_engtype_3D", 80.0), ("pid_2_engtype_3D", 70.0)]), 100.0);
        assert_eq!(gpu_percent(Vec::<(&str, f64)>::new()), 0.0);
    }

    #[test]
    #[ignore = "reads the live system"]
    fn sampler_reads_the_system() {
        let mut s = Sampler::new();
        std::thread::sleep(std::time::Duration::from_secs(1));
        s.sample();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let started = std::time::Instant::now();
        let stats = s.sample();
        let took = started.elapsed();
        let sum: f32 = stats.top.iter().map(|r| r.cpu).sum();
        println!("processes sum to {sum:.1}% of the machine");
        assert!((0.0..=100.0).contains(&stats.cpu));
        assert!(stats.ram_total > 0 && stats.ram_used <= stats.ram_total);
        assert!(!stats.top.is_empty());
        println!("sample took {took:?}");
        println!("cpu {:.1}% gpu {:?} ram {}/{} procs {}", stats.cpu, stats.gpu, stats.ram_used, stats.ram_total, stats.top.len());
        for r in top(stats.top, false, 5) {
            println!("  {} {:.1}% {}", r.name, r.cpu, r.ram);
        }
    }
}
