//! "Iniciar com o Windows": HKCU\...\Run value pointing at this executable.
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::w;

const RUN_KEY: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: windows::core::PCWSTR = w!("battery-bar");

fn command_line() -> Option<String> {
    std::env::current_exe().ok().map(|p| format!("\"{}\"", p.display()))
}

pub fn is_enabled() -> bool {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .is_ok();
    if !ok {
        return false;
    }
    let len = (size as usize / 2).saturating_sub(1); // size includes the NUL
    let stored = String::from_utf16_lossy(&buf[..len]);
    command_line().is_some_and(|cmd| stored.eq_ignore_ascii_case(&cmd))
}

pub fn set_enabled(on: bool) -> windows::core::Result<()> {
    unsafe {
        if on {
            let cmd = command_line().unwrap_or_default();
            let wide: Vec<u16> = cmd.encode_utf16().chain(Some(0)).collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                (wide.len() * 2) as u32,
            )
            .ok()
        } else {
            match RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) {
                e if e == ERROR_FILE_NOT_FOUND => Ok(()),
                e => e.ok(),
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_roundtrip() {
        let initial = is_enabled();
        set_enabled(true).unwrap();
        assert!(is_enabled());
        set_enabled(false).unwrap();
        assert!(!is_enabled());
        set_enabled(initial).unwrap();
    }
}
