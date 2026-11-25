//! Acrylic backdrop with an adjustable tint, through the undocumented
//! `SetWindowCompositionAttribute` (loaded at run time: if it is missing the
//! caller keeps the system backdrop).
use std::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{s, w};

/// `WCA_ACCENT_POLICY`.
const WCA_ACCENT_POLICY: u32 = 19;
/// `ACCENT_ENABLE_ACRYLICBLURBEHIND`.
const ACCENT_ACRYLIC: i32 = 4;

#[repr(C)]
struct AccentPolicy {
    state: i32,
    flags: i32,
    gradient_color: u32,
    animation_id: i32,
}

#[repr(C)]
struct CompositionAttribData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionAttribData) -> i32;

/// `0xAABBGGRR` for the acrylic gradient: `rgb` is `0xRRGGBB`, `alpha` is
/// limited to 0..=1 and rounded to a byte.
pub fn gradient_abgr(rgb: u32, alpha: f32) -> u32 {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    (a << 24) | (b << 16) | (g << 8) | r
}

/// Turns on the acrylic blur on `hwnd` tinted with `rgb` at opacity `alpha`.
/// False when the function is missing or fails.
pub fn apply(hwnd: HWND, rgb: u32, alpha: f32) -> bool {
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return false };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return false };
        let set: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let mut policy = AccentPolicy { state: ACCENT_ACRYLIC, flags: 0, gradient_color: gradient_abgr(rgb, alpha), animation_id: 0 };
        let mut data = CompositionAttribData {
            attribute: WCA_ACCENT_POLICY,
            data: (&raw mut policy).cast(),
            size: size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_extremes() {
        assert_eq!(gradient_abgr(0x20242C, 1.0), 0xFF2C2420);
        assert_eq!(gradient_abgr(0x20242C, 0.0), 0x002C2420);
        assert_eq!(gradient_abgr(0x20242C, 7.0), 0xFF2C2420);
        assert_eq!(gradient_abgr(0x20242C, -1.0), 0x002C2420);
    }
}
