//! The top panel: a popup with the Windows 11 system backdrop (frosted glass)
//! drawn over with Direct2D. Created on open and destroyed on close, so no
//! drawing resources stay allocated while it is closed.
pub mod trigger;

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// Posted to the owner by panel worker threads when new data is ready
/// (wParam: which source, see the panel tasks).
#[allow(dead_code)] // posted by the panel's worker threads (later tasks)
pub const WM_PANEL_ASYNC: u32 = WM_APP + 10;
/// Posted to the owner when the panel wants to close itself (Esc, click
/// outside, lost activation). wParam: the panel's HWND.
pub const WM_PANEL_CLOSED: u32 = WM_APP + 11;

const WIDTH_DIP: i32 = 760;
const HEIGHT_DIP: i32 = 420;

/// Horizontally centered on `monitor`, glued to its top edge; never wider than it.
pub fn panel_rect(monitor: RECT, width_px: i32, height_px: i32) -> RECT {
    let monitor_w = monitor.right - monitor.left;
    let width = width_px.min(monitor_w);
    let left = monitor.left + (monitor_w - width) / 2;
    RECT { left, top: monitor.top, right: left + width, bottom: monitor.top + height_px }
}

/// Drawing resources; only alive while the panel is open.
struct Surface {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    rt: Option<ID2D1HwndRenderTarget>,
}

/// State the window procedure reaches through `GWLP_USERDATA`.
struct Shared {
    owner: HWND,
    dpi: u32,
    surface: RefCell<Surface>,
}

impl Shared {
    fn paint(&self, hwnd: HWND) {
        // Skip a re-entrant paint; the region stays invalid and is painted again.
        let Ok(mut s) = self.surface.try_borrow_mut() else { return };
        if s.rt.is_none() {
            s.rt = create_target(&s.d2d, hwnd).ok();
        }
        let Some(rt) = s.rt.clone() else { return };
        let result = unsafe {
            rt.BeginDraw();
            // Fully transparent: the DWM backdrop shows through.
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            let drawn = draw(&rt, &s.dw, self.dpi);
            rt.EndDraw(None, None).and(drawn)
        };
        if result.is_err() {
            // Device lost (D2DERR_RECREATE_TARGET) or similar: rebuild next time.
            s.rt = None;
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }
}

fn create_target(d2d: &ID2D1Factory, hwnd: HWND) -> windows::core::Result<ID2D1HwndRenderTarget> {
    let mut rc = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rc)? };
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        // Premultiplied alpha so transparent pixels reveal the system backdrop.
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        // Draw in physical pixels; the content applies the DPI scale itself.
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
        hwnd,
        pixelSize: D2D_SIZE_U { width: (rc.right - rc.left) as u32, height: (rc.bottom - rc.top) as u32 },
        presentOptions: D2D1_PRESENT_OPTIONS_NONE,
    };
    unsafe { d2d.CreateHwndRenderTarget(&props, &hwnd_props) }
}

/// Panel content. Placeholder: a search box with its hint text.
fn draw(rt: &ID2D1HwndRenderTarget, dw: &IDWriteFactory, dpi: u32) -> windows::core::Result<()> {
    let scale = dpi.max(96) as f32 / 96.0;
    let px = |dip: f32| dip * scale;
    unsafe {
        let size = rt.GetSize();
        let card = D2D_RECT_F { left: px(16.0), top: px(16.0), right: size.width - px(16.0), bottom: px(16.0 + 36.0) };
        let fill = rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.07 }, None)?;
        rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: card, radiusX: px(8.0), radiusY: px(8.0) }, &fill);
        let format = dw.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            px(14.0),
            w!(""),
        )?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        let text = D2D_RECT_F { left: card.left + px(12.0), ..card };
        let hint = rt.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.6 }, None)?;
        // ClearType can't blend onto transparent pixels.
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let wide: Vec<u16> = "Pesquisar apps…".encode_utf16().collect();
        rt.DrawText(&wide, &format, &text, &hint, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
    }
    Ok(())
}

fn shared_of(hwnd: HWND) -> Option<&'static Shared> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Shared;
    // Cleared by `PanelWindow::drop` before the box is freed.
    unsafe { ptr.as_ref() }
}

fn post_closed(hwnd: HWND) {
    if let Some(shared) = shared_of(hwnd) {
        let _ = unsafe { PostMessageW(Some(shared.owner), WM_PANEL_CLOSED, WPARAM(hwnd.0 as usize), LPARAM(0)) };
    }
}

