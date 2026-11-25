//! Wi-Fi and Bluetooth: radio state (WinRT `Radio`), the connected Wi-Fi
//! network name (WLAN API) and the connected Bluetooth devices (Win32).
use std::sync::mpsc::{self, Receiver};

use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioState};
use windows::Win32::Devices::Bluetooth::{
    BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS, BluetoothFindDeviceClose, BluetoothFindFirstDevice,
    BluetoothFindNextDevice,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::NetworkManagement::WiFi::{
    WLAN_CONNECTION_ATTRIBUTES, WLAN_INTERFACE_INFO_LIST, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanOpenHandle,
    WlanQueryInterface, wlan_intf_opcode_current_connection, wlan_interface_state_connected,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow, PostMessageW, SW_SHOWNORMAL};
use windows::core::{HSTRING, w};

use crate::panel::view::Toggle;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RadioKind {
    Wifi,
    Bluetooth,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RadioSnapshot {
    pub wifi: Toggle,
    pub wifi_name: Option<String>,
    pub bt: Toggle,
    pub bt_connected: u32,
}

/// A missing radio is unavailable; a present one is on or off.
pub fn to_toggle(present: bool, on: bool) -> Toggle {
    match (present, on) {
        (false, _) => Toggle::Unavailable,
        (true, false) => Toggle::Off,
        (true, true) => Toggle::On,
    }
}

/// The system radios of `kind`; empty on any failure.
fn radios(kind: RadioKind) -> Vec<Radio> {
    let wanted = match kind {
        RadioKind::Wifi => windows::Devices::Radios::RadioKind::WiFi,
        RadioKind::Bluetooth => windows::Devices::Radios::RadioKind::Bluetooth,
    };
    let Ok(op) = Radio::GetRadiosAsync() else { return Vec::new() };
    let Ok(list) = op.join() else { return Vec::new() };
    list.into_iter().filter(|r| r.Kind().is_ok_and(|k| k == wanted)).collect()
}

fn toggle_of(kind: RadioKind) -> Toggle {
    let list = radios(kind);
    to_toggle(!list.is_empty(), list.iter().any(|r| r.State().is_ok_and(|s| s == RadioState::On)))
}

/// SSID of the first connected WLAN interface.
fn wifi_name() -> Option<String> {
    unsafe {
        let mut version = 0;
        let mut client = HANDLE::default();
        if WlanOpenHandle(2, None, &mut version, &mut client) != ERROR_SUCCESS.0 {
            return None;
        }
        let mut name = None;
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        if WlanEnumInterfaces(client, None, &mut list) == ERROR_SUCCESS.0 && !list.is_null() {
            let count = (*list).dwNumberOfItems as usize;
            let items = std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count);
            for itf in items.iter().filter(|i| i.isState == wlan_interface_state_connected) {
                let mut size = 0;
                let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
                let status = WlanQueryInterface(
                    client,
                    &itf.InterfaceGuid,
                    wlan_intf_opcode_current_connection,
                    None,
                    &mut size,
                    &mut data,
                    None,
                );
                if status == ERROR_SUCCESS.0 && !data.is_null() {
                    let ssid = &(*(data as *const WLAN_CONNECTION_ATTRIBUTES)).wlanAssociationAttributes.dot11Ssid;
                    let len = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
                    name = Some(String::from_utf8_lossy(&ssid.ucSSID[..len]).into_owned()).filter(|s| !s.is_empty());
                }
                if !data.is_null() {
                    WlanFreeMemory(data);
                }
                if name.is_some() {
                    break;
                }
            }
        }
        if !list.is_null() {
            WlanFreeMemory(list as _);
        }
        WlanCloseHandle(client, None);
        name
    }
}

/// How many paired Bluetooth devices are connected right now.
fn bt_connected() -> u32 {
    unsafe {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnConnected: true.into(),
            ..Default::default()
        };
        let mut info = BLUETOOTH_DEVICE_INFO { dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32, ..Default::default() };
        let Ok(find) = BluetoothFindFirstDevice(&params, &mut info) else { return 0 };
        let mut n = 1;
        while BluetoothFindNextDevice(find, &mut info).is_ok() {
            n += 1;
        }
        let _ = BluetoothFindDeviceClose(find);
        n
    }
}

/// Reads everything (blocking; call from a worker thread with COM initialized).
pub fn snapshot() -> RadioSnapshot {
    let wifi = toggle_of(RadioKind::Wifi);
    let bt = toggle_of(RadioKind::Bluetooth);
    RadioSnapshot {
        wifi,
        wifi_name: if wifi == Toggle::On { wifi_name() } else { None },
        bt,
        bt_connected: if bt == Toggle::On { bt_connected() } else { 0 },
    }
}

/// Turns every radio of `kind` on or off (blocking; worker thread).
pub fn set(kind: RadioKind, on: bool) {
    let state = if on { RadioState::On } else { RadioState::Off };
    let allowed = Radio::RequestAccessAsync().and_then(|op| op.join()).is_ok_and(|a| a == RadioAccessStatus::Allowed);
    if !allowed {
        return;
    }
    for radio in radios(kind) {
        let _ = radio.SetStateAsync(state).and_then(|op| op.join());
    }
}

/// Runs `job` on a worker thread (own COM apartment), then reads a snapshot
/// and announces it to `owner` by `WM_PANEL_ASYNC`. Dropping the receiver
/// (panel closed, or a newer request) discards the result.
pub fn spawn(owner: HWND, job: impl FnOnce() + Send + 'static) -> Receiver<RadioSnapshot> {
    let (tx, rx) = mpsc::channel();
    let owner = owner.0 as isize; // HWND is not Send
    std::thread::spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        job();
        let snap = snapshot();
        if com {
            unsafe { CoUninitialize() };
        }
        if tx.send(snap).is_ok() {
            let (msg, source) = (crate::panel::WM_PANEL_ASYNC, WPARAM(crate::panel::ASYNC_RADIOS));
            let _ = unsafe { PostMessageW(Some(HWND(owner as _)), msg, source, LPARAM(0)) };
        }
    });
    rx
}

/// Opens a `ms-settings:` page, letting it take the foreground.
pub fn open_settings(uri: &str) {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        ShellExecuteW(None, w!("open"), &HSTRING::from(uri), None, None, SW_SHOWNORMAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_radio_is_unavailable() {
        assert_eq!(to_toggle(false, true), Toggle::Unavailable);
        assert_eq!(to_toggle(false, false), Toggle::Unavailable);
    }

    #[test]
    fn present_off_is_off() {
        assert_eq!(to_toggle(true, false), Toggle::Off);
    }

    #[test]
    fn present_on_is_on() {
        assert_eq!(to_toggle(true, true), Toggle::On);
    }
}
