//! Panel colors: the user's accent and glass tint choices, saved in
//! `%APPDATA%\battery-bar\config` as `key=value` lines.
use std::path::{Path, PathBuf};

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::w;

/// Preset accent colors offered under the gear, in order.
pub const ACCENTS: [u32; 8] = [0x9BE3B0, 0x60CDFF, 0xB4A0FF, 0xFF8FB1, 0xFFB86B, 0xF5E07A, 0x7FE0D4, 0xE6E6E6];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Accent {
    /// Index into `ACCENTS`.
    Preset(usize),
    /// Follows the Windows accent color.
    Windows,
    /// RGB typed by hand in the file.
    Custom(u32),
}

/// Layer painted over the glass. `Tint(0..3)` in the panel follows this order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlassTint {
    Dark,
    Light,
    Accent,
}

impl GlassTint {
    pub const ALL: [GlassTint; 3] = [GlassTint::Dark, GlassTint::Light, GlassTint::Accent];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    pub accent: Accent,
    pub tint: GlassTint,
    /// 0..=100: how much of the desktop shows through the glass.
    pub transparency: u8,
}

/// Transparency used when the file has none (or garbage).
pub const DEFAULT_TRANSPARENCY: u8 = 45;

impl Default for Config {
    fn default() -> Self {
        Self { accent: Accent::Preset(0), tint: GlassTint::Dark, transparency: DEFAULT_TRANSPARENCY }
    }
}

/// The colors the panel is drawn with, resolved from the user's choice.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Colors {
    /// RGB of buttons that are on, bars, the active tab and today.
    pub accent: u32,
    /// RGB and opacity of the acrylic tint (opacity = 1 - transparency).
    pub tint: (u32, f32),
    /// What the accent came from, so the gear row can mark the selection.
    pub accent_choice: Accent,
    /// Which tint chip is selected.
    pub tint_choice: GlassTint,
    /// 0..=100, as saved.
    pub transparency: u8,
}

impl Default for Colors {
    fn default() -> Self {
        resolve_with(&Config::default(), ACCENTS[0])
    }
}

/// `0xAABBGGRR` (the DWM registry value) to `0xRRGGBB`.
pub fn abgr_to_rgb(abgr: u32) -> u32 {
    let (r, g, b) = (abgr & 0xFF, (abgr >> 8) & 0xFF, (abgr >> 16) & 0xFF);
    (r << 16) | (g << 8) | b
}

/// The Windows accent color, or the first preset when it can't be read.
pub fn windows_accent() -> u32 {
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\DWM"),
            w!("AccentColor"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    }
    .is_ok();
    if ok { abgr_to_rgb(value) } else { ACCENTS[0] }
}

pub fn resolve(c: &Config) -> Colors {
    let windows = if c.accent == Accent::Windows { windows_accent() } else { ACCENTS[0] };
    resolve_with(c, windows)
}

fn resolve_with(c: &Config, windows: u32) -> Colors {
    let accent = match c.accent {
        Accent::Preset(i) => ACCENTS.get(i).copied().unwrap_or(ACCENTS[0]),
        Accent::Windows => windows,
        Accent::Custom(rgb) => rgb,
    };
    let rgb = match c.tint {
        GlassTint::Dark => 0x20242C,
        GlassTint::Light => 0xF3F3F3,
        GlassTint::Accent => accent,
    };
    let alpha = (100 - c.transparency.min(100)) as f32 / 100.0;
    Colors { accent, tint: (rgb, alpha), accent_choice: c.accent, tint_choice: c.tint, transparency: c.transparency.min(100) }
}

fn parse_accent(v: &str) -> Option<Accent> {
    if v.eq_ignore_ascii_case("windows") {
        return Some(Accent::Windows);
    }
    if let Some(i) = v.strip_prefix("preset:") {
        return i.trim().parse::<usize>().ok().filter(|&i| i < ACCENTS.len()).map(Accent::Preset);
    }
    let hex = v.strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok()).flatten().map(Accent::Custom)
}

/// A number, limited to 0..=100.
fn parse_transparency(v: &str) -> Option<u8> {
    v.parse::<i64>().ok().map(|n| n.clamp(0, 100) as u8)
}

fn parse_tint(v: &str) -> Option<GlassTint> {
    match v.to_ascii_lowercase().as_str() {
        "dark" => Some(GlassTint::Dark),
        "light" => Some(GlassTint::Light),
        "accent" => Some(GlassTint::Accent),
        _ => None,
    }
}

