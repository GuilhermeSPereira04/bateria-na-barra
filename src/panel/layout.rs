//! Where each part of the panel goes, in physical pixels (pure).
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use super::view::{Hit, PanelView, SliderKind, Tab};

pub struct Layout {
    pub size: (f32, f32),
    pub regions: Vec<(D2D_RECT_F, Hit)>,
    pub search: D2D_RECT_F,
    #[allow(dead_code)] // the toggles are drawn from their regions
    pub quick_row: D2D_RECT_F,
    pub battery: D2D_RECT_F,
    pub media: D2D_RECT_F,
    /// Zero height while the lower part shows a list.
    #[allow(dead_code)] // the tabs are drawn from their regions
    pub tabs: D2D_RECT_F,
    pub body: D2D_RECT_F,
    /// The audio drop-down's card, drawn over the rest.
    pub menu: Option<D2D_RECT_F>,
}

// Base sizes in DIP (96 dpi).
const WIDTH: f32 = 760.0;
const PAD: f32 = 16.0;
const GAP: f32 = 8.0;
const SEARCH_H: f32 = 36.0;
/// The gear's color row; with the gap below it the panel grows by 40.
const COLOR_ROW_H: f32 = 32.0;
const QUICK_H: f32 = 52.0;
const TOGGLE_W: f32 = 120.0;
/// The "more" arrow at the right end of each quick toggle.
const ARROW_W: f32 = 28.0;
const MEDIA_H: f32 = 44.0;
const MEDIA_BTN: f32 = 36.0;
const TABS_H: f32 = 28.0;
const TAB_W: f32 = 96.0;
const BODY_H: f32 = 220.0;
/// Transparency row: the label's width and the bar's.
pub const SLIDER_LABEL_W: f32 = 128.0;
const SLIDER_W: f32 = 360.0;
/// Fits "Usar a cor do Windows" with room on both sides; the gear row then
/// takes 716 of the 728 DIP available.
const ACCENT_WINDOWS_W: f32 = 172.0;
/// Height of a row in the results / output list.
pub const ROW_H: f32 = 30.0;
pub const MAX_ROWS: usize = 8;
/// Most outputs the audio drop-down lists.
pub const MAX_OUTPUTS: usize = 8;
const MENU_W: f32 = 240.0;
/// The drop-down's first row: the volume bar.
const MENU_BAR_H: f32 = 36.0;
/// Where the bar starts and how wide it is inside the first row (icon before, value after).
pub const MENU_BAR_X: f32 = 40.0;
const MENU_BAR_W: f32 = 144.0;
/// Inner padding of the body card.
pub const INSET: f32 = 12.0;

/// Scale from DIP to physical pixels.
pub fn scale(dpi: u32) -> f32 {
    dpi.max(96) as f32 / 96.0
}

