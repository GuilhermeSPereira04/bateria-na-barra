//! Taskbar light/dark theme.
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

/// `SystemUsesLightTheme == 0` (or missing) means the taskbar is dark.
pub fn taskbar_is_dark() -> bool {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    }
    .is_ok();
    !ok || value == 0
}