/// Lenient: unknown keys and bad values keep the defaults.
pub fn parse(text: &str) -> Config {
    let mut c = Config::default();
    for raw in text.lines() {
        let line = strip_comment(raw);
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "accent" => c.accent = parse_accent(v).unwrap_or(c.accent),
            "tint" => c.tint = parse_tint(v).unwrap_or(c.tint),
            "transparency" => c.transparency = parse_transparency(v).unwrap_or(c.transparency),
            _ => {}
        }
    }
    c
}

/// Drops a trailing comment: `#` starts one only at the line start or after whitespace,
/// so `accent=#ff8800` keeps its value.
fn strip_comment(line: &str) -> &str {
    let mut prev_space = true;
    for (i, ch) in line.char_indices() {
        if ch == '#' && prev_space && !line[..i].trim_end().ends_with('=') {
            return &line[..i];
        }
        prev_space = ch.is_whitespace();
    }
    line
}

pub fn serialize(c: &Config) -> String {
    let accent = match c.accent {
        Accent::Preset(i) => format!("preset:{i}"),
        Accent::Windows => "windows".to_string(),
        Accent::Custom(rgb) => format!("#{rgb:06X}"),
    };
    let tint = match c.tint {
        GlassTint::Dark => "dark",
        GlassTint::Light => "light",
        GlassTint::Accent => "accent",
    };
    format!("accent={accent}\ntint={tint}\ntransparency={}\n", c.transparency.min(100))
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("battery-bar").join("config"))
}

fn load_from(path: &Path) -> Config {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

fn save_to(path: &Path, c: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serialize(c))
}

pub fn load() -> Config {
    config_path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(c: &Config) {
    if let Some(p) = config_path() {
        let _ = save_to(&p, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for c in [
            Config { accent: Accent::Preset(3), tint: GlassTint::Light, transparency: 0 },
            Config { accent: Accent::Windows, tint: GlassTint::Dark, transparency: 45 },
            Config { accent: Accent::Custom(0xFF8800), tint: GlassTint::Accent, transparency: 100 },
        ] {
            assert_eq!(parse(&serialize(&c)), c);
        }
    }

    #[test]
    fn garbage_falls_back_to_default() {
        assert_eq!(parse("accent=banana\ntint="), Config::default());
        assert_eq!(parse("accent=preset:99\ntint=neon"), Config::default());
    }

    #[test]
    fn comments_and_spaces() {
        assert_eq!(parse("  accent = #ff8800  # laranja\n").accent, Accent::Custom(0xFF8800));
        assert_eq!(parse("# accent=windows\ntint=light # claro").tint, GlassTint::Light);
        assert_eq!(parse("# accent=windows\n").accent, Accent::Preset(0));
    }

    #[test]
    fn transparency_roundtrip() {
        for t in [0, 45, 100] {
            let c = Config { transparency: t, ..Config::default() };
            assert_eq!(parse(&serialize(&c)).transparency, t);
        }
    }

    #[test]
    fn transparency_clamps_and_defaults() {
        assert_eq!(parse("transparency=150").transparency, 100);
        assert_eq!(parse("transparency=-3").transparency, 0);
        assert_eq!(parse("transparency=abc").transparency, 45);
        assert_eq!(parse("accent=windows").transparency, 45);
    }

    #[test]
    fn alpha_from_transparency() {
        let c = resolve_with(&Config::default(), 0);
        assert_eq!(c.tint.1, 0.55);
        assert_eq!(c.transparency, 45);
        let full = resolve_with(&Config { transparency: 100, ..Config::default() }, 0);
        assert_eq!(full.tint.1, 0.0);
    }

    #[test]
    fn abgr_to_rgb_swaps_channels() {
        assert_eq!(abgr_to_rgb(0xFF332211), 0x112233);
    }

    #[test]
    fn resolve_tints() {
        let c = |tint| resolve_with(&Config { accent: Accent::Preset(1), tint, ..Config::default() }, 0);
        assert_eq!(c(GlassTint::Dark).tint, (0x20242C, 0.55));
        assert_eq!(c(GlassTint::Light).tint, (0xF3F3F3, 0.55));
        assert_eq!(c(GlassTint::Accent).tint, (0x60CDFF, 0.55));
        let w = resolve_with(&Config { accent: Accent::Windows, tint: GlassTint::Accent, ..Config::default() }, 0x123456);
        assert_eq!((w.accent, w.tint), (0x123456, (0x123456, 0.55)));
        assert_eq!(w.accent_choice, Accent::Windows);
    }

    #[test]
    fn file_roundtrip_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("battery-bar-test-{}", std::process::id()));
        let path = dir.join("sub").join("config");
        assert_eq!(load_from(&path), Config::default());
        let c = Config { accent: Accent::Custom(0x0A0B0C), tint: GlassTint::Light, transparency: 70 };
        save_to(&path, &c).unwrap();
        assert_eq!(load_from(&path), c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
