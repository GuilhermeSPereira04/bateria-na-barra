//! Audio output: volume and mute of the default render endpoint (Core Audio)
//! and switching the default output (`IPolicyConfig`).
use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eCommunications, eConsole, eMultimedia,
    eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR};

use crate::panel::view::Toggle;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSnapshot {
    /// On = sound, Off = muted, Unavailable = no output endpoint.
    pub state: Toggle,
    /// 0..=100.
    pub volume: u8,
    /// Friendly name of the default output.
    pub output: Option<String>,
    /// Active outputs as (endpoint id, friendly name).
    pub outputs: Vec<(String, String)>,
}

/// `current` moved by `delta` percentage points, kept within 0..=100.
pub fn clamp_volume(current: u8, delta: i32) -> u8 {
    (current as i32).saturating_add(delta).clamp(0, 100) as u8
}

/// The snapshot for the default endpoint as (muted, volume, name), if any.
pub fn snapshot_from(endpoint: Option<(bool, u8, String)>, outputs: Vec<(String, String)>) -> AudioSnapshot {
    match endpoint {
        None => AudioSnapshot { outputs, ..Default::default() },
        Some((muted, volume, name)) => AudioSnapshot {
            state: if muted { Toggle::Off } else { Toggle::On },
            volume: volume.min(100),
            output: Some(name).filter(|n| !n.is_empty()),
            outputs,
        },
    }
}

/// Undocumented, but stable since Windows 7: what Sound settings and tools
/// like EarTrumpet use to change the default device. Only the slot order
/// matters; the methods before `SetDefaultEndpoint` are never called.
#[allow(non_snake_case)] // the COM names, as in the SDK
mod policy {
    use std::ffi::c_void;

    use windows::Win32::Media::Audio::ERole;
    use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, device: PCWSTR, default: i32, format: *mut *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, device: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
        fn GetProcessingPeriod(&self, device: PCWSTR, default: i32, period: *mut i64, min: *mut i64) -> HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(&self, device: PCWSTR, fx: i32, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetPropertyValue(&self, device: PCWSTR, fx: i32, key: *const c_void, value: *mut c_void) -> HRESULT;
        pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }
}
use policy::IPolicyConfig;

/// CPolicyConfigClient, the coclass behind `IPolicyConfig`.
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()
}

fn default_output() -> Option<IMMDevice> {
    unsafe { enumerator()?.GetDefaultAudioEndpoint(eRender, eMultimedia) }.ok()
}

fn volume_of(device: &IMMDevice) -> Option<IAudioEndpointVolume> {
    unsafe { device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) }.ok()
}

/// Copies a COM-allocated string and frees it.
fn take_pwstr(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { p.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
    s
}

fn id_of(device: &IMMDevice) -> Option<String> {
    take_pwstr(unsafe { device.GetId() }.ok()?)
}

/// "Alto-falantes (Realtek(R) Audio)" and the like.
fn name_of(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let name = PropVariantToStringAlloc(&value).ok().and_then(take_pwstr);
        let _ = PropVariantClear(&mut value);
        name.filter(|n| !n.is_empty())
    }
}

/// (muted, volume 0..=100, name) of the default output.
pub fn endpoint() -> Option<(bool, u8, String)> {
    let device = default_output()?;
    let volume = volume_of(&device)?;
    let level = unsafe { volume.GetMasterVolumeLevelScalar() }.ok()?;
    let muted = unsafe { volume.GetMute() }.is_ok_and(|m| m.as_bool());
    Some((muted, percent(level), name_of(&device).unwrap_or_default()))
}

/// A 0.0..=1.0 scalar level as a percentage.
fn percent(level: f32) -> u8 {
    (level * 100.0).round().clamp(0.0, 100.0) as u8
}

