//! Paints a `PanelView` with Direct2D over the transparent (glass) target.
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::PCWSTR;
use windows_numerics::Vector2;

use super::layout::{INSET, Layout, MAX_ROWS, ROW_H, scale};
use super::view::{Hit, PanelView, Tab, Toggle};
use crate::config::{ACCENTS, Colors};
use crate::model::ItemView;
use crate::render::{BOLT, ICON_FONT, TEXT_FONT, device_glyph, item_style, rgb};
use crate::speedtest::{Phase, SpeedState};

const CARD: f32 = 0.07; // white over the dark glass
const HOVER: f32 = 0.06;
const SELECTED: f32 = 0.10;
const RADIUS: f32 = 8.0;
const TEXT: u32 = 0xFFFFFF;
const ERROR: u32 = 0xFF6B5E;
/// Text over an accent fill (accents are light colors).
const ON_ACCENT: u32 = 0x0E1A12;
const UNAVAILABLE: f32 = 0.45;

const GLYPH_SEARCH: char = '\u{E721}';
const GLYPH_GEAR: char = '\u{E713}';
const GLYPH_MORE: char = '\u{E70D}';
const GLYPH_VOLUME: char = '\u{E767}';
const GLYPH_MUTE: char = '\u{E74F}';
const GLYPH_PREV: char = '\u{E892}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_LEFT: char = '\u{E76B}';
const GLYPH_RIGHT: char = '\u{E76C}';

const MONTHS: [&str; 12] =
    ["Janeiro", "Fevereiro", "Março", "Abril", "Maio", "Junho", "Julho", "Agosto", "Setembro", "Outubro", "Novembro", "Dezembro"];

/// Text formats (per DPI) and the brush (per render target), kept between paints.
pub struct Cache {
    dpi: u32,
    fonts: Option<Fonts>,
    brush: Option<ID2D1SolidColorBrush>,
}

struct Fonts {
    text: IDWriteTextFormat,
    bold: IDWriteTextFormat,
    small: IDWriteTextFormat,
    battery: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_big: IDWriteTextFormat,
}

impl Cache {
    pub fn new() -> Self {
        Self { dpi: 0, fonts: None, brush: None }
    }

    /// Forgets what belongs to a render target that is being replaced.
    pub fn reset_target(&mut self) {
        self.brush = None;
    }
}

fn format(dw: &IDWriteFactory, family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32) -> windows::core::Result<IDWriteTextFormat> {
    unsafe {
        let f = dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, windows::core::w!(""))?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let sign = dw.CreateEllipsisTrimmingSign(&f)?;
        let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
        f.SetTrimming(&trimming, &sign)?;
        Ok(f)
    }
}

impl Fonts {
    fn new(dw: &IDWriteFactory, s: f32) -> windows::core::Result<Self> {
        Ok(Self {
            text: format(dw, TEXT_FONT, DWRITE_FONT_WEIGHT_NORMAL, 13.0 * s)?,
            bold: format(dw, TEXT_FONT, DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0 * s)?,
            small: format(dw, TEXT_FONT, DWRITE_FONT_WEIGHT_NORMAL, 11.0 * s)?,
            // Same size and weight as the strip.
            battery: format(dw, TEXT_FONT, DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.0 * s)?,
            icon: format(dw, ICON_FONT, DWRITE_FONT_WEIGHT_NORMAL, 14.0 * s)?,
            icon_big: format(dw, ICON_FONT, DWRITE_FONT_WEIGHT_NORMAL, 16.0 * s)?,
        })
    }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

/// Drawing helpers bound to one paint.
struct Painter<'a> {
    rt: &'a ID2D1RenderTarget,
    dw: &'a IDWriteFactory,
    brush: &'a ID2D1SolidColorBrush,
    f: &'a Fonts,
    s: f32,
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

fn white(alpha: f32) -> D2D1_COLOR_F {
    rgb(TEXT, alpha)
}

impl Painter<'_> {
    fn px(&self, dip: f32) -> f32 {
        dip * self.s
    }

    fn color(&self, c: D2D1_COLOR_F) {
        unsafe { self.brush.SetColor(&c) };
    }

