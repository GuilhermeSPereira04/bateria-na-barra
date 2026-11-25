//! Everything the panel shows, gathered from the app state (pure data).
use crate::model::{DeviceKind, ItemView};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Resources,
    Calendar,
    Speed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[allow(dead_code)] // Off/On are reported by the radios and audio (Tasks 6, 7)
pub enum Toggle {
    #[default]
    Unavailable,
    Off,
    On,
}

#[derive(Clone, Default)]
pub struct QuickState {
    pub wifi: Toggle,
    pub wifi_name: Option<String>,
    pub bt: Toggle,
    pub bt_connected: u32,
    /// On = sound, Off = muted.
    pub audio: Toggle,
    pub volume: u8,
    pub output_name: Option<String>,
}

/// A key typed in the panel (for the search).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Backspace,
    Up,
    Down,
    Enter,
}

/// A clickable area of the panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Search,
    Gear,
    Wifi,
    WifiMore,
    Bluetooth,
    BtMore,
    Audio,
    AudioMore,
    AudioDevice(usize),
    /// Right-click on the audio toggle (no region of its own).
    AudioSettings,
    MediaPrev,
    MediaPlay,
    MediaNext,
    Tab(Tab),
    SortCpu,
    SortRam,
    CalPrev,
    CalNext,
    SpeedStart,
    Accent(usize),
    AccentWindows,
    Tint(usize),
    Result(usize),
}

#[derive(Default)]
pub struct PanelView {
    pub dpi: u32,
    pub colors: crate::config::Colors,
    pub quick: QuickState,
    pub battery: Vec<(DeviceKind, ItemView)>,
    pub media: Option<crate::media::NowPlaying>,
    pub tab: Tab,
    pub query: String,
    pub results: Vec<crate::apps::AppEntry>,
    pub selected: usize,
    pub stats: Option<crate::sysinfo::Stats>,
    pub sort_ram: bool,
    /// (year, month 1..=12).
    pub month: (i32, u32),
    /// (year, month, day) of the current local date.
    pub today: (i32, u32, u32),
    pub speed: crate::speedtest::SpeedState,
    pub gear_open: bool,
    pub audio_list: Option<Vec<String>>,
    pub hover: Option<Hit>,
}
