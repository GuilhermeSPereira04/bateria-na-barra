//! The top panel: a popup with the Windows 11 system backdrop (frosted glass)
//! drawn over with Direct2D. Created on open and destroyed on close, so no
//! drawing resources stay allocated while it is closed.
pub mod anim;
pub mod draw;
pub mod glass;
pub mod island;
pub mod layout;
pub mod trigger;
pub mod view;

use std::cell::{Cell, RefCell};
use std::time::Instant;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWA_TRANSITIONS_FORCEDISABLED, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateRoundRectRgn, EndPaint, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint, PAINTSTRUCT, ScreenToClient,
    SetWindowRgn, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, SetProcessWorkingSetSize};
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VIRTUAL_KEY, VK_BACK, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

use layout::{Layout, hit_test};
use view::{Hit, Key, PanelView, SliderKind};

/// Posted to the owner by panel worker threads when new data is ready
/// (wParam: which source, see `ASYNC_*`).
pub const WM_PANEL_ASYNC: u32 = WM_APP + 10;
/// `WM_PANEL_ASYNC` source: the app list for the search.
pub const ASYNC_APPS: usize = 1;
/// `WM_PANEL_ASYNC` source: the Wi-Fi and Bluetooth state.
pub const ASYNC_RADIOS: usize = 2;
/// `WM_PANEL_ASYNC` source: what is playing.
pub const ASYNC_MEDIA: usize = 3;
/// `WM_PANEL_ASYNC` source: speed test progress or outcome.
pub const ASYNC_SPEED: usize = 4;
/// `WM_PANEL_ASYNC` source: the audio outputs and the default one's state.
pub const ASYNC_AUDIO: usize = 5;
/// `WM_PANEL_ASYNC` source: a CPU/GPU/RAM sample for the Resources tab.
pub const ASYNC_STATS: usize = 6;
/// Posted to the owner when the panel wants to close itself (Esc, click
/// outside, lost activation). wParam: the panel's HWND.
pub const WM_PANEL_CLOSED: u32 = WM_APP + 11;
/// Posted to the owner after a click on something clickable; the owner
/// collects it with `PanelWindow::take_click`. wParam: the panel's HWND.
pub const WM_PANEL_HIT: u32 = WM_APP + 12;
/// Posted to the owner after keys were typed; the owner collects them with
/// `PanelWindow::take_keys`. wParam: the panel's HWND.
pub const WM_PANEL_KEY: u32 = WM_APP + 13;
/// Posted to the owner after the wheel turned over the audio toggle; the
/// owner collects the notches with `PanelWindow::take_wheel`. wParam: the panel's HWND.
pub const WM_PANEL_WHEEL: u32 = WM_APP + 14;

/// Posted to the owner while a bar is dragged; the owner collects the values
/// with `PanelWindow::take_drags`. wParam: the panel's HWND.
pub const WM_PANEL_DRAG: u32 = WM_APP + 21;

/// Posted to the owner when a closing animation reached the pill. wParam: the panel's HWND.
pub const WM_PANEL_ANIM_DONE: u32 = WM_APP + 22;

/// Timer of the open/close animation, on the panel's own window.
const ANIM_TIMER: usize = 1;

/// Whole wheel notches in an accumulated `WM_MOUSEWHEEL` delta, and the rest
/// (fine-grained wheels and touchpads send fractions of a notch).
fn wheel_notches(delta: i32) -> (i32, i32) {
    (delta / WHEEL_DELTA as i32, delta % WHEEL_DELTA as i32)
}

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

/// Open/close animation: `t` is linear progress (0 = pill, 1 = full panel).
#[derive(Clone, Copy)]
struct Anim {
    t: f32,
    opening: bool,
    last: Instant,
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
    /// Wheel delta over the audio toggle not yet taken by the owner.
    wheel: Cell<i32>,
    /// `TrackMouseEvent` is armed (a `WM_MOUSELEAVE` will come).
    tracking: Cell<bool>,
    /// The bar being dragged (the mouse is captured) and its last value.
    drag: Cell<Option<(SliderKind, u8)>>,
    /// Bar values since the owner last looked: (bar, value, button released).
    drags: RefCell<Vec<(SliderKind, u8, bool)>>,
    /// The acrylic backdrop is on, and the (rgb, alpha) it was applied with.
    glass: Cell<Option<(u32, f32)>>,
    anim: Cell<Anim>,
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

