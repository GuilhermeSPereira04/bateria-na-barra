//! Where each part of the panel goes, in physical pixels (pure).
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use super::view::{Hit, PanelView, Tab};

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
/// Height of a row in the results / output list.
pub const ROW_H: f32 = 30.0;
pub const MAX_ROWS: usize = 8;
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
        regions.push((r(x, y, 132.0, COLOR_ROW_H), Hit::AccentWindows));
        x += 132.0 + 16.0;
        for i in 0..3 {
            regions.push((r(x, y, 80.0, COLOR_ROW_H), Hit::Tint(i)));
            x += 80.0 + GAP;
        }
        y += COLOR_ROW_H + GAP;
    }

    let quick_row = r(PAD, y, inner, QUICK_H);
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
    let list: Option<(usize, fn(usize) -> Hit)> = if !view.query.is_empty() {
        Some((view.results.len(), Hit::Result))
    } else {
        view.audio_list.as_ref().map(|l| (l.len(), Hit::AudioDevice as fn(usize) -> Hit))
    };
    let (tabs, body) = if let Some((count, hit)) = list {
        for i in 0..count.min(MAX_ROWS) {
            regions.push((r(PAD, y + i as f32 * ROW_H, inner, ROW_H), hit(i)));
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

    Layout { size: (WIDTH * s, y * s), regions, search, quick_row, battery, media, tabs, body }
}

impl Layout {
    /// The rectangle of `hit`, if it is on screen.
    pub fn rect(&self, hit: Hit) -> Option<D2D_RECT_F> {
        self.regions.iter().find(|(_, h)| *h == hit).map(|(r, _)| *r)
    }
}

pub fn hit_test(l: &Layout, x: f32, y: f32) -> Option<Hit> {
    l.regions.iter().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppEntry;

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
            PanelView { audio_list: Some(vec!["Fone".into(), "Monitor".into()]), ..at(144) },
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
        assert_eq!(layout(&at(144)).size.1, 444.0 * 1.5);
    }

    fn results(n: usize) -> Vec<AppEntry> {
        (0..n).map(|i| AppEntry { name: format!("App {i}"), id: format!("id{i}") }).collect()
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
        assert_eq!(open.size.1, closed.size.1 + 40.0);
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
