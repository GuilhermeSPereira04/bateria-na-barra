//! The "island": a small black pill at the top center of the primary monitor
//! while the panel is closed. A click-through layered tool window painted once
//! per size with per-pixel alpha (no Direct2D, no resources kept around).
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use crate::render::Bitmap;

/// Size of the pill in DIP.
pub const PILL_W: f32 = 64.0;
pub const PILL_H: f32 = 8.0;

/// Pill size in pixels at `dpi`.
pub fn pill_size(dpi: u32) -> (i32, i32) {
    let s = dpi.max(96) as f32 / 96.0;
    ((PILL_W * s).round() as i32, (PILL_H * s).round() as i32)
}

/// The pill's rectangle: centered on `monitor`, touching its top edge.
pub fn island_rect(monitor: RECT, dpi: u32) -> RECT {
    let (w, h) = pill_size(dpi);
    let left = monitor.left + (monitor.right - monitor.left - w) / 2;
    RECT {
        left,
        top: monitor.top,
        right: left + w,
        bottom: monitor.top + h,
    }
}

/// Coverage (alpha, 0..=255) of a capsule filling `w` x `h`, anti-aliased.
pub fn pill_alpha(w: i32, h: i32) -> Vec<u8> {
    let r = h as f32 / 2.0;
    let (x0, x1) = (r.min(w as f32 / 2.0), (w as f32 - r).max(w as f32 / 2.0));
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = px - px.clamp(x0, x1);
            let dy = py - r;
            let dist = (dx * dx + dy * dy).sqrt() - r;
            out.push(((0.5 - dist).clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

unsafe extern "system" fn island_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

pub struct Island {
    hwnd: HWND,
    /// Where it was last painted and placed.
    rect: Option<RECT>,
    visible: bool,
}

impl Island {
    pub fn create() -> windows::core::Result<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = w!("BatteryBarIsland");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(island_proc),
                hInstance: hinstance.into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc); // fails harmlessly when already registered
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE,
                class,
                w!("battery-bar island"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;
            Ok(Self {
                hwnd,
                rect: None,
                visible: false,
            })
        }
    }

    /// Shows the pill on the primary monitor (repainting only when its rectangle changed).
    pub fn show(&mut self) {
        let r = island_rect(crate::taskbar::primary_monitor_rect(), super::primary_dpi());
        if self.rect != Some(r) {
            self.paint(r);
        }
        unsafe {
            let flags = SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW;
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        }
        self.visible = true;
    }

    pub fn hide(&mut self) {
        if self.visible {
            let _ = unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.visible = false;
        }
    }

    fn paint(&mut self, r: RECT) {
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let Some(mut bmp) = Bitmap::new(w, h) else {
            return;
        };
        for (px, a) in bmp.pixels_mut().chunks_exact_mut(4).zip(pill_alpha(w, h)) {
            px.copy_from_slice(&[0, 0, 0, a]); // premultiplied black
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let ok = unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None,
                Some(&POINT {
                    x: r.left,
                    y: r.top,
                }),
                Some(&SIZE { cx: w, cy: h }),
                Some(bmp.dc),
                Some(&POINT { x: 0, y: 0 }),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        };
        // The system keeps its own copy: `bmp` is freed right here.
        self.rect = ok.is_ok().then_some(r);
    }
}

impl Drop for Island {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[test]
    fn island_rect_at_100_and_150_percent() {
        let r = island_rect(MONITOR, 96);
        assert_eq!((r.left, r.top, r.right, r.bottom), (928, 0, 992, 8));
        let r = island_rect(MONITOR, 144);
        assert_eq!((r.left, r.top, r.right, r.bottom), (912, 0, 1008, 12));
        // Second monitor to the right: centered on the given one.
        let r = island_rect(
            RECT {
                left: 1920,
                top: 0,
                right: 3840,
                bottom: 1080,
            },
            96,
        );
        assert_eq!(r.left, 1920 + 928);
    }

    #[test]
    fn pill_is_round_at_the_ends() {
        let (w, h) = (120, 28);
        let a = pill_alpha(w, h);
        let at = |x: i32, y: i32| a[(y * w + x) as usize];
        assert_eq!(at(60, 14), 255); // center
        assert_eq!(at(60, 0), 255); // flat top touches the edge
        assert_eq!(at(0, 0), 0); // rounded corners
        assert_eq!(at(119, 27), 0);
        assert!(at(0, 14) > 200); // the tip of the end
    }
}