pub fn layout(view: &PanelView) -> Layout {
    let s = scale(view.dpi);
    let r = |x: f32, y: f32, w: f32, h: f32| D2D_RECT_F { left: x * s, top: y * s, right: (x + w) * s, bottom: (y + h) * s };
    let inner = WIDTH - 2.0 * PAD;
    let mut regions = Vec::new();

    let mut y = PAD;
    let search = r(PAD, y, inner - SEARCH_H - GAP, SEARCH_H);
    regions.push((search, Hit::Search));
    regions.push((r(WIDTH - PAD - SEARCH_H, y, SEARCH_H, SEARCH_H), Hit::Gear));
    y += SEARCH_H + GAP;

    if view.gear_open {
        let mut x = PAD;
        for i in 0..crate::config::ACCENTS.len() {
            regions.push((r(x, y, 32.0, COLOR_ROW_H), Hit::Accent(i)));
            x += 32.0;
        }
        x += 16.0;
        regions.push((r(x, y, ACCENT_WINDOWS_W, COLOR_ROW_H), Hit::AccentWindows));
        x += ACCENT_WINDOWS_W + 16.0;
        for i in 0..3 {
            regions.push((r(x, y, 80.0, COLOR_ROW_H), Hit::Tint(i)));
            x += 80.0 + GAP;
        }
        y += COLOR_ROW_H + GAP;
        // Transparency row: label, bar and value.
        regions.push((r(PAD + SLIDER_LABEL_W, y, SLIDER_W, COLOR_ROW_H), Hit::Slider(SliderKind::Transparency)));
        y += COLOR_ROW_H + GAP;
    }

    let quick_row = r(PAD, y, inner, QUICK_H);
    let audio_x = PAD + 2.0 * (TOGGLE_W + GAP);
    let menu_top = y + QUICK_H + GAP / 2.0;
    let mut x = PAD;
    for (main, more) in [(Hit::Wifi, Hit::WifiMore), (Hit::Bluetooth, Hit::BtMore), (Hit::Audio, Hit::AudioMore)] {
        regions.push((r(x, y, TOGGLE_W - ARROW_W, QUICK_H), main));
        regions.push((r(x + TOGGLE_W - ARROW_W, y, ARROW_W, QUICK_H), more));
        x += TOGGLE_W + GAP;
    }
    let battery = r(x, y, WIDTH - PAD - x, QUICK_H);
    y += QUICK_H + GAP;

    let media = r(PAD, y, inner, MEDIA_H);
    let btn_y = y + (MEDIA_H - MEDIA_BTN) / 2.0;
    let mut bx = WIDTH - PAD - 4.0 - 3.0 * MEDIA_BTN;
    for hit in [Hit::MediaPrev, Hit::MediaPlay, Hit::MediaNext] {
        regions.push((r(bx, btn_y, MEDIA_BTN, MEDIA_BTN), hit));
        bx += MEDIA_BTN;
    }
    y += MEDIA_H + GAP;

    // The lower part keeps one height in every mode, so typing doesn't resize the panel.
    let lower_h = TABS_H + GAP + BODY_H;
    let (tabs, body) = if !view.query.is_empty() {
        for i in 0..view.results.len().min(MAX_ROWS) {
            regions.push((r(PAD, y + i as f32 * ROW_H, inner, ROW_H), Hit::Result(i)));
        }
        (r(PAD, y, inner, 0.0), r(PAD, y, inner, lower_h))
    } else {
        for (i, tab) in [Tab::Resources, Tab::Calendar, Tab::Speed].into_iter().enumerate() {
            regions.push((r(PAD + i as f32 * (TAB_W + 4.0), y, TAB_W, TABS_H), Hit::Tab(tab)));
        }
        let by = y + TABS_H + GAP;
        let (left, right, top) = (PAD + INSET, WIDTH - PAD - INSET, by + INSET);
        match view.tab {
            Tab::Resources => {
                // Process table header on the right half: name | CPU | RAM.
                regions.push((r(right - 92.0 - 80.0, top, 80.0, 24.0), Hit::SortCpu));
                regions.push((r(right - 92.0, top, 92.0, 24.0), Hit::SortRam));
            }
            Tab::Calendar => {
                regions.push((r(left, top, 32.0, 28.0), Hit::CalPrev));
                regions.push((r(right - 32.0, top, 32.0, 28.0), Hit::CalNext));
            }
            Tab::Speed => regions.push((r(left, top, 120.0, 36.0), Hit::SpeedStart)),
        }
        (r(PAD, y, inner, TABS_H), r(PAD, by, inner, BODY_H))
    };
    y += lower_h + PAD;

    // The audio drop-down lies over the rest: its regions come first in hit tests.
    let menu = view.audio_menu.as_ref().map(|m| {
        let count = m.outputs.len().min(MAX_OUTPUTS);
        let mut menu_regions = vec![(
            r(audio_x + MENU_BAR_X, menu_top, MENU_BAR_W, MENU_BAR_H),
            Hit::Slider(SliderKind::Volume),
        )];
        for i in 0..count {
            menu_regions.push((r(audio_x, menu_top + MENU_BAR_H + i as f32 * ROW_H, MENU_W, ROW_H), Hit::AudioDevice(i)));
        }
        // The rest of the card takes clicks so they don't fall through to what is under it.
        let card = r(audio_x, menu_top, MENU_W, MENU_BAR_H + count as f32 * ROW_H);
        menu_regions.push((card, Hit::Blank));
        regions.splice(0..0, menu_regions);
        card
    });

    Layout { size: (WIDTH * s, y * s), regions, search, quick_row, battery, media, tabs, body, menu }
}

