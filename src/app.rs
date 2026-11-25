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

use crate::apps::{self, AppEntry, Catalog};
use crate::audio::{self, AudioSnapshot};
use crate::calendar;
use crate::config::{self, Accent, Colors, Config, GlassTint};
use crate::media::{self, MediaCmd, NowPlaying};
use crate::model::{ALL, DeviceKind, ReadOutcome, Tracker};
use crate::panel::trigger::{CloseRule, EdgeTrigger, Pauses};
use crate::panel::layout::MAX_ROWS;
use crate::panel::view::{Hit, Key, PanelView, QuickState, Tab, Toggle};
use crate::panel::{
    ASYNC_APPS, ASYNC_AUDIO, ASYNC_MEDIA, ASYNC_RADIOS, ASYNC_STATS, PanelWindow, WM_PANEL_ASYNC, WM_PANEL_CLOSED, WM_PANEL_HIT, WM_PANEL_KEY, WM_PANEL_WHEEL,
};
use crate::readers::{self, WM_READING};
use crate::radios::{self, RadioKind, RadioSnapshot};
use crate::render::Strip;
use crate::sysinfo::{Stats, StatsWorker};
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
const TIMER_STATS: usize = 4; // 1 s with the panel open on the Resources tab: sample
const TIMER_MEDIA: usize = 5; // 2 s while the panel is open: read what is playing
const TRAY_GLYPH: char = '\u{E83F}';
const TRAY_ICON_PX: i32 = 32;
/// Volume change per wheel notch over the audio toggle, in percentage points.
const VOLUME_STEP: i32 = 2;