    fn fill(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        self.color(c);
        let radius = self.px(RADIUS);
        unsafe { self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, self.brush) };
    }

    fn circle(&self, cx: f32, cy: f32, radius: f32, c: D2D1_COLOR_F, stroke: Option<f32>) {
        self.color(c);
        let e = D2D1_ELLIPSE { point: Vector2 { X: cx, Y: cy }, radiusX: radius, radiusY: radius };
        unsafe {
            match stroke {
                Some(w) => self.rt.DrawEllipse(&e, self.brush, w, None),
                None => self.rt.FillEllipse(&e, self.brush),
            }
        }
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.rt.DrawLine(Vector2 { X: a.0, Y: a.1 }, Vector2 { X: b.0, Y: b.1 }, self.brush, width, None) };
    }

    fn text(&self, s: &str, fmt: &IDWriteTextFormat, r: D2D_RECT_F, align: Align, c: D2D1_COLOR_F) {
        let a = match align {
            Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
        };
        self.color(c);
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let _ = fmt.SetTextAlignment(a);
            self.rt.DrawText(&wide, fmt, &r, self.brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
    }

    fn glyph(&self, g: char, fmt: &IDWriteTextFormat, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        self.text(&g.to_string(), fmt, r, Align::Center, c);
    }

    fn measure(&self, s: &str, fmt: &IDWriteTextFormat) -> f32 {
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let mut m = DWRITE_TEXT_METRICS::default();
            match self.dw.CreateTextLayout(&wide, fmt, 10_000.0, 100.0).and_then(|l| l.GetMetrics(&mut m)) {
                Ok(()) => m.widthIncludingTrailingWhitespace,
                Err(_) => 0.0,
            }
        }
    }
}

pub fn draw(rt: &ID2D1RenderTarget, dw: &IDWriteFactory, cache: &mut Cache, view: &PanelView, layout: &Layout) -> windows::core::Result<()> {
    let s = scale(view.dpi);
    if cache.fonts.is_none() || cache.dpi != view.dpi {
        cache.fonts = Some(Fonts::new(dw, s)?);
        cache.dpi = view.dpi;
    }
    if cache.brush.is_none() {
        cache.brush = Some(unsafe { rt.CreateSolidColorBrush(&white(1.0), None)? });
    }
    let (Some(f), Some(brush)) = (&cache.fonts, &cache.brush) else { return Ok(()) };
    let p = Painter { rt, dw, brush, f, s };
    // ClearType can't blend onto transparent pixels.
    unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };

    let (tint, alpha) = view.colors.tint;
    p.color(rgb(tint, alpha));
    unsafe { rt.FillRectangle(&rect(0.0, 0.0, layout.size.0, layout.size.1), brush) };

    search(&p, view, layout);
    if view.gear_open {
        colors_row(&p, &view.colors, layout);
    }
    quick(&p, view, layout);
    battery(&p, view, layout);
    media(&p, view, layout);
    lower(&p, view, layout);

    if let Some(r) = view.hover.and_then(|h| layout.rect(h)) {
        p.fill(r, white(HOVER));
    }
    Ok(())
}

fn search(p: &Painter, view: &PanelView, l: &Layout) {
    let r = l.search;
    p.fill(r, white(CARD));
    p.glyph(GLYPH_SEARCH, &p.f.icon, rect(r.left + p.px(8.0), r.top, r.left + p.px(32.0), r.bottom), white(0.6));
    let text = rect(r.left + p.px(38.0), r.top, r.right - p.px(12.0), r.bottom);
    if view.query.is_empty() {
        p.text("Pesquisar apps…", &p.f.text, text, Align::Left, white(0.6));
    } else {
        p.text(&view.query, &p.f.text, text, Align::Left, white(0.92));
    }
    if let Some(g) = l.rect(Hit::Gear) {
        p.fill(g, white(if view.gear_open { 0.14 } else { CARD }));
        p.glyph(GLYPH_GEAR, &p.f.icon_big, g, white(0.85));
    }
}

/// Index of the glass tint the colors use (dark, light, accent), as Task 4 resolves them.
fn tint_index(c: &Colors) -> Option<usize> {
    match c.tint {
        (0x000000, a) if a == 0.25 => Some(0),
        (0xFFFFFF, a) if a == 0.18 => Some(1),
        (rgb, a) if rgb == c.accent && a == 0.15 => Some(2),
        _ => None,
    }
}