impl Layout {
    /// The rectangle of `hit`, if it is on screen.
    pub fn rect(&self, hit: Hit) -> Option<D2D_RECT_F> {
        self.regions.iter().find(|(_, h)| *h == hit).map(|(r, _)| *r)
    }
}

/// The value (0 at the left edge of `track`, 100 at the right, limited) under `x`.
pub fn slider_value(track: D2D_RECT_F, x: f32) -> u8 {
    let w = track.right - track.left;
    if w <= 0.0 {
        return 0;
    }
    (((x - track.left) / w).clamp(0.0, 1.0) * 100.0).round() as u8
}

pub fn hit_test(l: &Layout, x: f32, y: f32) -> Option<Hit> {
    l.regions.iter().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppEntry;
    use crate::panel::view::AudioMenu;

    fn at(dpi: u32) -> PanelView {
        PanelView { dpi, ..Default::default() }
    }

    fn has(l: &Layout, hit: Hit) -> bool {
        l.regions.iter().any(|(_, h)| *h == hit)
    }

    fn overlap(a: &D2D_RECT_F, b: &D2D_RECT_F) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    #[test]
    fn layout_has_all_fixed_regions() {
        let l = layout(&at(96));
        // Search box: y 16..52; quick row: y 60..112, Wi-Fi x 16..136 with its arrow at 108..136.
        assert_eq!(hit_test(&l, 300.0, 34.0), Some(Hit::Search));
        assert_eq!(hit_test(&l, 62.0, 86.0), Some(Hit::Wifi));
        assert_eq!(hit_test(&l, 122.0, 86.0), Some(Hit::WifiMore));
        assert_eq!(hit_test(&l, 726.0, 34.0), Some(Hit::Gear));
        for hit in [Hit::Bluetooth, Hit::BtMore, Hit::Audio, Hit::AudioMore, Hit::MediaPrev, Hit::MediaPlay, Hit::MediaNext] {
            assert!(has(&l, hit), "{hit:?}");
        }
        for tab in [Tab::Resources, Tab::Calendar, Tab::Speed] {
            assert!(has(&l, Hit::Tab(tab)), "{tab:?}");
        }
        assert_eq!(l.size, (760.0, 444.0));
        assert_eq!(hit_test(&l, 5.0, 5.0), None); // padding
    }

    #[test]
    fn layout_scales_with_dpi() {
        let views = [
            at(144),
            PanelView { gear_open: true, ..at(144) },
            PanelView { tab: Tab::Calendar, ..at(144) },
            PanelView { tab: Tab::Speed, ..at(144) },
            PanelView { query: "a".into(), results: results(8), ..at(144) },
        ];
        for view in &views {
            let l = layout(view);
            assert_eq!(l.size.0, 760.0 * 1.5);
            for (i, (a, ha)) in l.regions.iter().enumerate() {
                assert!(a.left >= 0.0 && a.top >= 0.0 && a.right <= l.size.0 && a.bottom <= l.size.1, "{ha:?} outside");
                for (b, hb) in &l.regions[i + 1..] {
                    assert!(!overlap(a, b), "{ha:?} overlaps {hb:?}");
                }
            }
        }
        // The audio drop-down lies over the rest (so overlaps are expected) but fits in the panel.
        for gear_open in [false, true] {
            let view = PanelView { gear_open, audio_menu: Some(menu(MAX_OUTPUTS)), ..at(144) };
            let l = layout(&view);
            let card = l.menu.expect("menu card");
            assert!(card.left >= 0.0 && card.top >= 0.0 && card.right <= l.size.0 && card.bottom <= l.size.1);
            for (a, ha) in &l.regions {
                assert!(a.right <= l.size.0 && a.bottom <= l.size.1, "{ha:?} outside");
            }
        }
        assert_eq!(layout(&at(144)).size.1, 444.0 * 1.5);
    }

    fn results(n: usize) -> Vec<AppEntry> {
        (0..n).map(|i| AppEntry { name: format!("App {i}"), id: format!("id{i}"), icon: None }).collect()
    }

    fn menu(n: usize) -> AudioMenu {
        AudioMenu { outputs: (0..n).map(|i| format!("Saida {i}")).collect(), current: Some(0), volume: 50 }
    }

    #[test]
    fn audio_menu_overlays_below_audio_card() {
        let l = layout(&PanelView { audio_menu: Some(menu(2)), ..at(96) });
        let card = l.menu.expect("menu card");
        let audio = l.rect(Hit::Audio).expect("audio card");
        assert!(card.top >= audio.bottom);
        // Aligned with the audio card's left edge, 240 wide.
        assert_eq!((card.left, card.right - card.left), (audio.left, 240.0));
        // A point on row 0 lies over the tabs area and still hits the output.
        let tabs = l.rect(Hit::Tab(Tab::Speed)).expect("tabs");
        let row0 = l.rect(Hit::AudioDevice(0)).expect("row 0");
        assert!(row0.top >= card.top + 36.0);
        let (x, y) = ((row0.left + row0.right) / 2.0, row0.top + 2.0);
        assert_eq!(hit_test(&l, x, y), Some(Hit::AudioDevice(0)));
        assert!(overlap(&row0, &tabs), "the list is over the tabs");
        assert!(has(&l, Hit::Tab(Tab::Resources)) && has(&l, Hit::Tab(Tab::Speed)));
        assert!(has(&l, Hit::Slider(SliderKind::Volume)));
        // The body no longer changes for audio.
        assert_eq!(l.size, layout(&at(96)).size);
    }

    #[test]
    fn audio_menu_caps_outputs() {
        let l = layout(&PanelView { audio_menu: Some(menu(12)), ..at(96) });
        assert!(has(&l, Hit::AudioDevice(0)) && has(&l, Hit::AudioDevice(7)));
        assert!(!has(&l, Hit::AudioDevice(8)));
        let l = layout(&PanelView { audio_menu: Some(menu(0)), ..at(96) });
        assert!(has(&l, Hit::Slider(SliderKind::Volume)));
        assert!(!l.regions.iter().any(|(_, h)| matches!(h, Hit::AudioDevice(_))));
    }

    #[test]
    fn query_switches_body_to_results() {
        let view = PanelView { query: "a".into(), results: results(3), ..at(96) };
        let l = layout(&view);
        for i in 0..3 {
            assert!(has(&l, Hit::Result(i)), "Result({i})");
        }
        assert!(!has(&l, Hit::Result(3)));
        assert!(!l.regions.iter().any(|(_, h)| matches!(h, Hit::Tab(_) | Hit::SortCpu | Hit::SortRam)));
        // At most 8 rows.
        let l = layout(&PanelView { query: "a".into(), results: results(12), ..at(96) });
        assert!(has(&l, Hit::Result(7)) && !has(&l, Hit::Result(8)));
    }

    #[test]
    fn gear_adds_color_row() {
        let closed = layout(&at(96));
        let open = layout(&PanelView { gear_open: true, ..at(96) });
        for i in 0..8 {
            assert!(has(&open, Hit::Accent(i)), "Accent({i})");
        }
        for i in 0..3 {
            assert!(has(&open, Hit::Tint(i)), "Tint({i})");
        }
        assert!(has(&open, Hit::AccentWindows));
        assert!(!has(&closed, Hit::Accent(0)) && !has(&closed, Hit::AccentWindows));
        assert!(has(&open, Hit::Slider(SliderKind::Transparency)));
        assert!(!has(&closed, Hit::Slider(SliderKind::Transparency)));
        assert_eq!(open.size.1, closed.size.1 + 80.0);
    }

    #[test]
    fn slider_value_maps_the_track() {
        let t = D2D_RECT_F { left: 100.0, top: 0.0, right: 300.0, bottom: 20.0 };
        assert_eq!(slider_value(t, 100.0), 0);
        assert_eq!(slider_value(t, 300.0), 100);
        assert_eq!(slider_value(t, 200.0), 50);
        assert_eq!(slider_value(t, -50.0), 0);
        assert_eq!(slider_value(t, 900.0), 100);
    }

    #[test]
    fn resources_tab_has_sort_headers() {
        let l = layout(&at(96));
        assert!(has(&l, Hit::SortCpu) && has(&l, Hit::SortRam));
        let cal = layout(&PanelView { tab: Tab::Calendar, ..at(96) });
        assert!(has(&cal, Hit::CalPrev) && has(&cal, Hit::CalNext) && !has(&cal, Hit::SortCpu));
        let speed = layout(&PanelView { tab: Tab::Speed, ..at(96) });
        assert!(has(&speed, Hit::SpeedStart));
    }
}
