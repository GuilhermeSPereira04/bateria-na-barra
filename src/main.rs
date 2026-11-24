#![windows_subsystem = "windows"]

mod app;
mod model;
mod readers;
mod render;
mod startup;
mod taskbar;
mod theme;
mod tray;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::core::w;

fn main() {
    unsafe {
        // Single instance: a second launch just exits.
        let _mutex = CreateMutexW(None, true, w!("Local\\battery-bar-single"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let _ = app::run();
}
