//! The hidden owner window: holds all state and reacts to readers, timers,
//! clicks and shell broadcasts.
use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::model::{ALL, DeviceKind, ReadOutcome, Tracker};
use crate::panel::trigger::{CloseRule, EdgeTrigger, Pauses};
use crate::panel::view::{Hit, PanelView, Tab};
use crate::panel::{PanelWindow, WM_PANEL_CLOSED, WM_PANEL_HIT};
use crate::readers::{self, WM_READING};
use crate::render::Strip;
use crate::taskbar::{StripWindow, WM_STRIP_CLICK};
use crate::tray::{self, Tray, WM_TRAY};
use crate::{startup, taskbar, theme};

const TIMER_SYNC: usize = 1; // 1 s: keep the strip placed and on top
/// Posted by the WinEvent hooks when the taskbar moves (auto-hide slide,
/// resize) or the foreground window changes (fullscreen check).
const WM_TASKBAR_MOVED: u32 = WM_APP + 5;

static OWNER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TASKBAR: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe extern "system" fn on_location_change(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    use std::sync::atomic::Ordering::Relaxed;
    let foreground_changed = event == EVENT_SYSTEM_FOREGROUND;
    if foreground_changed || (id_object == OBJID_WINDOW.0 && hwnd.0 as isize == TASKBAR.load(Relaxed)) {
        let owner = HWND(OWNER.load(Relaxed) as _);
        let _ = unsafe { PostMessageW(Some(owner), WM_TASKBAR_MOVED, WPARAM(0), LPARAM(0)) };
    }
}

/// Watches the taskbar's position so the strip follows auto-hide immediately
/// instead of waiting for the 1 s timer. Re-run after Explorer restarts.
fn hook_taskbar_moves(previous: Option<HWINEVENTHOOK>) -> Option<HWINEVENTHOOK> {
    use std::sync::atomic::Ordering::Relaxed;
    unsafe {
        if let Some(h) = previous {
            let _ = UnhookWinEvent(h);
        }
        let tb = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok()?;
        TASKBAR.store(tb.0 as isize, Relaxed);
        let mut pid = 0u32;
        let thread = GetWindowThreadProcessId(tb, Some(&mut pid));
        let hook = SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(on_location_change),
            pid,
            thread,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        (!hook.is_invalid()).then_some(hook)
    }
}
const TIMER_EXPIRE: usize = 2; // 60 s: let held values expire
const TIMER_EDGE: usize = 3; // 100 ms: watch the top edge and the open panel
// Reserved for the panel: TIMER_STATS = 4, TIMER_MEDIA = 5.
const TRAY_GLYPH: char = '\u{E83F}';
const TRAY_ICON_PX: i32 = 32;

struct App {
    tracker: Tracker,
    strip: StripWindow,
    tray: Tray,
    rx: Receiver<(DeviceKind, ReadOutcome)>,
    refresh: Vec<Sender<()>>,
    taskbar_created: u32,
    dark: bool,
    hook: Option<HWINEVENTHOOK>,
    panel: Option<PanelWindow>,
    edge: EdgeTrigger,
    close: CloseRule,
    tab: Tab,
    gear_open: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

impl App {
    fn rebuild(&mut self) {
        let now = Instant::now();
        let items = ALL.iter().map(|&k| (k, self.tracker.view(k, now))).collect();
        self.strip.set_strip(Strip { items, dark: self.dark });
        self.tray.set_tip(&self.tracker.tooltip(now));
        self.refresh_panel();
    }

    fn menu(&mut self, hwnd: HWND) {
        match tray::show_menu(hwnd, startup::is_enabled()) {
            tray::CMD_REFRESH => {
                for r in &self.refresh {
                    let _ = r.send(());
                }
            }
            tray::CMD_PANEL => self.open_panel(hwnd),
            tray::CMD_STARTUP => {
                let _ = startup::set_enabled(!startup::is_enabled());
            }
            // Destroying the window here would re-enter owner_proc while APP is
            // borrowed; close it once this handler has returned.
            tray::CMD_QUIT => unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            _ => {}
        }
    }

    fn open_panel(&mut self, hwnd: HWND) {
        if self.panel.is_some() {
            return;
        }
        // Every opening starts on the first tab, with the colors row closed.
        self.tab = Tab::Resources;
        self.gear_open = false;
        if let Ok(panel) = PanelWindow::open(hwnd, self.panel_view()) {
            self.panel = Some(panel);
            self.close = CloseRule::new(); // disarmed until the cursor enters
        }
    }

    /// Everything the panel shows, from the current state.
    fn panel_view(&self) -> PanelView {
        let now = Instant::now();
        PanelView {
            battery: ALL.iter().map(|&k| (k, self.tracker.view(k, now))).collect(),
            tab: self.tab,
            gear_open: self.gear_open,
            ..Default::default()
        }
    }

    fn refresh_panel(&mut self) {
        if self.panel.is_none() {
            return;
        }
        let view = self.panel_view();
        if let Some(panel) = self.panel.as_mut() {
            panel.redraw(view);
        }
    }

    /// A click on the panel. The other hits are wired by the tasks that give them data.
    fn on_hit(&mut self, hit: Hit) {
        match hit {
            Hit::Tab(tab) => self.tab = tab,
            Hit::Gear => self.gear_open = !self.gear_open,
            _ => return,
        }
        self.refresh_panel();
    }

    fn close_panel(&mut self) {
        if self.panel.is_none() {
            return;
        }
        self.panel = None;
        // Hand the pages the panel touched back to the system.
        unsafe {
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        }
    }

