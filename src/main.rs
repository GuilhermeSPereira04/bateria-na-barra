#![windows_subsystem = "windows"]

mod app;
mod apps;
mod audio;
mod calendar;
mod config;
mod media;
mod model;
mod panel;
mod radios;
mod readers;
mod render;
mod search;
mod speedtest;
mod startup;
mod sysinfo;
mod taskbar;
mod theme;
mod tray;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};
use windows::core::{PCWSTR, w};

/// `--painel abrir` / `--painel fechar`: `Some(open)`; anything else `None`.
pub fn parse_panel_arg(args: &[String]) -> Option<bool> {
    match args {
        [flag, value] if flag == "--painel" => match value.as_str() {
            "abrir" => Some(true),
            "fechar" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn main() {
    unsafe {
        // Single instance: a second launch forwards `--painel` to the running
        // one (if asked to) and exits.
        let _mutex = CreateMutexW(None, true, w!("Local\\battery-bar-single"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let args: Vec<String> = std::env::args().skip(1).collect();
            if let Some(open) = parse_panel_arg(&args) {
                if let Ok(owner) = FindWindowW(w!("BatteryBarOwner"), PCWSTR::null()) {
                    let _ = PostMessageW(Some(owner), app::WM_PANEL_REMOTE, WPARAM(open as usize), LPARAM(0));
                }
            }
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let _ = app::run();
}

#[cfg(test)]
mod tests {
    use super::parse_panel_arg;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_panel_arg_cases() {
        assert_eq!(parse_panel_arg(&v(&["--painel", "abrir"])), Some(true));
        assert_eq!(parse_panel_arg(&v(&["--painel", "fechar"])), Some(false));
        assert_eq!(parse_panel_arg(&v(&["--painel", "x"])), None);
        assert_eq!(parse_panel_arg(&v(&[])), None);
        assert_eq!(parse_panel_arg(&v(&["--painel"])), None);
    }
}