/// The active outputs as (id, name).
fn outputs() -> Vec<(String, String)> {
    let Some(collection) = enumerator().and_then(|e| unsafe { e.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }.ok())
    else {
        return Vec::new();
    };
    let count = unsafe { collection.GetCount() }.unwrap_or(0);
    (0..count)
        .filter_map(|i| unsafe { collection.Item(i) }.ok())
        .filter_map(|d| Some((id_of(&d)?, name_of(&d).unwrap_or_else(|| "Saída de áudio".into()))))
        .collect()
}

/// Reads everything (COM must be initialized on the calling thread).
pub fn snapshot() -> AudioSnapshot {
    snapshot_from(endpoint(), outputs())
}

/// Moves the default output's volume by `delta` percentage points.
pub fn step_volume(delta: i32) {
    let Some(volume) = default_output().and_then(|d| volume_of(&d)) else { return };
    unsafe {
        let Ok(level) = volume.GetMasterVolumeLevelScalar() else { return };
        let current = percent(level);
        let next = clamp_volume(current, delta);
        if next != current {
            let _ = volume.SetMasterVolumeLevelScalar(f32::from(next) / 100.0, std::ptr::null());
        }
    }
}

/// Mutes or unmutes the default output.
pub fn toggle_mute() {
    let Some(volume) = default_output().and_then(|d| volume_of(&d)) else { return };
    unsafe {
        if let Ok(muted) = volume.GetMute() {
            let _ = volume.SetMute(!muted.as_bool(), std::ptr::null());
        }
    }
}

/// Makes `id` the default output for every role; false if Windows refused
/// (or `IPolicyConfig` is gone), so the caller can fall back to Settings.
pub fn set_default(id: &str) -> bool {
    let Ok(policy) = (unsafe { CoCreateInstance::<_, IPolicyConfig>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }) else {
        return false;
    };
    let id = HSTRING::from(id);
    [eConsole, eMultimedia, eCommunications]
        .into_iter()
        .all(|role| unsafe { policy.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role) }.is_ok())
}

/// Reads a snapshot on a worker thread (own COM apartment) and announces it
/// to `owner` by `WM_PANEL_ASYNC`. Dropping the receiver (panel closed, or a
/// newer request) discards the result.
pub fn spawn(owner: HWND) -> Receiver<AudioSnapshot> {
    let (tx, rx) = mpsc::channel();
    let owner = owner.0 as isize; // HWND is not Send
    std::thread::spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let snap = snapshot();
        if com {
            unsafe { CoUninitialize() };
        }
        if tx.send(snap).is_ok() {
            let (msg, source) = (crate::panel::WM_PANEL_ASYNC, WPARAM(crate::panel::ASYNC_AUDIO));
            let _ = unsafe { PostMessageW(Some(HWND(owner as _)), msg, source, LPARAM(0)) };
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_clamped() {
        assert_eq!(clamp_volume(98, 4), 100);
        assert_eq!(clamp_volume(1, -4), 0);
        assert_eq!(clamp_volume(50, 2), 52);
        assert_eq!(clamp_volume(50, -2), 48);
    }

    #[test]
    fn no_endpoint_is_unavailable() {
        let snap = snapshot_from(None, vec![("id".into(), "Fone".into())]);
        assert_eq!(snap.state, Toggle::Unavailable);
        assert_eq!(snap.output, None);
        assert_eq!(snap.outputs.len(), 1);
    }

    #[test]
    fn muted_endpoint_is_off() {
        let snap = snapshot_from(Some((true, 40, "Fone USB".into())), Vec::new());
        assert_eq!((snap.state, snap.volume, snap.output.as_deref()), (Toggle::Off, 40, Some("Fone USB")));
        let snap = snapshot_from(Some((false, 60, "Monitor".into())), Vec::new());
        assert_eq!((snap.state, snap.volume), (Toggle::On, 60));
    }

    /// Read-only look at the real outputs: `cargo test -- --ignored audio_reads`.
    #[test]
    #[ignore]
    fn audio_reads_real_outputs() {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let snap = snapshot();
        println!("{snap:?}");
        if com {
            unsafe { CoUninitialize() };
        }
    }
}
