//! Android phone battery over wireless ADB (`adb shell dumpsys battery`).
#![allow(dead_code)]

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::model::{Charge, ReadOutcome, Reading};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ADB_TIMEOUT: Duration = Duration::from_secs(15);

pub fn parse_dumpsys(text: &str) -> Option<Reading> {
    let field = |key: &str| -> Option<u32> {
        text.lines().find_map(|line| {
            let (k, v) = line.trim().split_once(':')?;
            (k.trim() == key).then(|| v.trim().parse().ok()).flatten()
        })
    };
    let level = field("level")?;
    let scale = field("scale").filter(|&s| s > 0).unwrap_or(100);
    let percent = (level * 100 / scale).min(100) as u8;
    let charge = match field("status") {
        Some(2) => Charge::Charging,
        Some(5) => Charge::Full,
        _ => Charge::Discharging,
    };
    Some(Reading { percent, charge })
}

pub fn parse_ready_serials(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            (parts.next()? == "device").then(|| serial.to_string())
        })
        .collect()
}

pub fn parse_mdns_connect_targets(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            match parts.as_slice() {
                [_, service, target, ..] if service.starts_with("_adb-tls-connect") => {
                    Some(target.to_string())
                }
                _ => None,
            }
        })
        .collect()
}

/// `adb.exe` next to our executable if present, else whatever `adb` is on PATH.
pub fn adb_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("adb.exe")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("adb"))
}

/// How an adb invocation ended.
#[derive(Debug, PartialEq)]
pub enum AdbRun {
    Done(String),
    /// Hit the deadline and was killed (e.g. the phone left the Wi-Fi mid-call).
    TimedOut,
    /// Could not start, or exited with an error.
    Failed,
}

impl AdbRun {
    fn output(self) -> Option<String> {
        match self {
            AdbRun::Done(out) => Some(out),
            _ => None,
        }
    }
}

/// `dumpsys battery` result → outcome. A timeout means the phone is gone,
/// so it counts as disconnected (hidden after the hold), not as "sem leitura".
pub fn shell_outcome(run: AdbRun) -> ReadOutcome {
    match run {
        AdbRun::Done(out) => parse_dumpsys(&out).map_or(ReadOutcome::Failed, ReadOutcome::Ok),
        AdbRun::TimedOut => ReadOutcome::Offline,
        AdbRun::Failed => ReadOutcome::Failed,
    }
}

/// Runs adb without a console window; gives up (and kills it) after 15 s.
fn run_adb(args: &[&str]) -> AdbRun {
    let mut cmd = Command::new(adb_path());
    cmd.args(args);
    run_with_timeout(cmd, ADB_TIMEOUT)
}

/// Runs `cmd` hidden and returns its stdout if it exits successfully within `timeout`.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> AdbRun {
    let Ok(mut child) = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    else {
        return AdbRun::Failed;
    };
    // Drain stdout on another thread so a chatty adb can't block on a full pipe.
    // A grandchild (the adb server on a cold start) may keep the pipe open
    // forever, so the deadline below also bounds waiting for this output.
    let Some(mut stdout) = child.stdout.take() else {
        return AdbRun::Failed;
    };
    let (out_tx, out_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        let _ = out_tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return AdbRun::TimedOut;
            }
        }
    };
    if !status.success() {
        return AdbRun::Failed;
    }
    match out_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(out) => AdbRun::Done(out),
        Err(_) => AdbRun::TimedOut,
    }
}