fn colors_row(p: &Painter, colors: &Colors, l: &Layout) {
    for (i, &hex) in ACCENTS.iter().enumerate() {
        let Some(r) = l.rect(Hit::Accent(i)) else { continue };
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        p.circle(cx, cy, p.px(10.0), rgb(hex, 1.0), None);
        if hex == colors.accent {
            p.circle(cx, cy, p.px(13.0), white(0.9), Some(p.px(1.5)));
        }
    }
    if let Some(r) = l.rect(Hit::AccentWindows) {
        p.fill(r, white(CARD));
        p.text("Cor do Windows", &p.f.text, r, Align::Center, white(0.92));
    }
    let selected = tint_index(colors);
    for (i, name) in ["Escuro", "Claro", "Destaque"].into_iter().enumerate() {
        let Some(r) = l.rect(Hit::Tint(i)) else { continue };
        if selected == Some(i) {
            p.fill(r, rgb(colors.accent, 1.0));
            p.text(name, &p.f.bold, r, Align::Center, rgb(ON_ACCENT, 0.9));
        } else {
            p.fill(r, white(CARD));
            p.text(name, &p.f.text, r, Align::Center, white(0.92));
        }
    }
}

fn quick(p: &Painter, view: &PanelView, l: &Layout) {
    let q = &view.quick;
    let plural = |n: u32| if n == 1 { "1 conectado".to_string() } else { format!("{n} conectados") };
    let wifi_sub = match q.wifi {
        Toggle::On => q.wifi_name.clone().unwrap_or_else(|| "Ligado".into()),
        _ => "Desligado".into(),
    };
    let bt_sub = match q.bt {
        Toggle::On if q.bt_connected > 0 => plural(q.bt_connected),
        Toggle::On => "Ligado".into(),
        _ => "Desligado".into(),
    };
    let audio_title = if q.audio == Toggle::Unavailable { "Áudio".to_string() } else { format!("{}%", q.volume) };
    let audio_sub = match q.audio {
        Toggle::Off => "Mudo".into(),
        _ => q.output_name.clone().unwrap_or_default(),
    };
    let audio_glyph = if q.audio == Toggle::Off { GLYPH_MUTE } else { GLYPH_VOLUME };
    let toggles = [
        (Hit::Wifi, Hit::WifiMore, q.wifi, None, "Wi‑Fi".to_string(), wifi_sub),
        (Hit::Bluetooth, Hit::BtMore, q.bt, None, "Bluetooth".to_string(), bt_sub),
        (Hit::Audio, Hit::AudioMore, q.audio, Some(audio_glyph), audio_title, audio_sub),
    ];
    for (main, more, state, glyph, title, sub) in toggles {
        let (Some(m), Some(a)) = (l.rect(main), l.rect(more)) else { continue };
        let card = rect(m.left, m.top, a.right, m.bottom);
        let (fg, sub) = match state {
            Toggle::On => {
                p.fill(card, rgb(view.colors.accent, 1.0));
                (rgb(ON_ACCENT, 0.9), sub)
            }
            Toggle::Off => {
                p.fill(card, white(CARD));
                (white(0.92), sub)
            }
            Toggle::Unavailable => {
                p.fill(card, white(CARD));
                (white(UNAVAILABLE), "Indisponível".to_string())
            }
        };
        let mid = (m.top + m.bottom) / 2.0;
        let mut x = m.left + p.px(12.0);
        if let Some(g) = glyph {
            p.glyph(g, &p.f.icon, rect(x, m.top + p.px(8.0), x + p.px(16.0), mid + p.px(2.0)), fg);
            x += p.px(20.0);
        }
        p.text(&title, &p.f.bold, rect(x, m.top + p.px(8.0), m.right - p.px(4.0), mid + p.px(2.0)), Align::Left, fg);
        let sub_color = D2D1_COLOR_F { a: fg.a * 0.8, ..fg };
        p.text(&sub, &p.f.small, rect(m.left + p.px(12.0), mid, m.right - p.px(4.0), m.bottom - p.px(8.0)), Align::Left, sub_color);
        let divider = D2D1_COLOR_F { a: 0.25, ..fg };
        p.line((a.left, a.top + p.px(14.0)), (a.left, a.bottom - p.px(14.0)), p.px(1.0), divider);
        p.glyph(GLYPH_MORE, &p.f.icon, a, fg);
    }
}