unsafe extern "system" fn panel_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => unsafe {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            if let Some(shared) = shared_of(hwnd) {
                shared.paint(hwnd);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        },
        WM_ERASEBKGND => LRESULT(1),
        // If Windows refused us the foreground on open, a click takes it.
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            post_closed(hwnd);
            LRESULT(0)
        }
        WM_ACTIVATE if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            post_closed(hwnd);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn primary_dpi() -> u32 {
    let mon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let (mut dx, mut dy) = (96, 96);
    if unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) }.is_ok() { dx.max(96) } else { 96 }
}

/// Moves the keyboard focus to `hwnd` without injecting input: borrow the
/// active thread's input state so `SetForegroundWindow` is allowed.
fn take_focus(hwnd: HWND, previous: HWND) {
    unsafe {
        let fg_thread = if previous.is_invalid() { 0 } else { GetWindowThreadProcessId(previous, None) };
        let me = GetCurrentThreadId();
        let attach = fg_thread != 0 && fg_thread != me;
        if attach {
            let _ = AttachThreadInput(fg_thread, me, true);
        }
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if attach {
            let _ = AttachThreadInput(fg_thread, me, false);
        }
    }
}

pub struct PanelWindow {
    hwnd: HWND,
    /// Foreground window before opening; gets the focus back on close.
    previous: HWND,
    /// Owns what the window procedure points at (and the drawing resources).
    _shared: Box<Shared>,
}

impl PanelWindow {
    /// Creates, shows and focuses the panel on the primary monitor. Close
    /// requests are posted to `owner` as `WM_PANEL_CLOSED`.
    pub fn open(owner: HWND) -> windows::core::Result<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = w!("BatteryBarPanel");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(panel_proc),
                hInstance: hinstance.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&wc); // fails harmlessly when already registered

            let dpi = primary_dpi();
            let (w_px, h_px) = (WIDTH_DIP * dpi as i32 / 96, HEIGHT_DIP * dpi as i32 / 96);
            let r = panel_rect(crate::taskbar::primary_monitor_rect(), w_px, h_px);
            let shared = Box::new(Shared {
                owner,
                dpi,
                surface: RefCell::new(Surface {
                    d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                    dw: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                    rt: None,
                }),
            });
            // No WS_EX_LAYERED: it would block the system backdrop.
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                class,
                w!("battery-bar panel"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                Some(hinstance.into()),
                None,
            )?;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*shared as *const Shared as isize);

            let set = |attr, value: i32| {
                let _ = DwmSetWindowAttribute(hwnd, attr, &value as *const i32 as _, size_of::<i32>() as u32);
            };
            set(DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW.0);
            set(DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
            set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0);
            let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

            let previous = GetForegroundWindow();
            let _ = ShowWindow(hwnd, SW_SHOW);
            take_focus(hwnd, previous);
            // The tray menu leaves our hidden owner in front; nothing to give back to.
            let previous = if previous == owner { HWND::default() } else { previous };
            Ok(Self { hwnd, previous, _shared: shared })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Schedules a repaint (drawn on the next `WM_PAINT`).
    #[allow(dead_code)] // called once the panel shows live data
    pub fn redraw(&mut self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
}

impl Drop for PanelWindow {
    fn drop(&mut self) {
        unsafe {
            // Detach first: closing must not post another WM_PANEL_CLOSED, and
            // the window procedure must not touch `_shared` once it is freed.
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            // Give the focus back only if we still hold it: after a click
            // outside, the clicked window keeps it.
            if GetForegroundWindow() == self.hwnd && !self.previous.is_invalid() && IsWindow(Some(self.previous)).as_bool() {
                let _ = SetForegroundWindow(self.previous);
            }
            let _ = DestroyWindow(self.hwnd);
        }
        // `_shared` (render target and factories) is released right after.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };

    #[test]
    fn panel_centered_at_top() {
        let r = panel_rect(MONITOR, 760, 420);
        assert_eq!((r.left, r.top, r.right, r.bottom), (580, 0, 1340, 420));
    }

    #[test]
    fn panel_clamped_on_small_monitor() {
        let small = RECT { left: 100, top: -50, right: 700, bottom: 400 };
        let r = panel_rect(small, 760, 420);
        assert_eq!((r.left, r.top, r.right), (100, -50, 700));
    }
}
