//! The strip window: a borderless, click-through-where-transparent layered
//! window kept over the taskbar, just left of the notification area.
#![allow(dead_code)]

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    MONITORINFO, MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::render::{Bitmap, Renderer, Strip};

/// Posted to the owner window when the strip is clicked (wParam: the mouse message).
pub const WM_STRIP_CLICK: u32 = WM_APP + 2;

fn px(dip: i32, dpi: u32) -> i32 {
    dip * dpi as i32 / 96
}

/// Where the strip goes: right edge 8 DIP left of the notification area
/// (or 160 DIP from the taskbar's right end when it can't be found), never
/// further left than 4 DIP into the taskbar, vertically centered.
pub fn compute_strip_rect(tb: RECT, notify: Option<RECT>, width: i32, height: i32, dpi: u32) -> RECT {
    let right = notify.map_or(tb.right - px(160, dpi), |n| n.left - px(8, dpi));
    let left = (right - width).max(tb.left + px(4, dpi));
    let top = tb.top + ((tb.bottom - tb.top - height) / 2).max(0);
    RECT { left, top, right: left + width, bottom: top + height }
}

/// True if `rect` horizontally overlaps any span; touching edges don't count.
pub fn overlaps(rect: RECT, spans: &[(i32, i32)]) -> bool {
    spans.iter().any(|&(l, r)| rect.left < r && rect.right > l)
}

/// Horizontal spans of the taskbar's buttons (app buttons, Start, search…),
/// excluding those inside the notification area. Uses UI Automation, so call
/// it off the UI thread (the caller must have initialized COM).
pub fn occupied_spans(tb_hwnd: HWND, notify: Option<RECT>) -> Vec<(i32, i32)> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::System::Variant::VARIANT;
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_ControlTypePropertyId,
    };
    // On Windows 11 the buttons live in a XAML island; searching from
    // Shell_TrayWnd alone stops at empty containers, so scan the island too.
    // Scanning Shell_TrayWnd itself is only a fallback (classic taskbars):
    // on Windows 11 it costs ~25 ms to find nothing.
    let mut roots: Vec<HWND> = [w!("Windows.UI.Composition.DesktopWindowContentBridge"), w!("MSTaskListWClass")]
        .into_iter()
        .filter_map(|class| unsafe { FindWindowExW(Some(tb_hwnd), None, class, PCWSTR::null()) }.ok())
        .collect();
    if roots.is_empty() {
        roots.push(tb_hwnd);
    }
    thread_local! {
        static UIA: std::cell::OnceCell<Option<IUIAutomation>> = const { std::cell::OnceCell::new() };
    }
    let Some(uia) = UIA.with(|c| c.get_or_init(|| unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() }).clone()) else {
        return Vec::new();
    };
    let scan = |hwnd: HWND| -> windows::core::Result<Vec<(i32, i32)>> {
        unsafe {
            let root = uia.ElementFromHandle(hwnd)?;
            let cond = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0))?;
            let found = root.FindAll(TreeScope_Descendants, &cond)?;
            let mut spans = Vec::new();
            for i in 0..found.Length()? {
                let r = found.GetElement(i)?.CurrentBoundingRectangle()?;
                if r.right <= r.left {
                    continue; // off-screen/virtualized
                }
                if notify.is_some_and(|n| r.left >= n.left && r.right <= n.right) {
                    continue;
                }
                spans.push((r.left, r.right));
            }
            Ok(spans)
        }
    };
    let mut spans: Vec<(i32, i32)> = roots.into_iter().flat_map(|h| scan(h).unwrap_or_default()).collect();
    spans.sort_unstable();
    spans.dedup();
    spans
}