fn battery(p: &Painter, view: &PanelView, l: &Layout) {
    let r = l.battery;
    p.fill(r, white(CARD));
    let visible: Vec<_> = view.battery.iter().filter(|(_, v)| *v != ItemView::Hidden).collect();
    if visible.is_empty() {
        p.text("Nenhum aparelho", &p.f.text, rect(r.left + p.px(14.0), r.top, r.right, r.bottom), Align::Left, white(0.55));
        return;
    }
    let mut x = r.left + p.px(14.0);
    for (i, (kind, item)) in visible.into_iter().enumerate() {
        if i > 0 {
            x += p.px(14.0);
        }
        let (text, color, charging) = item_style(item, true);
        let icon = device_glyph(*kind).to_string();
        let w = p.measure(&icon, &p.f.icon);
        p.text(&icon, &p.f.icon, rect(x, r.top, x + w, r.bottom), Align::Left, color);
        x += w + p.px(4.0);
        let w = p.measure(&text, &p.f.battery);
        p.text(&text, &p.f.battery, rect(x, r.top, x + w, r.bottom), Align::Left, color);
        x += w;
        if charging {
            x += p.px(2.0);
            let bolt = BOLT.to_string();
            let w = p.measure(&bolt, &p.f.icon);
            p.text(&bolt, &p.f.icon, rect(x, r.top, x + w, r.bottom), Align::Left, color);
            x += w;
        }
    }
}

fn media(p: &Painter, view: &PanelView, l: &Layout) {
    let r = l.media;
    p.fill(r, white(CARD));
    let end = l.rect(Hit::MediaPrev).map_or(r.right, |b| b.left) - p.px(8.0);
    let left = r.left + p.px(12.0);
    let (playing, alpha) = match &view.media {
        Some(np) => {
            let w = p.measure(&np.title, &p.f.bold).min(end - left);
            p.text(&np.title, &p.f.bold, rect(left, r.top, left + w, r.bottom), Align::Left, white(0.92));
            if !np.artist.is_empty() {
                let artist = format!(" · {}", np.artist);
                p.text(&artist, &p.f.text, rect(left + w, r.top, end, r.bottom), Align::Left, white(0.6));
            }
            (np.playing, 0.85)
        }
        None => {
            p.text("Nada tocando", &p.f.text, rect(left, r.top, end, r.bottom), Align::Left, white(0.55));
            (false, UNAVAILABLE)
        }
    };
    let play = if playing { GLYPH_PAUSE } else { GLYPH_PLAY };
    for (hit, g) in [(Hit::MediaPrev, GLYPH_PREV), (Hit::MediaPlay, play), (Hit::MediaNext, GLYPH_NEXT)] {
        if let Some(b) = l.rect(hit) {
            p.glyph(g, &p.f.icon, b, white(alpha));
        }
    }
}

fn lower(p: &Painter, view: &PanelView, l: &Layout) {
    p.fill(l.body, white(CARD));
    if !view.query.is_empty() {
        let names: Vec<&str> = view.results.iter().map(|a| a.name.as_str()).collect();
        rows(p, l, &names, Hit::Result, Some(view.selected), "Nada encontrado");
        return;
    }
    if let Some(outputs) = &view.audio_list {
        let names: Vec<&str> = outputs.iter().map(String::as_str).collect();
        rows(p, l, &names, Hit::AudioDevice, None, "Nenhuma saída de áudio");
        return;
    }
    for (tab, name) in [(Tab::Resources, "Recursos"), (Tab::Calendar, "Calendário"), (Tab::Speed, "Speed test")] {
        let Some(r) = l.rect(Hit::Tab(tab)) else { continue };
        if tab == view.tab {
            let accent = rgb(view.colors.accent, 1.0);
            p.text(name, &p.f.bold, r, Align::Center, accent);
            let cx = (r.left + r.right) / 2.0;
            let y = r.bottom - p.px(1.0);
            p.line((cx - p.px(10.0), y), (cx + p.px(10.0), y), p.px(2.0), accent);
        } else {
            p.text(name, &p.f.text, r, Align::Center, white(0.6));
        }
    }
    match view.tab {
        Tab::Resources => resources(p, view, l),
        Tab::Calendar => calendar(p, view, l),
        Tab::Speed => speed(p, view, l),
    }
}