    /// Runs the animation toward open (true) or closed (false) from wherever it is.
    fn start_anim(&self, hwnd: HWND, opening: bool) {
        let mut a = self.anim.get();
        a.opening = opening;
        a.last = Instant::now();
        self.anim.set(a);
        unsafe { SetTimer(Some(hwnd), ANIM_TIMER, anim::FRAME_MS, None) };
        self.apply_anim(hwnd);
    }

    fn tick(&self, hwnd: HWND) {
        let mut a = self.anim.get();
        let now = Instant::now();
        a.t = anim::advance(a.t, a.opening, now.duration_since(a.last).as_secs_f32() * 1000.0);
        a.last = now;
        self.anim.set(a);
        let done = a.t == if a.opening { 1.0 } else { 0.0 };
        if done {
            let _ = unsafe { KillTimer(Some(hwnd), ANIM_TIMER) };
            if a.opening {
                // The frames of the animation left freed heap behind: hand it back.
                unsafe {
                    let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                }
            }
        }
        self.apply_anim(hwnd);
        if done && !a.opening {
            post_owner(hwnd, WM_PANEL_ANIM_DONE);
        }
    }

    /// Sizes the window to the current shape (between the island's pill and
    /// the full panel), keeps the content centered in it, rounds its corners
    /// and repaints with the veil.
    fn apply_anim(&self, hwnd: HWND) {
        let t = self.anim.get().t;
        let e = anim::ease_out(t);
        let Ok(mut scene) = self.scene.try_borrow_mut() else { return };
        let dpi = scene.view.dpi;
        let full = rect_for(scene.layout.size);
        let pill = island::island_rect(crate::taskbar::primary_monitor_rect(), dpi);
        let r = if t >= 1.0 { full } else { anim::lerp_rect(pill, full, e) };
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        scene.view.veil = anim::veil(e);
        scene.view.shift = ((w - (full.right - full.left)) / 2) as f32;
        drop(scene);
        unsafe {
            let _ = SetWindowPos(hwnd, None, r.left, r.top, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            if t >= 1.0 {
                SetWindowRgn(hwnd, None, true);
            } else {
                // The acrylic ignores the region, but the drawing doesn't: this rounds the pill.
                let radius = (island::pill_size(dpi).1 as f32 / 2.0 * (1.0 - e) + 8.0 * layout::scale(dpi) * e).round() as i32;
                let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, 2 * radius, 2 * radius);
                SetWindowRgn(hwnd, Some(rgn), true); // the system owns the region now
            }
        }
        invalidate(hwnd);
        unsafe {
            let _ = UpdateWindow(hwnd);
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

    /// The value of bar `kind` for a pointer at client `x`.
    fn slider_at(&self, kind: SliderKind, x: f32) -> Option<u8> {
        let scene = self.scene.try_borrow().ok()?;
        scene.layout.rect(Hit::Slider(kind)).map(|track| layout::slider_value(track, x))
    }

    /// Queues a bar value for the owner. A move replaces a pending move of the same bar.
    fn push_drag(&self, hwnd: HWND, kind: SliderKind, value: u8, last: bool) {
        if let Ok(mut q) = self.drags.try_borrow_mut() {
            if !last && matches!(q.last(), Some((k, _, false)) if *k == kind) {
                q.pop();
            }
            q.push((kind, value, last));
            post_owner(hwnd, WM_PANEL_DRAG);
        }
    }

    /// Turns the acrylic on for `tint` (or updates it); false keeps the system backdrop.
    fn apply_glass(&self, hwnd: HWND, tint: (u32, f32)) {
        if self.glass.get() == Some(tint) {
            return;
        }
        if glass::apply(hwnd, tint.0, tint.1) {
            if self.glass.get().is_none() {
                unsafe {
                    let none = DWMSBT_NONE.0;
                    let _ = DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &none as *const i32 as _, size_of::<i32>() as u32);
                }
            }
            self.glass.set(Some(tint));
            if let Ok(mut scene) = self.scene.try_borrow_mut() {
                scene.view.acrylic = true;
            }
        }
    }

    fn hit_at(&self, lp: LPARAM) -> Option<Hit> {
        // Signed client coordinates.
        self.hit_at_point((lp.0 & 0xFFFF) as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as i16 as f32)
    }

    fn hit_at_point(&self, x: f32, y: f32) -> Option<Hit> {
        self.scene.try_borrow().ok().and_then(|scene| hit_test(&scene.layout, x, y))
    }
}

fn create_target(d2d: &ID2D1Factory, hwnd: HWND) -> windows::core::Result<ID2D1HwndRenderTarget> {
    let mut rc = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rc)? };
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
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
    // While it moves, the mouse does nothing (the content is shifted); once it folds back, nothing does.
    let mouse = matches!(msg, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONUP | WM_MOUSEWHEEL);
    let keys = matches!(msg, WM_KEYDOWN | WM_CHAR | WM_ACTIVATE);
    if let Some(a) = shared.map(|s| s.anim.get())
        && ((mouse && (!a.opening || a.t < 1.0)) || (keys && !a.opening))
    {
        return LRESULT(0);
    }
    match msg {
        WM_TIMER if wp.0 == ANIM_TIMER => {
            if let Some(shared) = shared {
                shared.tick(hwnd);
            }
            LRESULT(0)
        }
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
                if let Some((kind, _)) = shared.drag.get()
                    && let Some(v) = shared.slider_at(kind, (lp.0 & 0xFFFF) as i16 as f32)
                {
                    shared.drag.set(Some((kind, v)));
                    shared.push_drag(hwnd, kind, v, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(shared) = shared
                && let Some(Hit::Slider(kind)) = shared.hit_at(lp)
                && let Some(v) = shared.slider_at(kind, (lp.0 & 0xFFFF) as i16 as f32)
            {
                unsafe { SetCapture(hwnd) };
                shared.drag.set(Some((kind, v)));
                shared.push_drag(hwnd, kind, v, false);
            }
            LRESULT(0)
        }
        // Capture lost without a release (another window took it): end the drag.
        WM_CAPTURECHANGED => {
            if let Some(shared) = shared
                && let Some((kind, v)) = shared.drag.take()
            {
                shared.push_drag(hwnd, kind, v, true);
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
                && let Some((kind, _)) = shared.drag.take()
            {
                let v = shared.slider_at(kind, (lp.0 & 0xFFFF) as i16 as f32).unwrap_or(0);
                shared.push_drag(hwnd, kind, v, true);
                let _ = unsafe { ReleaseCapture() };
                return LRESULT(0); // a drag is not a click
            }
            if let Some(shared) = shared {
                // A click on nothing still counts: it closes the audio list.
                shared.click.set(Some(shared.hit_at(lp).unwrap_or(Hit::Blank)));
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
                    // The audio arrow opens the output list; the settings page is right-click only.
                    Some(Hit::Audio | Hit::AudioMore) => Some(Hit::AudioSettings),
                    _ => None,
                };
                if let Some(hit) = more {
                    shared.click.set(Some(hit));
                    post_owner(hwnd, WM_PANEL_HIT);
                }
            }
            LRESULT(0)
        }
        // The wheel over the audio toggle sets the volume (handled by the owner).
        // Unlike clicks, WM_MOUSEWHEEL carries screen coordinates.
        WM_MOUSEWHEEL => {
            let mut pt = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
            let over_audio = shared.is_some_and(|shared| {
                unsafe { ScreenToClient(hwnd, &mut pt) }.as_bool()
                    && matches!(
                        shared.hit_at_point(pt.x as f32, pt.y as f32),
                        Some(Hit::Audio | Hit::AudioMore | Hit::AudioDevice(_) | Hit::Slider(SliderKind::Volume))
                    )
            });
            if let Some(shared) = shared.filter(|_| over_audio) {
                let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as i32;
                shared.wheel.set(shared.wheel.get().saturating_add(delta));
                post_owner(hwnd, WM_PANEL_WHEEL);
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
            }
        }
        // If Windows refused us the foreground on open, a click takes it.
        WM_MOUSEACTIVATE => LRESULT(MA_ACTIVATE as isize),
        // lParam 1 = Esc: the owner closes the audio list first if it is open.
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            if let Some(shared) = shared {
                let _ = unsafe { PostMessageW(Some(shared.owner), WM_PANEL_CLOSED, WPARAM(hwnd.0 as usize), LPARAM(1)) };
            }
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

pub(crate) fn primary_dpi() -> u32 {
    let mon = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let (mut dx, mut dy) = (96, 96);
    if unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) }.is_ok() { dx.max(96) } else { 96 }
}