    /// Every `TIMER_EDGE` tick: open on a held top edge, close once the cursor left.
    fn edge_tick(&mut self, hwnd: HWND) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return; // secure desktop (lock screen, UAC)
        }
        let at_top_edge = crate::panel::on_top_edge(pt, taskbar::primary_monitor_rect());
        let button_down =
            unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 || GetAsyncKeyState(VK_RBUTTON.0 as i32) < 0 };
        let now = Instant::now();
        if self.edge.update(at_top_edge, button_down, now) && self.panel.is_none() && !taskbar::fullscreen_now() {
            self.open_panel(hwnd);
        }
        let Some(panel) = &self.panel else { return };
        let mut r = RECT::default();
        let _ = unsafe { GetWindowRect(panel.hwnd(), &mut r) };
        let inside = pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom;
        // Pauses come from the panel's state (search text, speed test, popups) once it has one.
        if self.close.update(inside, Pauses::default(), now) {
            self.close_panel();
        }
    }
}

/// Removes the app state (dropping it removes the tray icon).
fn take_app() -> Option<App> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut app| app.take()))
}

fn with_app(f: impl FnOnce(&mut App)) {
    // Menus run a nested message loop; skip re-entrant messages instead of panicking.
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
        {
            f(app);
        }
    });
}

fn is_click(msg: u32) -> bool {
    msg == WM_LBUTTONUP || msg == WM_RBUTTONUP
}

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_READING => with_app(|app| {
            let now = Instant::now();
            while let Ok((kind, outcome)) = app.rx.try_recv() {
                app.tracker.update(kind, outcome, now);
            }
            app.rebuild();
        }),
        WM_TIMER if wp.0 == TIMER_SYNC => with_app(|app| app.strip.sync()),
        WM_TIMER if wp.0 == TIMER_EXPIRE => with_app(|app| app.rebuild()),
        WM_TIMER if wp.0 == TIMER_EDGE => with_app(|app| app.edge_tick(hwnd)),
        // Ignore a late request from a panel that is already gone.
        WM_PANEL_CLOSED => with_app(|app| {
            if app.panel.as_ref().is_some_and(|p| p.hwnd().0 as usize == wp.0) {
                app.close_panel();
            }
        }),
        WM_PANEL_HIT => with_app(|app| {
            if let Some(hit) = app.panel.as_ref().and_then(|p| p.take_click()) {
                app.on_hit(hit);
            }
        }),
        WM_STRIP_CLICK => with_app(|app| app.menu(hwnd)),
        WM_TRAY if is_click(lp.0 as u32) => with_app(|app| app.menu(hwnd)),
        WM_SETTINGCHANGE => {
            let what = if lp.0 != 0 { unsafe { PCWSTR(lp.0 as _).to_string().unwrap_or_default() } } else { String::new() };
            if what == "ImmersiveColorSet" {
                with_app(|app| {
                    app.dark = theme::taskbar_is_dark();
                    let icon = app.strip.renderer().render_glyph_icon(TRAY_GLYPH, app.dark, TRAY_ICON_PX);
                    app.tray.set_icon(icon);
                    app.rebuild();
                });
            }
        }
        WM_TASKBAR_MOVED => with_app(|app| app.strip.sync()),
        WM_DISPLAYCHANGE | WM_DPICHANGED => with_app(|app| {
            if msg == WM_DISPLAYCHANGE {
                app.close_panel();
            }
            app.strip.invalidate();
            app.strip.sync();
        }),
        WM_DESTROY => {
            drop(take_app()); // drops the tray icon
            unsafe { PostQuitMessage(0) };
        }
        m if m != 0 && APP.with(|c| c.try_borrow().ok().and_then(|a| a.as_ref().map(|a| a.taskbar_created))) == Some(m) => {
            with_app(|app| {
                app.tray.readd();
                app.hook = hook_taskbar_moves(app.hook.take());
                app.strip.invalidate();
                app.strip.sync();
            });
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

pub fn run() -> windows::core::Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class = w!("BatteryBarOwner");
        let wc = WNDCLASSW { lpfnWndProc: Some(owner_proc), hInstance: hinstance.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        // A real (invisible) top-level window: message-only windows miss shell broadcasts.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
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

        let strip = StripWindow::create(hwnd)?;
        let dark = theme::taskbar_is_dark();
        let tray = Tray::new(hwnd, strip.renderer().render_glyph_icon(TRAY_GLYPH, dark, TRAY_ICON_PX));
        let (tx, rx) = mpsc::channel();
        let refresh = readers::spawn_all(hwnd.0 as isize, tx);
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        // A game going fullscreen changes the foreground: re-check right away
        // instead of waiting for the 1 s timer. Lives as long as the process.
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_location_change),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        OWNER.store(hwnd.0 as isize, std::sync::atomic::Ordering::Relaxed);
        let hook = hook_taskbar_moves(None);
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App {
                tracker: Tracker::new(),
                strip,
                tray,
                rx,
                refresh,
                taskbar_created,
                dark,
                hook,
                panel: None,
                edge: EdgeTrigger::new(),
                close: CloseRule::new(),
                tab: Tab::Resources,
                gear_open: false,
            });
        });
        with_app(|app| app.rebuild());

        SetTimer(Some(hwnd), TIMER_SYNC, 1000, None);
        SetTimer(Some(hwnd), TIMER_EXPIRE, 60_000, None);
        SetTimer(Some(hwnd), TIMER_EDGE, 100, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_app_while_borrowed_does_not_panic() {
        // WM_DESTROY can arrive while a handler still holds APP (DestroyWindow
        // from inside the tray menu); dropping state must not panic there.
        APP.with(|cell| {
            let _held = cell.borrow_mut();
            assert!(take_app().is_none());
        });
    }
}
