//! The top panel: a popup with the Windows 11 system backdrop (frosted glass)
//! drawn over with Direct2D. Created on open and destroyed on close, so no
//! drawing resources stay allocated while it is closed.
pub mod draw;
pub mod layout;
pub mod trigger;
pub mod view;

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
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
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_BACK, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use layout::{Layout, hit_test};
use view::{Hit, Key, PanelView};

/// Posted to the owner by panel worker threads when new data is ready
/// (wParam: which source, see `ASYNC_*`).
pub const WM_PANEL_ASYNC: u32 = WM_APP + 10;
/// `WM_PANEL_ASYNC` source: the app list for the search.
pub const ASYNC_APPS: usize = 1;
/// `WM_PANEL_ASYNC` source: the Wi-Fi and Bluetooth state.
pub const ASYNC_RADIOS: usize = 2;
/// Posted to the owner when the panel wants to close itself (Esc, click
/// outside, lost activation). wParam: the panel's HWND.
pub const WM_PANEL_CLOSED: u32 = WM_APP + 11;
/// Posted to the owner after a click on something clickable; the owner
/// collects it with `PanelWindow::take_click`. wParam: the panel's HWND.
pub const WM_PANEL_HIT: u32 = WM_APP + 12;
/// Posted to the owner after keys were typed; the owner collects them with
/// `PanelWindow::take_keys`. wParam: the panel's HWND.
pub const WM_PANEL_KEY: u32 = WM_APP + 13;

/// The search key behind a `WM_CHAR` (printable characters only; Backspace,
/// Enter and Esc come through `WM_KEYDOWN`).
fn char_key(unit: usize) -> Option<Key> {
    let c = char::from_u32(unit as u32)?;
    (!c.is_control()).then_some(Key::Char(c))
}

/// The search key behind a `WM_KEYDOWN`.
fn vk_key(vk: usize) -> Option<Key> {
    match VIRTUAL_KEY(vk as u16) {
        VK_BACK => Some(Key::Backspace),
        VK_UP => Some(Key::Up),
        VK_DOWN => Some(Key::Down),
        VK_RETURN => Some(Key::Enter),
        _ => None,
    }
}

/// True when `pt` is on the top row of `monitor` (not on a monitor stacked above it).
pub fn on_top_edge(pt: POINT, monitor: RECT) -> bool {
    pt.y == monitor.top && pt.x >= monitor.left && pt.x < monitor.right
}

/// Horizontally centered on `monitor`, glued to its top edge; never wider than it.
pub fn panel_rect(monitor: RECT, width_px: i32, height_px: i32) -> RECT {
    let monitor_w = monitor.right - monitor.left;
    let width = width_px.min(monitor_w);
    let left = monitor.left + (monitor_w - width) / 2;
    RECT { left, top: monitor.top, right: left + width, bottom: monitor.top + height_px }
}

/// Window rectangle for a layout of `size` pixels on the primary monitor.
fn rect_for(size: (f32, f32)) -> RECT {
    panel_rect(crate::taskbar::primary_monitor_rect(), size.0.round() as i32, size.1.round() as i32)
}

/// Drawing resources; only alive while the panel is open.
struct Surface {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    rt: Option<ID2D1HwndRenderTarget>,
    cache: draw::Cache,
}

/// What is shown, and where.
struct Scene {
    view: PanelView,
    layout: Layout,
}

/// State the window procedure reaches through `GWLP_USERDATA`.
struct Shared {
    owner: HWND,
    surface: RefCell<Surface>,
    scene: RefCell<Scene>,
    /// The last click, waiting for the owner (see `WM_PANEL_HIT`).
    click: Cell<Option<Hit>>,
    /// Keys typed since the owner last looked (see `WM_PANEL_KEY`).
    keys: RefCell<Vec<Key>>,
    /// `TrackMouseEvent` is armed (a `WM_MOUSELEAVE` will come).
    tracking: Cell<bool>,
}

fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

impl Shared {
    fn paint(&self, hwnd: HWND) {
        // Skip a re-entrant paint; the region stays invalid and is painted again.
        let (Ok(mut s), Ok(scene)) = (self.surface.try_borrow_mut(), self.scene.try_borrow()) else { return };
        if s.rt.is_none() {
            s.rt = create_target(&s.d2d, hwnd).ok();
        }
        let Some(rt) = s.rt.clone() else { return };
        let Surface { dw, cache, .. } = &mut *s;
        let ended = unsafe {
            rt.BeginDraw();
            // Fully transparent: the DWM backdrop shows through.
            rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            // A drawing error (a missing font, say) would fail again on every
            // paint: keep whatever was drawn instead of retrying.
            let _ = draw::draw(&rt, dw, cache, &scene.view, &scene.layout);
            rt.EndDraw(None, None)
        };
        if ended.is_err_and(|e| e.code() == D2DERR_RECREATE_TARGET) {
            // Device lost: rebuild the target and paint again.
            s.rt = None;
            s.cache.reset_target();
            invalidate(hwnd);
        }
    }