/// Moves the keyboard focus to `hwnd` without injecting input: borrow the
/// active thread's input state so `SetForegroundWindow` is allowed. A hung
/// foreground thread (attaching could freeze ours) or one of our own process
/// is not attached; `SetForegroundWindow` is then just tried alone.
fn take_focus(hwnd: HWND, previous: HWND) {
    unsafe {
        let mut pid = 0u32;
        let fg_thread = if previous.is_invalid() { 0 } else { GetWindowThreadProcessId(previous, Some(&mut pid)) };
        let me = GetCurrentThreadId();
        let attach = fg_thread != 0
            && fg_thread != me
            && pid != GetCurrentProcessId()
            && !IsHungAppWindow(previous).as_bool();
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

/// Gives the focus back to `previous` only if `hwnd` still holds it: after a
/// click outside, the clicked window keeps it.
fn restore_focus(hwnd: HWND, previous: HWND) {
    unsafe {
        if GetForegroundWindow() == hwnd && !previous.is_invalid() && IsWindow(Some(previous)).as_bool() {
            let _ = SetForegroundWindow(previous);
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
            view.veil = 1.0;
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
                wheel: Cell::new(0),
                tracking: Cell::new(false),
                drag: Cell::new(None),
                drags: RefCell::new(Vec::new()),
                glass: Cell::new(None),
                anim: Cell::new(Anim { t: 0.0, opening: true, last: Instant::now() }),
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
            // Our own animation replaces the system's fade/scale on show and destroy.
            set(DWMWA_TRANSITIONS_FORCEDISABLED, 1);
            set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0);
            let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);

            // Acrylic with the chosen tint; the system backdrop stays if that fails.
            shared.apply_glass(hwnd, shared.scene.try_borrow().map(|s| s.view.colors.tint).unwrap_or((0x20242C, 0.55)));

            let previous = GetForegroundWindow();
            // Starts as the island's pill and unfolds.
            shared.start_anim(hwnd, true);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
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
        let tint = view.colors.tint;
        self.shared.update(self.hwnd, move |current| {
            let (dpi, hover, acrylic, veil, shift) = (current.dpi, current.hover, current.acrylic, current.veil, current.shift);
            *current = PanelView { dpi, hover, acrylic, veil, shift, ..view };
        });
        self.shared.apply_glass(self.hwnd, tint);
    }

    /// Folds the panel back into the pill (the owner drops it on
    /// `WM_PANEL_ANIM_DONE`, or when `finished`). Gives the focus back right away.
    pub fn begin_close(&mut self) {
        restore_focus(self.hwnd, self.previous);
        self.previous = HWND::default();
        self.shared.start_anim(self.hwnd, false);
    }

    /// Unfolds a panel that was folding back (reopened before it reached the pill).
    pub fn begin_open(&mut self, owner: HWND) {
        let previous = unsafe { GetForegroundWindow() };
        self.shared.start_anim(self.hwnd, true);
        take_focus(self.hwnd, previous);
        self.previous = if previous == owner || previous == self.hwnd { HWND::default() } else { previous };
    }

    /// True once a closing animation reached the pill.
    pub fn finished(&self) -> bool {
        let a = self.shared.anim.get();
        !a.opening && a.t <= 0.0
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

    /// Whole wheel notches turned over the audio toggle since the last call
    /// (positive = away from the user); the remainder waits for more.
    pub fn take_wheel(&self) -> i32 {
        let (notches, rest) = wheel_notches(self.shared.wheel.get());
        self.shared.wheel.set(rest);
        notches
    }

    /// The bar values announced by the last `WM_PANEL_DRAG`, in order:
    /// (bar, value, button released).
    pub fn take_drags(&self) -> Vec<(SliderKind, u8, bool)> {
        self.shared.drags.try_borrow_mut().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default()
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
            restore_focus(self.hwnd, self.previous);
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
    fn wheel_counts_whole_notches() {
        assert_eq!(wheel_notches(120), (1, 0));
        assert_eq!(wheel_notches(-240), (-2, 0));
        assert_eq!(wheel_notches(60), (0, 60)); // a fine-grained touchpad: wait for more
        assert_eq!(wheel_notches(180), (1, 60));
        assert_eq!(wheel_notches(-60), (0, -60));
    }

    #[test]
    fn panel_clamped_on_small_monitor() {
        let small = RECT { left: 100, top: -50, right: 700, bottom: 400 };
        let r = panel_rect(small, 760, 420);
        assert_eq!((r.left, r.top, r.right), (100, -50, 700));
    }
}