/// One line per item (results or audio outputs), at most `MAX_ROWS`.
fn rows(p: &Painter, l: &Layout, names: &[&str], hit: fn(usize) -> Hit, selected: Option<usize>, empty: &str) {
    if names.is_empty() {
        let b = l.body;
        let r = rect(b.left + p.px(INSET), b.top, b.right, b.top + p.px(ROW_H));
        p.text(empty, &p.f.text, r, Align::Left, white(0.55));
        return;
    }
    for (i, name) in names.iter().take(MAX_ROWS).enumerate() {
        let Some(r) = l.rect(hit(i)) else { continue };
        if selected == Some(i) {
            p.fill(r, white(SELECTED));
        }
        let text = rect(r.left + p.px(INSET), r.top, r.right - p.px(INSET), r.bottom);
        p.text(name, &p.f.text, text, Align::Left, white(0.92));
    }
}

/// Decimal comma, as in pt-BR.
fn decimal(v: f32) -> String {
    format!("{v:.1}").replace('.', ",")
}

fn bytes(b: u64) -> String {
    const GB: f32 = 1024.0 * 1024.0 * 1024.0;
    let b = b as f32;
    if b >= GB { format!("{} GB", decimal(b / GB)) } else { format!("{:.0} MB", b / (1024.0 * 1024.0)) }
}

fn resources(p: &Painter, view: &PanelView, l: &Layout) {
    let b = l.body;
    let accent = rgb(view.colors.accent, 1.0);
    let left = b.left + p.px(INSET);
    let right = b.left + p.px(INSET + 300.0);
    let st = view.stats.as_ref();
    let pct = |v: f32| format!("{v:.0}%");
    let ram_frac = st.map(|s| if s.ram_total > 0 { s.ram_used as f32 / s.ram_total as f32 } else { 0.0 });
    let meters = [
        ("CPU", st.map(|s| pct(s.cpu)), st.map(|s| s.cpu / 100.0)),
        ("GPU", st.map(|s| s.gpu.map_or("—".into(), pct)), st.and_then(|s| s.gpu.map(|g| g / 100.0))),
        (
            "RAM",
            st.zip(ram_frac).map(|(s, f)| format!("{} / {} ({:.0}%)", bytes(s.ram_used), bytes(s.ram_total), f * 100.0)),
            ram_frac,
        ),
    ];
    for (i, (name, value, frac)) in meters.into_iter().enumerate() {
        let top = b.top + p.px(INSET + i as f32 * 64.0);
        let line = rect(left, top, right, top + p.px(22.0));
        p.text(name, &p.f.bold, line, Align::Left, white(0.92));
        p.text(&value.unwrap_or_else(|| "…".into()), &p.f.text, line, Align::Right, white(0.8));
        let bar_top = top + p.px(28.0);
        let track = rect(left, bar_top, right, bar_top + p.px(6.0));
        p.color(white(0.10));
        let round = |r: D2D_RECT_F| D2D1_ROUNDED_RECT { rect: r, radiusX: p.px(3.0), radiusY: p.px(3.0) };
        unsafe { p.rt.FillRoundedRectangle(&round(track), p.brush) };
        if let Some(fr) = frac {
            let w = (right - left) * fr.clamp(0.0, 1.0);
            p.color(accent);
            unsafe { p.rt.FillRoundedRectangle(&round(rect(left, bar_top, left + w, track.bottom)), p.brush) };
        }
    }

    let (Some(cpu), Some(ram)) = (l.rect(Hit::SortCpu), l.rect(Hit::SortRam)) else { return };
    let name_left = b.left + p.px(INSET + 332.0);
    let pad = p.px(6.0);
    let head = |text: &str, r: D2D_RECT_F, sorted: bool| {
        let (fmt, c) = if sorted { (&p.f.bold, accent) } else { (&p.f.text, white(0.6)) };
        p.text(text, fmt, rect(r.left, r.top, r.right - pad, r.bottom), Align::Right, c);
    };
    p.text("Processo", &p.f.text, rect(name_left, cpu.top, cpu.left, cpu.bottom), Align::Left, white(0.6));
    head("CPU", cpu, !view.sort_ram);
    head("RAM", ram, view.sort_ram);
    let row_h = p.px(28.0);
    let Some(st) = st else {
        let r = rect(name_left, cpu.bottom + p.px(4.0), ram.right, cpu.bottom + p.px(4.0) + row_h);
        p.text("…", &p.f.text, r, Align::Left, white(0.6));
        return;
    };
    for (i, row) in st.top.iter().take(5).enumerate() {
        let top = cpu.bottom + p.px(4.0) + i as f32 * row_h;
        let bottom = top + row_h;
        p.text(&row.name, &p.f.text, rect(name_left, top, cpu.left - pad, bottom), Align::Left, white(0.92));
        p.text(&format!("{}%", decimal(row.cpu)), &p.f.text, rect(cpu.left, top, cpu.right - pad, bottom), Align::Right, white(0.8));
        p.text(&bytes(row.ram), &p.f.text, rect(ram.left, top, ram.right - pad, bottom), Align::Right, white(0.8));
    }
}