    fn resize_target(&self, width: u32, height: u32) {
        let Ok(mut s) = self.surface.try_borrow_mut() else { return };
        let failed = s.rt.as_ref().is_some_and(|rt| unsafe { rt.Resize(&D2D_SIZE_U { width, height }) }.is_err());
        if failed {
            s.rt = None;
            s.cache.reset_target();
        }
    }

    /// Recomputes the layout after `change`; moves/resizes the window when its size changed.
    fn update(&self, hwnd: HWND, change: impl FnOnce(&mut PanelView)) {
        let resized = {
            let Ok(mut scene) = self.scene.try_borrow_mut() else { return };
            change(&mut scene.view);
            let layout = layout::layout(&scene.view);
            let resized = layout.size != scene.layout.size;
            scene.layout = layout;
            resized.then_some(scene.layout.size)
        };
        if let Some(size) = resized {
            let r = rect_for(size);
            unsafe {
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        invalidate(hwnd);
    }

    fn set_hover(&self, hwnd: HWND, hover: Option<Hit>) {
        let changed = self.scene.try_borrow_mut().is_ok_and(|mut scene| {
            let changed = scene.view.hover != hover;
            scene.view.hover = hover;
            changed
        });
        if changed {
            invalidate(hwnd);
        }
    }

    fn hit_at(&self, lp: LPARAM) -> Option<Hit> {
        // Signed client coordinates.
        let (x, y) = ((lp.0 & 0xFFFF) as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as i16 as f32);
        self.scene.try_borrow().ok().and_then(|scene| hit_test(&scene.layout, x, y))
    }
}

fn create_target(d2d: &ID2D1Factory, hwnd: HWND) -> windows::core::Result<ID2D1HwndRenderTarget> {
    let mut rc = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rc)? };
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        // Premultiplied alpha so transparent pixels reveal the system backdrop.
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        // Draw in physical pixels; the layout applies the DPI scale itself.
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

/// The `Shared` of a panel window. The reference is only valid while the
/// window procedure runs: `PanelWindow::drop` clears the pointer before the
/// box is freed, so it must not be kept across messages.
fn shared_of(hwnd: HWND) -> Option<&'static Shared> {
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Shared;
    unsafe { ptr.as_ref() }
}

/// Queues `key` for the owner (outside this window procedure: Enter may close the panel).
fn type_key(hwnd: HWND, shared: Option<&Shared>, key: Option<Key>) {
    if let (Some(shared), Some(key)) = (shared, key)
        && let Ok(mut keys) = shared.keys.try_borrow_mut()
    {
        keys.push(key);
        // The owner drains the whole queue; a later message finds it empty.
        post_owner(hwnd, WM_PANEL_KEY);
    }
}

fn post_owner(hwnd: HWND, msg: u32) {
    if let Some(shared) = shared_of(hwnd) {
        let _ = unsafe { PostMessageW(Some(shared.owner), msg, WPARAM(hwnd.0 as usize), LPARAM(0)) };
    }
}