/// Rescans the taskbar's buttons every 10 s (only while the taskbar is shown) on a background thread.
pub fn spawn_scanner(shared: std::sync::Arc<std::sync::Mutex<Vec<(i32, i32)>>>) {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    std::thread::spawn(move || {
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        loop {
            // Nothing to avoid while an auto-hide taskbar is tucked away.
            if let Some(geo) = query_geometry() {
                let spans = occupied_spans(geo.tb_hwnd, geo.notify);
                if let Ok(mut guard) = shared.lock() {
                    *guard = spans;
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
}

struct Geometry {
    tb_hwnd: HWND,
    tb: RECT,
    notify: Option<RECT>,
    monitor: RECT,
    dpi: u32,
}

fn find_child_rect(parent: HWND, class: PCWSTR) -> Option<RECT> {
    unsafe {
        let child = FindWindowExW(Some(parent), None, class, PCWSTR::null()).ok()?;
        let mut r = RECT::default();
        GetWindowRect(child, &mut r).ok()?;
        (r.right > r.left).then_some(r)
    }
}

/// Main taskbar geometry, or `None` when it is hidden (auto-hide) or missing.
fn query_geometry() -> Option<Geometry> {
    unsafe {
        let tb_hwnd = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
        if !IsWindowVisible(tb_hwnd).as_bool() {
            return None;
        }
        let mut tb = RECT::default();
        GetWindowRect(tb_hwnd, &mut tb).ok()?;
        let mon = MonitorFromWindow(tb_hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return None;
        }
        let m = mi.rcMonitor;
        // Auto-hide slides the taskbar (almost) off screen.
        let visible_h = tb.bottom.min(m.bottom) - tb.top.max(m.top);
        let visible_w = tb.right.min(m.right) - tb.left.max(m.left);
        if visible_h <= 8 || visible_w <= 8 {
            return None;
        }
        let (mut dx, mut dy) = (96, 96);
        let dpi = if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() { dx.max(96) } else { 96 };
        Some(Geometry { tb_hwnd, tb, notify: find_child_rect(tb_hwnd, w!("TrayNotifyWnd")), monitor: m, dpi })
    }
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Fullscreen = covers the whole monitor without a title bar (games, F11, video
/// players). Maximized windows keep `WS_CAPTION`, and with an auto-hide taskbar
/// they also cover the whole monitor, so the caption is what tells them apart.
fn is_fullscreen(rect: RECT, style: u32, monitor: RECT) -> bool {
    let covers = rect.left <= monitor.left
        && rect.top <= monitor.top
        && rect.right >= monitor.right
        && rect.bottom >= monitor.bottom;
    covers && style & WS_CAPTION.0 != WS_CAPTION.0
}

/// Hide when the foreground window is fullscreen, or when the shell itself
/// reports a fullscreen app / presentation (also covers an overlay or popup
/// taking focus while a game keeps running fullscreen).
fn fullscreen_active(foreground_fullscreen: bool, shell_state: QUERY_USER_NOTIFICATION_STATE) -> bool {
    foreground_fullscreen
        || matches!(shell_state, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
}

fn shell_state() -> QUERY_USER_NOTIFICATION_STATE {
    unsafe { SHQueryUserNotificationState() }.unwrap_or(QUNS_ACCEPTS_NOTIFICATIONS)
}

/// True when the foreground window (not the shell, not us) covers the whole monitor.
fn foreground_is_fullscreen(own: HWND, monitor: RECT) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || fg == own {
            return false;
        }
        if matches!(class_name(fg).as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "Progman" | "WorkerW") {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(fg, &mut r).is_err() {
            return false;
        }
        is_fullscreen(r, GetWindowLongW(fg, GWL_STYLE) as u32, monitor)
    }
}

/// The primary monitor's rectangle, in physical pixels.
pub fn primary_monitor_rect() -> RECT {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        mi.rcMonitor
    }
}

/// True when a game, video or presentation runs fullscreen on the primary monitor.
pub fn fullscreen_now() -> bool {
    fullscreen_active(foreground_is_fullscreen(HWND::default(), primary_monitor_rect()), shell_state())
}

pub struct StripWindow {
    hwnd: HWND,
    renderer: Renderer,
    strip: Option<Strip>,
    bitmap: Option<Bitmap>,
    rendered_for: Option<(u32, i32)>, // (dpi, taskbar height)
    last_rect: Option<RECT>,
    /// Taskbar button spans the strip must not cover; refreshed by `spawn_scanner`.
    occupied: std::sync::Arc<std::sync::Mutex<Vec<(i32, i32)>>>,
}

unsafe extern "system" fn strip_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let owner = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
            if owner != 0 {
                let _ = unsafe { PostMessageW(Some(HWND(owner as _)), WM_STRIP_CLICK, WPARAM(msg as usize), LPARAM(0)) };
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl StripWindow {
    /// Creates the (hidden) strip window. Clicks are posted to `owner` as `WM_STRIP_CLICK`.
    pub fn create(owner: HWND) -> windows::core::Result<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = w!("BatteryBarStrip");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(strip_proc),
                hInstance: hinstance.into(),
                lpszClassName: class,
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                class,
                w!("battery-bar"),
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
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, owner.0 as isize);
            Ok(Self {
                hwnd,
                renderer: Renderer::new()?,
                strip: None,
                bitmap: None,
                rendered_for: None,
                last_rect: None,
                occupied: {
                    let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                    spawn_scanner(shared.clone());
                    shared
                },
            })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// New content: redraws and repositions right away.
    pub fn set_strip(&mut self, strip: Strip) {
        self.strip = Some(strip);
        self.rendered_for = None;
        self.sync();
    }

    /// Forces a redraw on the next `sync` (DPI/theme/explorer restart).
    pub fn invalidate(&mut self) {
        self.rendered_for = None;
        self.last_rect = None;
    }

    fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.last_rect = None;
    }

    /// Re-renders if needed, repositions, re-asserts topmost, or hides. Called every second.
    pub fn sync(&mut self) {
        let Some(geo) = query_geometry() else {
            return self.hide();
        };
        if fullscreen_active(foreground_is_fullscreen(self.hwnd, geo.monitor), shell_state()) {
            return self.hide();
        }
        let tb_height = geo.tb.bottom - geo.tb.top;
        if self.rendered_for != Some((geo.dpi, tb_height)) {
            self.bitmap = self.strip.as_ref().and_then(|s| self.renderer.render(s, geo.dpi, tb_height));
            self.rendered_for = Some((geo.dpi, tb_height));
            self.last_rect = None;
        }
        let Some(bmp) = &self.bitmap else {
            return self.hide();
        };
        let target = compute_strip_rect(geo.tb, geo.notify, bmp.width, bmp.height, geo.dpi);
        let covered = self.occupied.lock().map(|spans| overlaps(target, &spans)).unwrap_or(false);
        if covered {
            return self.hide();
        }
        unsafe {
            if self.last_rect != Some(target) {
                let dst = POINT { x: target.left, y: target.top };
                let size = SIZE { cx: bmp.width, cy: bmp.height };
                let src = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    self.hwnd,
                    None,
                    Some(&dst),
                    Some(&size),
                    Some(bmp.dc),
                    Some(&src),
                    windows::Win32::Foundation::COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                self.last_rect = Some(target);
            }
            // Clicking the taskbar raises it; put the strip back on top.
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        let _ = geo.tb_hwnd;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: RECT = RECT { left: 0, top: 1032, right: 1920, bottom: 1080 };
    const NOTIFY: RECT = RECT { left: 1700, top: 1032, right: 1920, bottom: 1080 };

    #[test]
    fn left_of_tray() {
        let r = compute_strip_rect(TB, Some(NOTIFY), 200, 48, 96);
        assert_eq!((r.left, r.top, r.right, r.bottom), (1492, 1032, 1692, 1080));
    }

    #[test]
    fn centered_vertically() {
        assert_eq!(compute_strip_rect(TB, Some(NOTIFY), 200, 40, 96).top, 1036);
    }

    #[test]
    fn no_notify_fallback() {
        assert_eq!(compute_strip_rect(TB, None, 200, 48, 96).right, 1760);
    }

    #[test]
    fn clamped_left() {
        let r = compute_strip_rect(TB, Some(NOTIFY), 5000, 48, 96);
        assert_eq!((r.left, r.right), (4, 5004));
    }

    #[test]
    fn overlap_rules() {
        let strip = RECT { left: 100, top: 0, right: 200, bottom: 48 };
        assert!(!overlaps(strip, &[(50, 99)]));
        assert!(overlaps(strip, &[(150, 160)]));
        assert!(!overlaps(strip, &[(200, 300)]));
        assert!(overlaps(strip, &[(10, 20), (190, 250)]));
        assert!(!overlaps(strip, &[]));
    }

    #[test]
    fn shell_fullscreen_state_hides_even_without_focus() {
        // An overlay/popup has focus while an exclusive-fullscreen game runs.
        assert!(fullscreen_active(false, QUNS_RUNNING_D3D_FULL_SCREEN));
        assert!(fullscreen_active(false, QUNS_BUSY));
        assert!(fullscreen_active(false, QUNS_PRESENTATION_MODE));
    }

    #[test]
    fn normal_shell_state_follows_foreground_check() {
        assert!(!fullscreen_active(false, QUNS_ACCEPTS_NOTIFICATIONS));
        assert!(!fullscreen_active(false, QUNS_QUIET_TIME)); // Do Not Disturb is not fullscreen
        assert!(fullscreen_active(true, QUNS_ACCEPTS_NOTIFICATIONS));
    }

    const MONITOR: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };

    #[test]
    fn maximized_window_with_caption_is_not_fullscreen() {
        // A maximized Chrome with an auto-hide taskbar covers the whole monitor.
        let rect = RECT { left: -8, top: -8, right: 1928, bottom: 1088 };
        assert!(!is_fullscreen(rect, 0x17CF_0000, MONITOR));
    }

    #[test]
    fn borderless_window_covering_monitor_is_fullscreen() {
        assert!(is_fullscreen(MONITOR, 0x9000_0000, MONITOR)); // WS_POPUP | WS_VISIBLE
    }

    #[test]
    fn smaller_borderless_window_is_not_fullscreen() {
        let rect = RECT { left: 0, top: 0, right: 1280, bottom: 720 };
        assert!(!is_fullscreen(rect, 0x9000_0000, MONITOR));
    }

    #[test]
    fn dpi_150() {
        assert_eq!(compute_strip_rect(TB, Some(NOTIFY), 200, 48, 144).right, 1688);
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Needs a real Windows 11 taskbar with at least one app button.
    #[test]
    #[ignore]
    fn scan_finds_app_buttons() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            let tb = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).unwrap();
            let notify = find_child_rect(tb, w!("TrayNotifyWnd"));
            let t = std::time::Instant::now();
            let spans = occupied_spans(tb, notify);
            println!("spans={spans:?} in {:?}", t.elapsed());
            assert!(!spans.is_empty());
            assert!(spans.iter().all(|&(_, r)| notify.is_none_or(|n| r <= n.left)), "tray buttons excluded");
        }
    }
}