/// Starts the adb server detached from any pipe of ours (all stdio null), so it
/// can't hold our stdout open on later calls.
fn ensure_server() {
    let mut cmd = Command::new(adb_path());
    cmd.arg("start-server")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);
    if let Ok(mut child) = cmd.spawn() {
        let deadline = Instant::now() + ADB_TIMEOUT;
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn ready_serials() -> Option<Vec<String>> {
    run_adb(&["devices"]).output().map(|out| parse_ready_serials(&out))
}

pub fn read_once() -> ReadOutcome {
    ensure_server();
    let Some(mut serials) = ready_serials() else {
        return ReadOutcome::Offline; // adb missing or not responding
    };
    if serials.is_empty() {
        // Wireless debugging changes port when the phone restarts: rediscover it.
        let targets = run_adb(&["mdns", "services"])
            .output()
            .map(|out| parse_mdns_connect_targets(&out))
            .unwrap_or_default();
        for target in &targets {
            let _ = run_adb(&["connect", target]);
        }
        serials = ready_serials().unwrap_or_default();
    }
    let Some(serial) = serials.first() else {
        return ReadOutcome::Offline;
    };
    let run = run_adb(&["-s", serial, "shell", "dumpsys", "battery"]);
    if run == AdbRun::TimedOut {
        // Drop the dead connection so the next cycle rediscovers the phone
        // instead of waiting 15 s on it again.
        let _ = run_adb(&["disconnect", serial]);
    }
    shell_outcome(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_bounds_output_held_by_grandchild() {
        // `start /b` leaves ping running with our stdout pipe after cmd exits,
        // just like the adb server does on a cold start.
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "/b", "ping", "-n", "15", "127.0.0.1"]);
        let t = Instant::now();
        let _ = run_with_timeout(cmd, Duration::from_secs(2));
        assert!(t.elapsed() < Duration::from_secs(5), "took {:?}", t.elapsed());
    }

    #[test]
    fn shell_timeout_means_phone_left() {
        assert_eq!(shell_outcome(AdbRun::TimedOut), ReadOutcome::Offline);
    }

    #[test]
    fn shell_error_or_garbage_is_failed() {
        assert_eq!(shell_outcome(AdbRun::Failed), ReadOutcome::Failed);
        assert_eq!(shell_outcome(AdbRun::Done("error: closed".into())), ReadOutcome::Failed);
    }

    #[test]
    fn shell_dumpsys_is_ok() {
        let out = AdbRun::Done("  status: 3
  level: 42
".into());
        assert_eq!(shell_outcome(out), ReadOutcome::Ok(Reading { percent: 42, charge: Charge::Discharging }));
    }

    #[test]
    fn dumpsys_charging() {
        let text = "Current Battery Service state:\n  AC powered: false\n  USB powered: true\n  status: 2\n  level: 81\n  scale: 100\n";
        assert_eq!(parse_dumpsys(text), Some(Reading { percent: 81, charge: Charge::Charging }));
    }

    #[test]
    fn dumpsys_full() {
        let text = "Current Battery Service state:\n  status: 5\n  level: 100\n  scale: 100\n";
        assert_eq!(parse_dumpsys(text), Some(Reading { percent: 100, charge: Charge::Full }));
    }

    #[test]
    fn dumpsys_discharging() {
        let text = "Current Battery Service state:\r\n  status: 3\r\n  level: 42\r\n  scale: 100\r\n";
        assert_eq!(parse_dumpsys(text), Some(Reading { percent: 42, charge: Charge::Discharging }));
    }

    #[test]
    fn dumpsys_scale_other_than_100() {
        let text = "  status: 3\n  level: 25\n  scale: 50\n";
        assert_eq!(parse_dumpsys(text), Some(Reading { percent: 50, charge: Charge::Discharging }));
    }

    #[test]
    fn dumpsys_ignores_nested_level_keys() {
        // Some ROMs print extra keys like "Max charging voltage" or "  mod level: 0"; only the exact key counts.
        let text = "  status: 3\n  battery level estimate: 7\n  level: 64\n";
        assert_eq!(parse_dumpsys(text), Some(Reading { percent: 64, charge: Charge::Discharging }));
    }

    #[test]
    fn dumpsys_garbage() {
        assert_eq!(parse_dumpsys("error: device offline"), None);
    }

    #[test]
    fn devices_filters_states() {
        let text = "List of devices attached\n192.168.0.5:37543\tdevice\nabc\tunauthorized\nadb-R5C-xyz._adb-tls-connect._tcp\tdevice\n\n";
        assert_eq!(
            parse_ready_serials(text),
            vec!["192.168.0.5:37543".to_string(), "adb-R5C-xyz._adb-tls-connect._tcp".to_string()]
        );
    }

    #[test]
    fn mdns_targets() {
        let text = "List of discovered mdns services\nadb-R5C-xyz\t_adb-tls-connect._tcp\t192.168.0.5:37543\nadb-R5C-xyz\t_adb-tls-pairing._tcp\t192.168.0.5:41111\n";
        assert_eq!(parse_mdns_connect_targets(text), vec!["192.168.0.5:37543".to_string()]);
    }
}

#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn adb_once() {
        let t = std::time::Instant::now();
        println!("read_once = {:?} in {:?}", super::read_once(), t.elapsed());
    }
}