unsafe extern "system" fn panel_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let shared = shared_of(hwnd);
    match msg {
        WM_PAINT => unsafe {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            if let Some(shared) = shared {
                shared.paint(hwnd);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        },
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            if let Some(shared) = shared {
                shared.resize_target((lp.0 & 0xFFFF) as u32, ((lp.0 >> 16) & 0xFFFF) as u32);
            }
            LRESULT(0)
        }
        // The layout is DPI-driven: re-lay out and resize to it (ignoring the suggested rect).
        WM_DPICHANGED => {
            if let Some(shared) = shared {
                let dpi = (wp.0 & 0xFFFF) as u32;
                shared.update(hwnd, |view| view.dpi = dpi);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(shared) = shared {
                if !shared.tracking.get() {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    shared.tracking.set(unsafe { TrackMouseEvent(&mut tme) }.is_ok());
                }
                shared.set_hover(hwnd, shared.hit_at(lp));
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(shared) = shared {
                shared.tracking.set(false);
                shared.set_hover(hwnd, None);
            }
            LRESULT(0)
        }
        // Handled by the owner, outside this window procedure: a click may close the panel.
        WM_LBUTTONUP => {
            if let Some(shared) = shared
                && let Some(hit) = shared.hit_at(lp)
            {
                shared.click.set(Some(hit));
                post_owner(hwnd, WM_PANEL_HIT);
            }
            LRESULT(0)
        }
        // Right-clicking a toggle opens its settings page (the "more" side).
        WM_RBUTTONUP => {
            if let Some(shared) = shared {
                let more = match shared.hit_at(lp) {
                    Some(Hit::Wifi) => Some(Hit::WifiMore),
                    Some(Hit::Bluetooth) => Some(Hit::BtMore),
                    _ => None,
                };
                if let Some(hit) = more {
                    shared.click.set(Some(hit));
                    post_owner(hwnd, WM_PANEL_HIT);
                }
            }
            LRESULT(0)
        }
        // If Windows refused us the foreground on open, a click takes it.
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            post_owner(hwnd, WM_PANEL_CLOSED);
            LRESULT(0)
        }
        WM_CHAR => {
            type_key(hwnd, shared, char_key(wp.0));
            LRESULT(0)
        }
        WM_KEYDOWN if vk_key(wp.0).is_some() => {
            type_key(hwnd, shared, vk_key(wp.0));
            LRESULT(0)
        }
        WM_ACTIVATE if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE => {
            post_owner(hwnd, WM_PANEL_CLOSED);
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
    shared: Box<Shared>,
}

impl PanelWindow {
    /// Creates, shows and focuses the panel on the primary monitor, showing
    /// `view` (its `dpi` is set here). Close requests and clicks are posted
    /// to `owner` as `WM_PANEL_CLOSED` / `WM_PANEL_HIT`.
    pub fn open(owner: HWND, mut view: PanelView) -> windows::core::Result<Self> {
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

            view.dpi = primary_dpi();
            view.hover = None;
            let layout = layout::layout(&view);
            let r = rect_for(layout.size);
            let shared = Box::new(Shared {
                owner,
                surface: RefCell::new(Surface {
                    d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                    dw: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                    rt: None,
                    cache: draw::Cache::new(),
                }),
                scene: RefCell::new(Scene { view, layout }),
                click: Cell::new(None),
                keys: RefCell::new(Vec::new()),
                tracking: Cell::new(false),
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
            Ok(Self { hwnd, previous, shared })
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Shows `view` (drawn on the next `WM_PAINT`), resizing the window if its
    /// layout changed size. The panel keeps its own `dpi` and `hover`.
    pub fn redraw(&mut self, view: PanelView) {
        self.shared.update(self.hwnd, move |current| {
            let (dpi, hover) = (current.dpi, current.hover);
            *current = PanelView { dpi, hover, ..view };
        });
    }

    /// Don't give the focus back on close: an app launched from the panel is
    /// about to take it, and the previous window must not jump over it.
    pub fn forget_previous(&mut self) {
        self.previous = HWND::default();
    }

    /// The click announced by the last `WM_PANEL_HIT`, once.
    pub fn take_click(&self) -> Option<Hit> {
        self.shared.click.take()
    }

    /// The keys announced by the last `WM_PANEL_KEY`, in typing order.
    pub fn take_keys(&self) -> Vec<Key> {
        self.shared.keys.try_borrow_mut().map(|mut k| std::mem::take(&mut *k)).unwrap_or_default()
    }
}

impl Drop for PanelWindow {
    fn drop(&mut self) {
        unsafe {
            // Detach first: closing must not post another WM_PANEL_CLOSED, and
            // the window procedure must not touch `shared` once it is freed.
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            // Give the focus back only if we still hold it: after a click
            // outside, the clicked window keeps it.
            if GetForegroundWindow() == self.hwnd && !self.previous.is_invalid() && IsWindow(Some(self.previous)).as_bool() {
                let _ = SetForegroundWindow(self.previous);
            }
            let _ = DestroyWindow(self.hwnd);
        }
        // `shared` (render target and factories) is released right after.
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
    fn top_edge_only_on_primary_top_row() {
        assert!(on_top_edge(POINT { x: 960, y: 0 }, MONITOR));
        assert!(!on_top_edge(POINT { x: 960, y: 1 }, MONITOR)); // inside, below the top
        assert!(!on_top_edge(POINT { x: 960, y: -500 }, MONITOR)); // monitor stacked above
        assert!(!on_top_edge(POINT { x: 1920, y: 0 }, MONITOR)); // right of the primary
        assert!(!on_top_edge(POINT { x: -1, y: 0 }, MONITOR));
    }

    #[test]
    fn typed_keys() {
        assert_eq!(char_key('a' as usize), Some(Key::Char('a')));
        assert_eq!(char_key('ã' as usize), Some(Key::Char('ã')));
        assert_eq!(char_key(' ' as usize), Some(Key::Char(' ')));
        // Control characters (Backspace, Enter, Esc, Ctrl+Backspace) and lone surrogates.
        for unit in [0x08, 0x0D, 0x1B, 0x7F, 0xD83D] {
            assert_eq!(char_key(unit), None, "{unit:#x}");
        }
        assert_eq!(vk_key(VK_BACK.0 as usize), Some(Key::Backspace));
        assert_eq!(vk_key(VK_RETURN.0 as usize), Some(Key::Enter));
        assert_eq!(vk_key(VK_UP.0 as usize), Some(Key::Up));
        assert_eq!(vk_key(VK_DOWN.0 as usize), Some(Key::Down));
        assert_eq!(vk_key(VK_ESCAPE.0 as usize), None); // closes, handled apart
    }

    #[test]
    fn panel_clamped_on_small_monitor() {
        let small = RECT { left: 100, top: -50, right: 700, bottom: 400 };
        let r = panel_rect(small, 760, 420);
        assert_eq!((r.left, r.top, r.right), (100, -50, 700));
    }
}