fn calendar(p: &Painter, view: &PanelView, l: &Layout) {
    let (Some(prev), Some(next)) = (l.rect(Hit::CalPrev), l.rect(Hit::CalNext)) else { return };
    p.glyph(GLYPH_LEFT, &p.f.icon, prev, white(0.85));
    p.glyph(GLYPH_RIGHT, &p.f.icon, next, white(0.85));
    let (year, month) = view.month;
    if (1..=12).contains(&month) {
        let title = format!("{} {year}", MONTHS[month as usize - 1]);
        p.text(&title, &p.f.bold, rect(prev.right, prev.top, next.left, prev.bottom), Align::Center, white(0.92));
    }
    // Weekday header, Sunday first; the day grid comes with Task 10.
    let top = prev.bottom + p.px(8.0);
    let col = (next.right - prev.left) / 7.0;
    for (i, d) in ["D", "S", "T", "Q", "Q", "S", "S"].into_iter().enumerate() {
        let x = prev.left + i as f32 * col;
        p.text(d, &p.f.small, rect(x, top, x + col, top + p.px(20.0)), Align::Center, white(0.6));
    }
}

fn speed(p: &Painter, view: &PanelView, l: &Layout) {
    let Some(btn) = l.rect(Hit::SpeedStart) else { return };
    let running = matches!(view.speed, SpeedState::Running { .. });
    if running {
        p.fill(btn, white(CARD));
        p.text("Testando…", &p.f.bold, btn, Align::Center, white(UNAVAILABLE));
    } else {
        p.fill(btn, rgb(view.colors.accent, 1.0));
        p.text("Testar", &p.f.bold, btn, Align::Center, rgb(ON_ACCENT, 0.9));
    }
    let mbps = |v: f32| format!("{} Mb/s", decimal(v));
    let opt = |v: Option<String>| v.unwrap_or_else(|| "…".into());
    let (line, sub, color) = match &view.speed {
        SpeedState::Idle => ("Mede ping, download e upload".to_string(), None, white(0.6)),
        SpeedState::Running { phase, ping_ms, down, up } => {
            let phase = match phase {
                Phase::Ping => "Medindo ping…",
                Phase::Download => "Medindo download…",
                Phase::Upload => "Medindo upload…",
            };
            let nums = format!(
                "Ping {} · ↓ {} · ↑ {}",
                opt(ping_ms.map(|v| format!("{v} ms"))),
                opt(down.map(mbps)),
                opt(up.map(mbps))
            );
            (nums, Some(phase.to_string()), white(0.92))
        }
        SpeedState::Done { ping_ms, down, up, at } => (
            format!("Ping {ping_ms} ms · ↓ {} · ↑ {}", mbps(*down), mbps(*up)),
            Some(format!("Às {:02}:{:02}", at.0, at.1)),
            white(0.92),
        ),
        SpeedState::Error(msg) => (msg.clone(), None, rgb(ERROR, 1.0)),
    };
    let x = btn.right + p.px(16.0);
    let right = l.body.right - p.px(INSET);
    p.text(&line, &p.f.text, rect(x, btn.top, right, btn.bottom), Align::Left, color);
    if let Some(sub) = sub {
        p.text(&sub, &p.f.small, rect(x, btn.bottom, right, btn.bottom + p.px(20.0)), Align::Left, white(0.6));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::AppEntry;
    use crate::media::NowPlaying;
    use crate::model::{Charge, DeviceKind, Reading};
    use crate::panel::layout::layout;
    use crate::render::Bitmap;
    use crate::sysinfo::{ProcRow, Stats};
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

    /// Draws `view` into a software target; returns the BGRA pixels.
    fn render(view: &PanelView, cache: &mut Cache) -> (Bitmap, windows::core::Result<()>) {
        let l = layout(view);
        let bmp = Bitmap::new(l.size.0 as i32, l.size.1 as i32).unwrap();
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap();
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let rt = d2d.CreateDCRenderTarget(&props).unwrap();
            let rc = windows::Win32::Foundation::RECT { left: 0, top: 0, right: bmp.width, bottom: bmp.height };
            rt.BindDC(bmp.dc, &rc).unwrap();
            rt.BeginDraw();
            rt.Clear(Some(&rgb(0, 0.0)));
            let drawn = draw(&rt, &dw, cache, view, &l);
            rt.EndDraw(None, None).unwrap();
            cache.reset_target(); // the next call uses another target
            (bmp, drawn)
        }
    }

    fn full_view(dpi: u32) -> PanelView {
        let charging = Reading { percent: 15, charge: Charge::Charging };
        PanelView {
            dpi,
            battery: vec![
                (DeviceKind::Mouse, ItemView::Value(charging)),
                (DeviceKind::Controller, ItemView::Dash),
                (DeviceKind::Phone, ItemView::Hidden),
            ],
            media: Some(NowPlaying { title: "Kick Back".into(), artist: "Kenshi Yonezu".into(), playing: true, cover: None }),
            stats: Some(Stats {
                cpu: 23.0,
                gpu: None,
                ram_used: 11 << 30,
                ram_total: 16 << 30,
                top: vec![ProcRow { name: "chrome".into(), cpu: 4.5, ram: 2 << 30 }],
            }),
            month: (2026, 11),
            hover: Some(Hit::Wifi),
            ..Default::default()
        }
    }

    #[test]
    fn draws_every_mode() {
        let mut cache = Cache::new();
        let results = vec![AppEntry { name: "Calculadora".into(), id: "calc".into() }];
        let views = [
            PanelView::default(),
            full_view(96),
            full_view(144),
            PanelView { gear_open: true, ..full_view(96) },
            PanelView { tab: Tab::Calendar, ..full_view(96) },
            PanelView { tab: Tab::Speed, speed: SpeedState::Error("Sem conexão".into()), ..full_view(96) },
            PanelView { query: "calc".into(), results, ..full_view(96) },
            PanelView { query: "zzz".into(), ..full_view(96) },
            PanelView { audio_list: Some(vec!["Fone".into()]), ..full_view(96) },
        ];
        for view in &views {
            let (bmp, drawn) = render(view, &mut cache);
            drawn.unwrap();
            // The tint covers everything; the search card adds to it.
            let alpha = |x: i32, y: i32| bmp.pixels()[((y * bmp.width + x) * 4 + 3) as usize];
            let s = scale(view.dpi);
            assert!(alpha(2, 2) > 0, "tint");
            assert!(alpha((100.0 * s) as i32, (20.0 * s) as i32) > alpha(2, 2), "search card");
        }
    }

    #[test]
    fn tint_index_matches_presets() {
        let accent = 0x60CDFF;
        assert_eq!(tint_index(&Colors::default()), Some(0));
        assert_eq!(tint_index(&Colors { accent, tint: (0xFFFFFF, 0.18) }), Some(1));
        assert_eq!(tint_index(&Colors { accent, tint: (accent, 0.15) }), Some(2));
        assert_eq!(tint_index(&Colors { accent, tint: (0x123456, 0.5) }), None);
    }

    #[test]
    fn numbers_use_decimal_comma() {
        assert_eq!(bytes(2_254_857_830), "2,1 GB");
        assert_eq!(bytes(600 << 20), "600 MB");
        assert_eq!(decimal(4.5), "4,5");
    }
}