/// The local date as (year, month, day).
fn today() -> (i32, u32, u32) {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (t.wYear as i32, t.wMonth as u32, t.wDay as u32)
}

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
    /// Month shown on the calendar tab (year, 1..=12).
    month: (i32, u32),
    config: Config,
    colors: Colors,
    /// Search text, its matches and the highlighted one (panel open only).
    query: String,
    results: Vec<AppEntry>,
    selected: usize,
    /// The app list while the panel is open, and its pending enumeration.
    catalog: Option<Catalog>,
    apps_rx: Option<Receiver<Vec<AppEntry>>>,
    /// Last known Wi-Fi/Bluetooth state, and the pending read (panel open only).
    quick: QuickState,
    radios_rx: Option<Receiver<RadioSnapshot>>,
    /// Audio outputs as (id, name), whether their list is open in the body,
    /// the pending read, and whether the volume or mute changed here since
    /// that read started (the local values are then newer than its own).
    audio_outputs: Vec<(String, String)>,
    audio_list_open: bool,
    audio_rx: Option<Receiver<AudioSnapshot>>,
    audio_touched: bool,
    /// What is playing (last read) and the pending read (panel open only).
    media: Option<NowPlaying>,
    media_rx: Option<Receiver<Option<NowPlaying>>>,
    /// Last resources sample, its column order, and the sampling thread
    /// (panel open on the Resources tab only).
    stats: Option<Stats>,
    sort_ram: bool,
    sampler: Option<StatsWorker>,
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
        self.sort_ram = false;
        self.gear_open = false;
        let (year, month, _) = today();
        self.month = (year, month);
        self.audio_list_open = false;
        self.config = config::load();
        self.colors = config::resolve(&self.config);
        self.query.clear();
        self.results.clear();
        self.selected = 0;
        if let Ok(panel) = PanelWindow::open(hwnd, self.panel_view()) {
            self.panel = Some(panel);
            self.close = CloseRule::new(); // disarmed until the cursor enters
            self.apps_rx = Some(apps::enumerate_async(hwnd));
            self.radios_rx = Some(radios::spawn(hwnd, || {}));
            self.read_audio(hwnd);
            self.read_media(hwnd, None);
            unsafe { SetTimer(Some(hwnd), TIMER_MEDIA, 2000, None) };
            self.sync_stats(hwnd);
        }
    }

    /// Samples only while the panel is open on the Resources tab: starts the
    /// sampling thread and its timer on entering, drops both on leaving.
    fn sync_stats(&mut self, hwnd: HWND) {
        let wanted = self.panel.is_some() && self.tab == Tab::Resources;
        if wanted && self.sampler.is_none() {
            self.sampler = Some(StatsWorker::spawn(hwnd));
            unsafe { SetTimer(Some(hwnd), TIMER_STATS, 1000, None) };
        } else if !wanted && self.sampler.is_some() {
            self.sampler = None;
            self.stats = None; // the next visit starts from `…`
            let _ = unsafe { KillTimer(Some(hwnd), TIMER_STATS) };
        }
    }

    /// Every `TIMER_STATS` tick.
    fn stats_tick(&mut self) {
        if let Some(sampler) = self.sampler.as_mut() {
            sampler.request();
        }
    }

    /// A sample arrived from the sampling thread.
    fn on_stats(&mut self) {
        let Some(stats) = self.sampler.as_mut().and_then(|s| s.take()) else { return };
        self.stats = Some(stats);
        self.refresh_panel();
    }

    /// Sends `cmd` if any, then reads what is playing, on a worker thread.
    fn read_media(&mut self, hwnd: HWND, cmd: Option<MediaCmd>) {
        // Replacing the receiver discards a read still in flight.
        self.media_rx = Some(media::spawn(hwnd, cmd, self.media.clone()));
    }

    /// Every `TIMER_MEDIA` tick; skipped while a read is still pending.
    fn media_tick(&mut self, hwnd: HWND) {
        if self.panel.is_some() && self.media_rx.is_none() {
            self.read_media(hwnd, None);
        }
    }

    /// What is playing arrived from its worker thread.
    fn on_media(&mut self) {
        let Some(now) = self.media_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.media_rx = None;
        if self.media != now {
            self.media = now;
            self.refresh_panel();
        }
    }

    /// A player button: only with something playing.
    fn media_command(&mut self, hwnd: HWND, cmd: MediaCmd) {
        let Some(now) = self.media.as_mut() else { return };
        if cmd == MediaCmd::PlayPause {
            now.playing = !now.playing; // shown now; the read confirms it
            self.refresh_panel();
        }
        self.read_media(hwnd, Some(cmd));
    }

    /// Reads the outputs and the default one's state on a worker thread.
    fn read_audio(&mut self, hwnd: HWND) {
        self.audio_touched = false;
        // Replacing the receiver discards a read still in flight.
        self.audio_rx = Some(audio::spawn(hwnd));
    }

    /// The audio state arrived from its worker thread.
    fn on_audio(&mut self) {
        let Some(snap) = self.audio_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.audio_rx = None;
        self.audio_outputs = snap.outputs.clone();
        if !self.audio_touched {
            self.show_audio(snap);
        }
        self.refresh_panel();
    }

    fn show_audio(&mut self, snap: AudioSnapshot) {
        self.quick.audio = snap.state;
        self.quick.volume = snap.volume;
        self.quick.output_name = snap.output;
    }

    /// After changing the volume or mute here: read the default output back
    /// (fast, on this thread) and redraw.
    fn audio_changed(&mut self) {
        self.audio_touched = true;
        self.show_audio(audio::snapshot_from(audio::endpoint(), Vec::new()));
        self.refresh_panel();
    }

    /// The wheel turned `notches` over the audio toggle (positive = louder).
    fn on_wheel(&mut self, notches: i32) {
        if self.quick.audio == Toggle::Unavailable {
            return;
        }
        audio::step_volume(notches.saturating_mul(VOLUME_STEP));
        self.audio_changed();
    }

    /// Output `i` of the list becomes the default; Settings if Windows refuses.
    fn pick_output(&mut self, hwnd: HWND, i: usize) {
        let Some(id) = self.audio_outputs.get(i).map(|(id, _)| id.clone()) else { return };
        if audio::set_default(&id) {
            self.audio_list_open = false;
            self.read_audio(hwnd);
            self.refresh_panel();
        } else {
            self.open_settings("ms-settings:sound");
        }
    }

    /// The app list arrived from the enumeration thread.
    fn on_apps(&mut self) {
        let Some(list) = self.apps_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.apps_rx = None;
        self.catalog = Some(Catalog::new(list));
        self.search();
        self.refresh_panel();
    }

    /// The Wi-Fi/Bluetooth state arrived from its worker thread.
    fn on_radios(&mut self) {
        let Some(snap) = self.radios_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.radios_rx = None;
        self.quick.wifi = snap.wifi;
        self.quick.wifi_name = snap.wifi_name;
        self.quick.bt = snap.bt;
        self.quick.bt_connected = snap.bt_connected;
        self.refresh_panel();
    }

    /// Flips a radio on a worker thread, then reads the state again.
    fn toggle_radio(&mut self, hwnd: HWND, kind: RadioKind) {
        let current = if kind == RadioKind::Wifi { self.quick.wifi } else { self.quick.bt };
        if current == Toggle::Unavailable {
            return;
        }
        let on = current == Toggle::On;
        // Replacing the receiver discards a read still in flight.
        self.radios_rx = Some(radios::spawn(hwnd, move || radios::set(kind, !on)));
    }

    /// Opens a Settings page; the panel gives way to it.
    fn open_settings(&mut self, uri: &str) {
        radios::open_settings(uri);
        if let Some(panel) = self.panel.as_mut() {
            panel.forget_previous(); // Settings gets the focus
        }
        self.close_panel();
    }

    /// Recomputes the results for the current query, highlighting the first.
    fn search(&mut self) {
        self.selected = 0;
        self.results = match &mut self.catalog {
            Some(catalog) if !self.query.is_empty() => catalog.search(&self.query, MAX_ROWS),
            _ => Vec::new(),
        };
    }

    /// A key typed in the panel (the caller redraws).
    fn on_key(&mut self, key: Key) {
        match key {
            // A leading space would only hide the tabs.
            Key::Char(c) if c.is_whitespace() && self.query.is_empty() => {}
            Key::Char(c) => {
                self.query.push(c);
                self.search();
            }
            Key::Backspace => {
                self.query.pop();
                self.search();
            }
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(self.results.len().saturating_sub(1)),
            Key::Enter => self.launch(self.selected),
        }
    }

    /// Opens result `i` and closes the panel.
    fn launch(&mut self, i: usize) {
        if self.query.is_empty() {
            return;
        }
        let Some(id) = self.results.get(i).map(|a| a.id.clone()) else { return };
        apps::launch(&id);
        if let Some(panel) = self.panel.as_mut() {
            panel.forget_previous(); // the launched app gets the focus
        }
        self.close_panel();
    }

    /// Everything the panel shows, from the current state.
    fn panel_view(&self) -> PanelView {
        let now = Instant::now();
        PanelView {
            battery: ALL.iter().map(|&k| (k, self.tracker.view(k, now))).collect(),
            tab: self.tab,
            gear_open: self.gear_open,
            month: self.month,
            today: today(),
            colors: self.colors,
            query: self.query.clone(),
            results: self.results.clone(),
            selected: self.selected,
            quick: self.quick.clone(),
            media: self.media.clone(),
            stats: self.stats.as_ref().map(|s| s.ranked(self.sort_ram, 5)),
            sort_ram: self.sort_ram,
            audio_list: self.audio_list_open.then(|| self.audio_outputs.iter().map(|(_, name)| name.clone()).collect()),
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
    fn on_hit(&mut self, hwnd: HWND, hit: Hit) {
        match hit {
            Hit::Wifi => return self.toggle_radio(hwnd, RadioKind::Wifi),
            Hit::Bluetooth => return self.toggle_radio(hwnd, RadioKind::Bluetooth),
            Hit::WifiMore => return self.open_settings("ms-settings:network-wifi"),
            Hit::BtMore => return self.open_settings("ms-settings:bluetooth"),
            Hit::Audio if self.quick.audio != Toggle::Unavailable => {
                audio::toggle_mute();
                return self.audio_changed();
            }
            Hit::AudioMore => {
                self.audio_list_open = !self.audio_list_open;
                if self.audio_list_open {
                    self.read_audio(hwnd); // outputs may have come and gone
                }
            }
            Hit::AudioDevice(i) => return self.pick_output(hwnd, i),
            Hit::AudioSettings => return self.open_settings("ms-settings:sound"),
            Hit::MediaPrev => return self.media_command(hwnd, MediaCmd::Prev),
            Hit::MediaPlay => return self.media_command(hwnd, MediaCmd::PlayPause),
            Hit::MediaNext => return self.media_command(hwnd, MediaCmd::Next),
            Hit::Tab(tab) => {
                self.tab = tab;
                self.sync_stats(hwnd);
            }
            Hit::CalPrev => self.month = calendar::shift(self.month, -1),
            Hit::CalNext => self.month = calendar::shift(self.month, 1),
            Hit::SortCpu => self.sort_ram = false,
            Hit::SortRam => self.sort_ram = true,
            Hit::Gear => self.gear_open = !self.gear_open,
            Hit::Accent(i) if i < config::ACCENTS.len() => self.set_config(Config { accent: Accent::Preset(i), ..self.config }),
            Hit::AccentWindows => self.set_config(Config { accent: Accent::Windows, ..self.config }),
            Hit::Tint(i) if i < GlassTint::ALL.len() => self.set_config(Config { tint: GlassTint::ALL[i], ..self.config }),
            Hit::Result(i) => return self.launch(i),
            _ => return,
        }
        self.refresh_panel();
    }

    /// Applies, saves and resolves a new color choice (the caller redraws).
    fn set_config(&mut self, config: Config) {
        self.config = config;
        config::save(&config);
        self.colors = config::resolve(&config);
    }

    fn close_panel(&mut self) {
        if self.panel.is_none() {
            return;
        }
        self.panel = None;
        // The app list and its icons only live while the panel is open.
        self.catalog = None;
        self.apps_rx = None;
        self.radios_rx = None;
        self.audio_rx = None;
        self.media_rx = None;
        let owner = HWND(OWNER.load(std::sync::atomic::Ordering::Relaxed) as _);
        let _ = unsafe { KillTimer(Some(owner), TIMER_MEDIA) };
        self.sync_stats(owner);
        self.audio_list_open = false;
        self.results = Vec::new();
        self.query = String::new();
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
        // The speed test pauses it too, once the panel has it.
        let pauses = Pauses {
            search_text: !self.query.is_empty(),
            popup_open: self.gear_open || self.audio_list_open,
            ..Pauses::default()
        };
        if self.close.update(inside, pauses, now) {
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
        WM_TIMER if wp.0 == TIMER_MEDIA => with_app(|app| app.media_tick(hwnd)),
        WM_TIMER if wp.0 == TIMER_STATS => with_app(|app| app.stats_tick()),
        // Ignore a late request from a panel that is already gone.
        WM_PANEL_CLOSED => with_app(|app| {
            if app.panel.as_ref().is_some_and(|p| p.hwnd().0 as usize == wp.0) {
                app.close_panel();
            }
        }),
        WM_PANEL_HIT => with_app(|app| {
            if let Some(hit) = app.panel.as_ref().and_then(|p| p.take_click()) {
                app.on_hit(hwnd, hit);
            }
        }),
        WM_PANEL_KEY => with_app(|app| {
            let keys = app.panel.as_ref().map(|p| p.take_keys()).unwrap_or_default();
            if keys.is_empty() {
                return;
            }
            for key in keys {
                app.on_key(key);
                if app.panel.is_none() {
                    return; // Enter opened an app
                }
            }
            app.refresh_panel();
        }),
        WM_PANEL_ASYNC if wp.0 == ASYNC_APPS => with_app(|app| app.on_apps()),
        WM_PANEL_ASYNC if wp.0 == ASYNC_RADIOS => with_app(|app| app.on_radios()),
        WM_PANEL_ASYNC if wp.0 == ASYNC_AUDIO => with_app(|app| app.on_audio()),
        WM_PANEL_ASYNC if wp.0 == ASYNC_MEDIA => with_app(|app| app.on_media()),
        WM_PANEL_ASYNC if wp.0 == ASYNC_STATS => with_app(|app| app.on_stats()),
        WM_PANEL_WHEEL => with_app(|app| {
            let notches = app.panel.as_ref().map(|p| p.take_wheel()).unwrap_or(0);
            if notches != 0 {
                app.on_wheel(notches);
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
                    if app.config.accent == Accent::Windows {
                        app.colors = config::resolve(&app.config);
                    }
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
                month: (0, 1),
                config: Config::default(),
                colors: Colors::default(),
                query: String::new(),
                results: Vec::new(),
                selected: 0,
                catalog: None,
                apps_rx: None,
                quick: QuickState::default(),
                radios_rx: None,
                audio_outputs: Vec::new(),
                audio_list_open: false,
                audio_rx: None,
                audio_touched: false,
                media: None,
                media_rx: None,
                stats: None,
                sort_ram: false,
                sampler: None,
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
