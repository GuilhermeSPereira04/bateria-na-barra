//! Paints a `PanelView` with Direct2D over the transparent (glass) target.
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::PCWSTR;
use windows_numerics::{Matrix3x2, Vector2};

use super::layout::{INSET, Layout, MAX_OUTPUTS, MAX_ROWS, ROW_H, SLIDER_LABEL_W, TABS_PAD, scale};
use super::view::{AudioMenu, Hit, PanelView, SliderKind, Tab, Toggle};
use crate::apps::AppEntry;
use crate::config::{ACCENTS, Accent, Colors, GlassTint};
use crate::media::COVER_PX;
use crate::model::ItemView;
use crate::render::{BOLT, ICON_FONT, TEXT_FONT, device_glyph, item_style, rgb};
use crate::speedtest::{Phase, SpeedState};

const CARD: f32 = 0.08; // white over the dark glass
const HOVER: f32 = 0.06;
const SELECTED: f32 = 0.10;
/// Corner radius of the big cards, in DIP.
const CARD_RADIUS: f32 = 20.0;
/// Knob and groove of the bars, in DIP.
const KNOB: f32 = 8.0;
const GROOVE: f32 = 6.0;
/// Result icons, in DIP.
const ICON: f32 = 20.0;
/// The media cover and its distance from the card's left edge, in DIP.
const COVER: f32 = 48.0;
const COVER_INSET: f32 = 12.0;
const TEXT: u32 = 0xFFFFFF;
const TEXT_ON_LIGHT: u32 = 0x1B1B1B;
const ERROR: u32 = 0xFF6B5E;
/// Text over a light accent fill (the default blue takes white).
const ON_LIGHT_ACCENT: u32 = 0x0E1A12;
const UNAVAILABLE: f32 = 0.45;
const ACCENT_WINDOWS: &str = "Usar a cor do Windows";

const GLYPH_SEARCH: char = '\u{E721}';
const GLYPH_GEAR: char = '\u{E713}';
const GLYPH_MORE: char = '\u{E70D}';
const GLYPH_CHECK: char = '\u{E73E}';
const GLYPH_VOLUME: char = '\u{E767}';
const GLYPH_MUTE: char = '\u{E74F}';
const GLYPH_PREV: char = '\u{E892}';
const GLYPH_PLAY: char = '\u{E768}';
const GLYPH_PAUSE: char = '\u{E769}';
const GLYPH_NEXT: char = '\u{E893}';
const GLYPH_LEFT: char = '\u{E76B}';
const GLYPH_RIGHT: char = '\u{E76C}';
const GLYPH_SPEAKER: char = '\u{E7F5}';
const GLYPH_HEADPHONE: char = '\u{E7F6}';
const GLYPH_SCREEN: char = '\u{E7F4}';

const MONTHS: [&str; 12] =
    ["Janeiro", "Fevereiro", "Março", "Abril", "Maio", "Junho", "Julho", "Agosto", "Setembro", "Outubro", "Novembro", "Dezembro"];

/// Text formats (per DPI), the brush, the result icons and the media cover
/// (per render target), kept between paints.
pub struct Cache {
    dpi: u32,
    fonts: Option<Fonts>,
    brush: Option<ID2D1SolidColorBrush>,
    /// Vertical gradient of the light edge on cards and pills.
    edge: Option<ID2D1LinearGradientBrush>,
    /// Bitmaps of the results' icons, by app id.
    icons: Vec<(String, ID2D1Bitmap)>,
    /// The media cover's bitmap and the pixels it was made from.
    cover: Option<(Vec<u8>, ID2D1Bitmap)>,
}

struct Fonts {
    text: IDWriteTextFormat,
    bold: IDWriteTextFormat,
    small: IDWriteTextFormat,
    battery: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_big: IDWriteTextFormat,
    icon_xl: IDWriteTextFormat,
    title: IDWriteTextFormat,
}

impl Cache {
    pub fn new() -> Self {
        Self { dpi: 0, fonts: None, brush: None, edge: None, icons: Vec::new(), cover: None }
    }

    /// Forgets what belongs to a render target that is being replaced.
    pub fn reset_target(&mut self) {
        self.brush = None;
        self.edge = None;
        self.icons.clear();
        self.cover = None;
    }

    /// Re-creates the cover's bitmap only when its pixels change.
    fn sync_cover(&mut self, rt: &ID2D1RenderTarget, cover: Option<&Vec<u8>>) {
        match cover {
            Some(px) if self.cover.as_ref().is_some_and(|(kept, _)| kept == px) => {}
            Some(px) if px.len() == (COVER_PX * COVER_PX * 4) as usize => {
                self.cover = bitmap(rt, COVER_PX, px).ok().map(|bmp| (px.clone(), bmp));
            }
            _ => self.cover = None,
        }
    }

    /// Keeps bitmaps for exactly the visible results that have an icon.
    fn sync_icons(&mut self, rt: &ID2D1RenderTarget, results: &[AppEntry]) {
        let visible = &results[..results.len().min(MAX_ROWS)];
        self.icons.retain(|(id, _)| visible.iter().any(|a| &a.id == id && a.icon.is_some()));
        for app in visible {
            let Some(icon) = &app.icon else { continue };
            if self.icons.iter().any(|(id, _)| id == &app.id) {
                continue;
            }
            if let Ok(bmp) = bitmap(rt, icon.size, &icon.bgra) {
                self.icons.push((app.id.clone(), bmp));
            }
        }
    }
}

/// A `side` x `side` bitmap from premultiplied BGRA pixels.
fn bitmap(rt: &ID2D1RenderTarget, side: u32, bgra: &[u8]) -> windows::core::Result<ID2D1Bitmap> {
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let size = D2D_SIZE_U { width: side, height: side };
    unsafe { rt.CreateBitmap(size, Some(bgra.as_ptr() as _), side * 4, &props) }
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
            icon_xl: format(dw, ICON_FONT, DWRITE_FONT_WEIGHT_NORMAL, 22.0 * s)?,
            title: format(dw, TEXT_FONT, DWRITE_FONT_WEIGHT_SEMI_BOLD, 15.0 * s)?,
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
    edge: &'a ID2D1LinearGradientBrush,
    f: &'a Fonts,
    s: f32,
    icons: &'a [(String, ID2D1Bitmap)],
    cover: Option<&'a ID2D1Bitmap>,
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

/// Radius that rounds the ends of `r` completely.
fn pill_radius(r: D2D_RECT_F) -> f32 {
    ((r.right - r.left).min(r.bottom - r.top)) / 2.0
}

/// Text color over an accent fill: white, unless the accent is a light color.
fn on_accent(accent: u32) -> u32 {
    let (r, g, b) = (((accent >> 16) & 0xFF) as f32, ((accent >> 8) & 0xFF) as f32, (accent & 0xFF) as f32);
    if (0.299 * r + 0.587 * g + 0.114 * b) / 255.0 > 0.6 { ON_LIGHT_ACCENT } else { TEXT }
}

thread_local! {
    static FOREGROUND: std::cell::Cell<u32> = const { std::cell::Cell::new(TEXT) };
}

/// Text / glyph / overlay color: white on the dark glass, near-black on the light one.
pub fn foreground(tint: GlassTint) -> u32 {
    if tint == GlassTint::Light { TEXT_ON_LIGHT } else { TEXT }
}

fn white(alpha: f32) -> D2D1_COLOR_F {
    rgb(FOREGROUND.with(|f| f.get()), alpha)
}

impl Painter<'_> {
    fn px(&self, dip: f32) -> f32 {
        dip * self.s
    }

    fn color(&self, c: D2D1_COLOR_F) {
        unsafe { self.brush.SetColor(&c) };
    }

    /// Rounded rectangle with corner radius `radius` (pixels).
    fn round(&self, r: D2D_RECT_F, radius: f32, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, self.brush) };
    }

    /// Fully round ends (a circle when the rectangle is square).
    fn pill(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        self.round(r, pill_radius(r), c);
    }

    /// The soft shadow around a shape: two faint rings just outside it.
    fn shadow(&self, r: D2D_RECT_F, radius: f32) {
        for (grow, dy, alpha) in [(1.0, 1.0, 0.10), (2.0, 2.0, 0.05)] {
            let (g, dy) = (self.px(grow), self.px(dy));
            let rr = D2D1_ROUNDED_RECT {
                rect: rect(r.left - g, r.top - g + dy, r.right + g, r.bottom + g + dy),
                radiusX: radius + g,
                radiusY: radius + g,
            };
            self.color(rgb(0, alpha));
            unsafe { self.rt.DrawRoundedRectangle(&rr, self.brush, g * 2.0, None) };
        }
    }

    /// The 1 px light edge: bright at the top, fading toward the bottom.
    fn edge(&self, r: D2D_RECT_F, radius: f32) {
        let half = self.px(0.5);
        let rr = D2D1_ROUNDED_RECT { rect: rect(r.left + half, r.top + half, r.right - half, r.bottom - half), radiusX: radius, radiusY: radius };
        unsafe {
            self.edge.SetStartPoint(Vector2 { X: 0.0, Y: r.top });
            self.edge.SetEndPoint(Vector2 { X: 0.0, Y: r.bottom });
            self.rt.DrawRoundedRectangle(&rr, self.edge, self.px(1.0), None);
        }
    }

    /// A big card: shadow, fill and light edge.
    fn card(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        let radius = self.px(CARD_RADIUS);
        self.shadow(r, radius);
        self.round(r, radius, c);
        self.edge(r, radius);
    }

    /// A pill or circle with shadow and light edge.
    fn pill_card(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        let radius = pill_radius(r);
        self.shadow(r, radius);
        self.round(r, radius, c);
        self.edge(r, radius);
    }

    /// Hover highlight: round, at most a card's radius.
    fn hover(&self, r: D2D_RECT_F) {
        self.round(r, pill_radius(r).min(self.px(CARD_RADIUS)), white(HOVER));
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
    let shown: &[AppEntry] = if view.query.is_empty() { &[] } else { &view.results };
    cache.sync_icons(rt, shown);
    cache.sync_cover(rt, view.media.as_ref().and_then(|m| m.cover.as_ref()));
    if cache.edge.is_none() {
        cache.edge = Some(edge_brush(rt)?);
    }
    let (Some(f), Some(brush), Some(edge)) = (&cache.fonts, &cache.brush, &cache.edge) else { return Ok(()) };
    let cover = cache.cover.as_ref().map(|(_, bmp)| bmp);
    let p = Painter { rt, dw, brush, edge, f, s, icons: &cache.icons, cover };
    // ClearType can't blend onto transparent pixels.
    unsafe { rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };

    // With the acrylic backdrop the tint comes from the system; without it
    // (fallback) paint a milder layer over the plain backdrop.
    FOREGROUND.with(|f| f.set(foreground(view.colors.tint_choice)));
    if !view.acrylic {
        let (tint, alpha) = view.colors.tint;
        p.color(rgb(tint, alpha * 0.5));
        unsafe { rt.FillRectangle(&rect(0.0, 0.0, layout.size.0, layout.size.1), brush) };
    }

    unsafe { rt.SetTransform(&Matrix3x2::translation(view.shift, 0.0)) };
    search(&p, view, layout);
    if view.gear_open {
        colors_row(&p, &view.colors, layout);
    }
    quick(&p, view, layout);
    battery(&p, view, layout);
    media(&p, view, layout);
    lower(&p, view, layout);
    if let Some(menu) = &view.audio_menu {
        audio_menu(&p, view, menu, layout);
    }

    if let Some(r) = view.hover.filter(|h| !matches!(h, Hit::Slider(_) | Hit::Blank | Hit::AudioDevice(_))).and_then(|h| layout.rect(h)) {
        p.hover(r);
    }
    // Opening / closing animation: black over everything while the shape is a pill.
    unsafe { rt.SetTransform(&Matrix3x2::identity()) };
    if view.veil > 0.0 {
        p.color(rgb(0, view.veil));
        unsafe { rt.FillRectangle(&rect(0.0, 0.0, layout.size.0, layout.size.1), brush) };
    }
    Ok(())
}

fn edge_brush(rt: &ID2D1RenderTarget) -> windows::core::Result<ID2D1LinearGradientBrush> {
    let stops = [
        D2D1_GRADIENT_STOP { position: 0.0, color: rgb(0xFFFFFF, 0.22) },
        D2D1_GRADIENT_STOP { position: 1.0, color: rgb(0xFFFFFF, 0.03) },
    ];
    unsafe {
        let stops = rt.CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: Vector2 { X: 0.0, Y: 0.0 }, endPoint: Vector2 { X: 0.0, Y: 1.0 } };
        rt.CreateLinearGradientBrush(&props, None, &stops)
    }
}

fn search(p: &Painter, view: &PanelView, l: &Layout) {
    let r = l.search;
    p.pill_card(r, white(CARD));
    p.glyph(GLYPH_SEARCH, &p.f.icon, rect(r.left + p.px(14.0), r.top, r.left + p.px(38.0), r.bottom), white(0.6));
    let text = rect(r.left + p.px(44.0), r.top, r.right - p.px(16.0), r.bottom);
    if view.query.is_empty() {
        p.text("Pesquisar apps…", &p.f.text, text, Align::Left, white(0.6));
    } else {
        p.text(&view.query, &p.f.text, text, Align::Left, white(0.92));
    }
    if let Some(g) = l.rect(Hit::Gear) {
        p.pill_card(g, white(if view.gear_open { 0.16 } else { CARD }));
        p.glyph(GLYPH_GEAR, &p.f.icon_big, g, white(0.85));
    }
}

fn colors_row(p: &Painter, colors: &Colors, l: &Layout) {
    for (i, &hex) in ACCENTS.iter().enumerate() {
        let Some(r) = l.rect(Hit::Accent(i)) else { continue };
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
        p.circle(cx, cy, p.px(10.0), rgb(hex, 1.0), None);
        if colors.accent_choice == Accent::Preset(i) {
            p.circle(cx, cy, p.px(13.0), white(0.9), Some(p.px(1.5)));
        }
    }
    if let Some(r) = l.rect(Hit::AccentWindows) {
        if colors.accent_choice == Accent::Windows {
            p.pill(r, rgb(colors.accent, 1.0));
            p.text(ACCENT_WINDOWS, &p.f.bold, r, Align::Center, rgb(on_accent(colors.accent), 0.95));
        } else {
            p.pill_card(r, white(CARD));
            p.text(ACCENT_WINDOWS, &p.f.text, r, Align::Center, white(0.92));
        }
    }
    if let Some(track) = l.rect(Hit::Slider(SliderKind::Transparency)) {
        let label = rect(track.left - p.px(SLIDER_LABEL_W), track.top, track.left - p.px(8.0), track.bottom);
        p.text("Transparência", &p.f.text, label, Align::Left, white(0.92));
        slider(p, track, colors.transparency, colors.accent);
        let value = rect(track.right + p.px(14.0), track.top, track.right + p.px(64.0), track.bottom);
        p.text(&format!("{}%", colors.transparency), &p.f.text, value, Align::Left, white(0.92));
    }
    let selected = GlassTint::ALL.iter().position(|&t| t == colors.tint_choice);
    for (i, name) in ["Escuro", "Claro", "Destaque"].into_iter().enumerate() {
        let Some(r) = l.rect(Hit::Tint(i)) else { continue };
        if selected == Some(i) {
            p.pill(r, rgb(colors.accent, 1.0));
            p.text(name, &p.f.bold, r, Align::Center, rgb(on_accent(colors.accent), 0.95));
        } else {
            p.pill_card(r, white(CARD));
            p.text(name, &p.f.text, r, Align::Center, white(0.92));
        }
    }
}

/// A bar: groove, the filled part in the accent color and the knob.
fn slider(p: &Painter, track: D2D_RECT_F, value: u8, accent: u32) {
    let cy = (track.top + track.bottom) / 2.0;
    let half = p.px(GROOVE / 2.0);
    let x = track.left + (track.right - track.left) * f32::from(value.min(100)) / 100.0;
    let groove = |right: f32, c: D2D1_COLOR_F| {
        p.color(c);
        let r = D2D_RECT_F { left: track.left, top: cy - half, right, bottom: cy + half };
        unsafe { p.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: half, radiusY: half }, p.brush) };
    };
    groove(track.right, white(0.22));
    if x > track.left {
        groove(x, rgb(accent, 1.0));
    }
    p.circle(x, cy + p.px(1.0), p.px(KNOB), rgb(0, 0.28), None);
    p.circle(x, cy, p.px(KNOB), rgb(TEXT, 1.0), None);
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
                p.pill_card(card, rgb(view.colors.accent, 1.0));
                (rgb(on_accent(view.colors.accent), 0.95), sub)
            }
            Toggle::Off => {
                p.pill_card(card, white(CARD));
                (white(0.92), sub)
            }
            Toggle::Unavailable => {
                p.pill_card(card, white(CARD));
                (white(UNAVAILABLE), "Indisponível".to_string())
            }
        };
        let mid = (m.top + m.bottom) / 2.0;
        let mut x = m.left + p.px(20.0);
        if let Some(g) = glyph {
            p.glyph(g, &p.f.icon, rect(x, m.top + p.px(8.0), x + p.px(16.0), mid + p.px(2.0)), fg);
            x += p.px(20.0);
        }
        p.text(&title, &p.f.bold, rect(x, m.top + p.px(8.0), m.right - p.px(4.0), mid + p.px(2.0)), Align::Left, fg);
        let sub_color = D2D1_COLOR_F { a: fg.a * 0.8, ..fg };
        p.text(&sub, &p.f.small, rect(m.left + p.px(20.0), mid, m.right - p.px(4.0), m.bottom - p.px(8.0)), Align::Left, sub_color);
        let divider = D2D1_COLOR_F { a: 0.25, ..fg };
        p.line((a.left, a.top + p.px(14.0)), (a.left, a.bottom - p.px(14.0)), p.px(1.0), divider);
        p.glyph(GLYPH_MORE, &p.f.icon, a, fg);
    }
}

fn battery(p: &Painter, view: &PanelView, l: &Layout) {
    let r = l.battery;
    p.pill_card(r, white(CARD));
    let visible: Vec<_> = view.battery.iter().filter(|(_, v)| *v != ItemView::Hidden).collect();
    if visible.is_empty() {
        p.text("Nenhum aparelho", &p.f.text, rect(r.left + p.px(20.0), r.top, r.right, r.bottom), Align::Left, white(0.55));
        return;
    }
    let mut x = r.left + p.px(20.0);
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
    p.card(r, white(CARD));
    let end = l.rect(Hit::MediaPrev).map_or(r.right, |b| b.left) - p.px(8.0);
    let mid = (r.top + r.bottom) / 2.0;
    let mut left = r.left + p.px(20.0);
    if let (Some(bmp), Some(_)) = (p.cover, &view.media) {
        let (size, x) = (p.px(COVER), r.left + p.px(COVER_INSET));
        let y = mid - size / 2.0;
        // The cover is painted through a bitmap brush so its corners can be round.
        let k = size / COVER_PX as f32;
        let bitmap_props = D2D1_BITMAP_BRUSH_PROPERTIES {
            extendModeX: D2D1_EXTEND_MODE_CLAMP,
            extendModeY: D2D1_EXTEND_MODE_CLAMP,
            interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
        };
        let brush_props = D2D1_BRUSH_PROPERTIES { opacity: 1.0, transform: Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: x, M32: y } };
        if let Ok(cover) = unsafe { p.rt.CreateBitmapBrush(bmp, Some(&bitmap_props), Some(&brush_props)) } {
            let radius = p.px(10.0);
            let rr = D2D1_ROUNDED_RECT { rect: rect(x, y, x + size, y + size), radiusX: radius, radiusY: radius };
            unsafe { p.rt.FillRoundedRectangle(&rr, &cover) };
        }
        left = x + size + p.px(12.0);
    }
    let playing = match &view.media {
        Some(np) => {
            if np.artist.is_empty() {
                p.text(&np.title, &p.f.title, rect(left, r.top, end, r.bottom), Align::Left, white(0.96));
            } else {
                p.text(&np.title, &p.f.title, rect(left, r.top + p.px(12.0), end, mid), Align::Left, white(0.96));
                p.text(&np.artist, &p.f.text, rect(left, mid, end, r.bottom - p.px(12.0)), Align::Left, white(0.6));
            }
            np.playing
        }
        None => {
            p.text("Nada tocando", &p.f.text, rect(left, r.top, end, r.bottom), Align::Left, white(0.55));
            false
        }
    };
    let alpha = if view.media.is_some() { 0.95 } else { UNAVAILABLE };
    for (hit, g) in [(Hit::MediaPrev, GLYPH_PREV), (Hit::MediaPlay, play_glyph(playing)), (Hit::MediaNext, GLYPH_NEXT)] {
        if let Some(b) = l.rect(hit) {
            p.glyph(g, &p.f.icon_xl, b, white(alpha));
        }
    }
}

/// Pause while playing, play otherwise.
fn play_glyph(playing: bool) -> char {
    if playing { GLYPH_PAUSE } else { GLYPH_PLAY }
}

fn lower(p: &Painter, view: &PanelView, l: &Layout) {
    p.card(l.body, white(CARD));
    if !view.query.is_empty() {
        let items: Vec<(&str, Option<&ID2D1Bitmap>)> = view
            .results
            .iter()
            .map(|a| (a.name.as_str(), p.icons.iter().find(|(id, _)| id == &a.id).map(|(_, b)| b)))
            .collect();
        rows(p, l, &items, Hit::Result, Some(view.selected), "Nada encontrado");
        return;
    }
    // Segmented control: one pill around the tabs, a lighter pill under the selected one.
    if let (Some(first), Some(last)) = (l.rect(Hit::Tab(Tab::Resources)), l.rect(Hit::Tab(Tab::Speed))) {
        let (dx, dy) = (p.px(TABS_PAD.0), p.px(TABS_PAD.1));
        p.pill_card(rect(first.left - dx, first.top - dy, last.right + dx, first.bottom + dy), white(0.07));
    }
    for (tab, name) in [(Tab::Resources, "Recursos"), (Tab::Calendar, "Calendário"), (Tab::Speed, "Speed test")] {
        let Some(r) = l.rect(Hit::Tab(tab)) else { continue };
        if tab == view.tab {
            p.pill(r, white(0.2));
            p.text(name, &p.f.bold, r, Align::Center, white(1.0));
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

/// The audio drop-down over everything: volume bar, then the outputs with a check on the current one.
fn audio_menu(p: &Painter, view: &PanelView, menu: &AudioMenu, l: &Layout) {
    let Some(card) = l.menu else { return };
    // Mostly opaque so it reads over whatever is below.
    let light = view.colors.tint_choice == GlassTint::Light;
    let radius = p.px(CARD_RADIUS);
    p.shadow(card, radius);
    p.round(card, radius, if light { rgb(0xF3F3F3, 0.94) } else { rgb(0x20242C, 0.92) });
    p.round(card, radius, white(0.08));
    p.edge(card, radius);
    if let Some(track) = l.rect(Hit::Slider(SliderKind::Volume)) {
        let icon = rect(card.left + p.px(INSET + 4.0), track.top, track.left - p.px(10.0), track.bottom);
        let glyph = if view.quick.audio == Toggle::Off { GLYPH_MUTE } else { GLYPH_VOLUME };
        p.glyph(glyph, &p.f.icon, icon, white(0.92));
        slider(p, track, menu.volume, view.colors.accent);
        let value = rect(track.right + p.px(14.0), track.top, card.right - p.px(4.0), track.bottom);
        p.text(&format!("{}%", menu.volume), &p.f.text, value, Align::Left, white(0.92));
    }
    if menu.outputs.is_empty() {
        let top = card.top + p.px(36.0);
        p.text("Nenhuma saída de áudio", &p.f.text, rect(card.left + p.px(INSET + 4.0), top, card.right, top + p.px(ROW_H)), Align::Left, white(0.55));
    }
    for (i, name) in menu.outputs.iter().take(MAX_OUTPUTS).enumerate() {
        let Some(r) = l.rect(Hit::AudioDevice(i)) else { continue };
        let current = menu.current == Some(i);
        let inset = p.px(6.0);
        if view.hover == Some(Hit::AudioDevice(i)) {
            p.pill(rect(r.left + inset, r.top + p.px(1.0), r.right - inset, r.bottom - p.px(1.0)), white(HOVER));
        }
        let color = if current { rgb(view.colors.accent, 1.0) } else { white(0.7) };
        let icon = rect(r.left + p.px(INSET + 4.0), r.top, r.left + p.px(INSET + 24.0), r.bottom);
        p.glyph(output_glyph(name), &p.f.icon, icon, color);
        let text = rect(r.left + p.px(INSET + 32.0), r.top, r.right - p.px(INSET + 28.0), r.bottom);
        p.text(name, &p.f.text, text, Align::Left, white(if current { 1.0 } else { 0.85 }));
        if current {
            let check = rect(r.right - p.px(INSET + 24.0), r.top, r.right - p.px(INSET + 4.0), r.bottom);
            p.glyph(GLYPH_CHECK, &p.f.icon, check, rgb(view.colors.accent, 1.0));
        }
    }
}

/// An icon for an output device, guessed from its name.
fn output_glyph(name: &str) -> char {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["fone", "headphone", "headset", "airpods", "buds"]) {
        GLYPH_HEADPHONE
    } else if has(&["hdmi", "displayport", "monitor", "tv", "display"]) {
        GLYPH_SCREEN
    } else {
        GLYPH_SPEAKER
    }
}

/// One line per item (results or audio outputs, with an optional icon), at most `MAX_ROWS`.
fn rows(p: &Painter, l: &Layout, items: &[(&str, Option<&ID2D1Bitmap>)], hit: fn(usize) -> Hit, selected: Option<usize>, empty: &str) {
    if items.is_empty() {
        let b = l.body;
        let r = rect(b.left + p.px(INSET), b.top, b.right, b.top + p.px(ROW_H));
        p.text(empty, &p.f.text, r, Align::Left, white(0.55));
        return;
    }
    for (i, (name, icon)) in items.iter().take(MAX_ROWS).enumerate() {
        let Some(r) = l.rect(hit(i)) else { continue };
        if selected == Some(i) {
            p.pill(rect(r.left + p.px(6.0), r.top + p.px(1.0), r.right - p.px(6.0), r.bottom - p.px(1.0)), white(SELECTED));
        }
        let mut left = r.left + p.px(INSET);
        if let Some(bmp) = icon {
            let size = p.px(ICON);
            let top = (r.top + r.bottom - size) / 2.0;
            let dest = rect(left, top, left + size, top + size);
            unsafe { p.rt.DrawBitmap(*bmp, Some(&dest), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
            left += size + p.px(10.0);
        }
        let text = rect(left, r.top, r.right - p.px(INSET), r.bottom);
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
        let track = rect(left, bar_top, right, bar_top + p.px(10.0));
        p.color(white(0.12));
        let round = |r: D2D_RECT_F| D2D1_ROUNDED_RECT { rect: r, radiusX: p.px(5.0), radiusY: p.px(5.0) };
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
    // Weekday header, Sunday first.
    let top = prev.bottom + p.px(8.0);
    let col = (next.right - prev.left) / 7.0;
    for (i, d) in ["D", "S", "T", "Q", "Q", "S", "S"].into_iter().enumerate() {
        let x = prev.left + i as f32 * col;
        p.text(d, &p.f.small, rect(x, top, x + col, top + p.px(20.0)), Align::Center, white(0.6));
    }
    if !(1..=12).contains(&month) {
        return;
    }
    let grid_top = top + p.px(20.0);
    let row_h = p.px(24.0);
    let is_today_month = view.today.0 == year && view.today.1 == month;
    for (r, week) in crate::calendar::month_grid(year, month).iter().enumerate() {
        for (c, day) in week.iter().enumerate() {
            let Some(day) = *day else { continue };
            let x = prev.left + c as f32 * col;
            let y = grid_top + r as f32 * row_h;
            let cell = rect(x, y, x + col, y + row_h);
            let today = is_today_month && view.today.2 == day as u32;
            if today {
                let (cx, cy) = (x + col / 2.0, y + row_h / 2.0);
                p.circle(cx, cy, row_h / 2.0 - p.px(1.0), rgb(view.colors.accent, 1.0), None);
            }
            let alpha = if today { 1.0 } else { 0.85 };
            p.text(&day.to_string(), &p.f.text, cell, Align::Center, white(alpha));
        }
    }
}

fn speed(p: &Painter, view: &PanelView, l: &Layout) {
    let Some(btn) = l.rect(Hit::SpeedStart) else { return };
    let running = matches!(view.speed, SpeedState::Running { .. });
    if running {
        p.pill_card(btn, white(CARD));
        p.text("Testando…", &p.f.bold, btn, Align::Center, white(UNAVAILABLE));
    } else {
        p.pill(btn, rgb(view.colors.accent, 1.0));
        p.text("Testar", &p.f.bold, btn, Align::Center, rgb(on_accent(view.colors.accent), 0.95));
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
    use crate::apps::Icon;
    use crate::media::NowPlaying;
    use crate::model::{Charge, DeviceKind, Reading};
    use crate::panel::layout::layout;
    use crate::render::Bitmap;
    use crate::sysinfo::{ProcRow, Stats};

    /// A software target bound to `bmp`.
    fn target(bmp: &Bitmap) -> ID2D1RenderTarget {
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
            let rt = d2d.CreateDCRenderTarget(&props).unwrap();
            let rc = windows::Win32::Foundation::RECT { left: 0, top: 0, right: bmp.width, bottom: bmp.height };
            rt.BindDC(bmp.dc, &rc).unwrap();
            rt.into()
        }
    }

    /// Draws `view` into a software target; returns the BGRA pixels.
    fn render(view: &PanelView, cache: &mut Cache) -> (Bitmap, windows::core::Result<()>) {
        let l = layout(view);
        let bmp = Bitmap::new(l.size.0 as i32, l.size.1 as i32).unwrap();
        let rt = target(&bmp);
        unsafe {
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
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
            today: (2026, 11, 25),
            hover: Some(Hit::Wifi),
            ..Default::default()
        }
    }

    #[test]
    fn draws_every_mode() {
        let mut cache = Cache::new();
        let results = vec![AppEntry { name: "Calculadora".into(), id: "calc".into(), icon: None }];
        let views = [
            PanelView::default(),
            full_view(96),
            full_view(144),
            PanelView { gear_open: true, ..full_view(96) },
            PanelView { tab: Tab::Calendar, ..full_view(96) },
            PanelView { tab: Tab::Speed, speed: SpeedState::Error("Sem conexão".into()), ..full_view(96) },
            PanelView { query: "calc".into(), results, ..full_view(96) },
            PanelView { query: "zzz".into(), ..full_view(96) },
            PanelView { audio_menu: Some(AudioMenu { outputs: vec!["Fone".into()], current: Some(0), volume: 40 }), ..full_view(96) },
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
    fn light_tint_uses_dark_foreground() {
        assert_eq!(foreground(GlassTint::Dark), 0xFFFFFF);
        assert_eq!(foreground(GlassTint::Accent), 0xFFFFFF);
        assert_eq!(foreground(GlassTint::Light), 0x1B1B1B);
        // Drawing with the light tint leaves the thread-local dark; a dark draw resets it.
        let mut cache = Cache::new();
        let mut view = full_view(96);
        view.colors.tint_choice = GlassTint::Light;
        view.colors.tint = (0xF3F3F3, 0.55);
        render(&view, &mut cache).1.unwrap();
        assert_eq!(white(1.0).r, 0x1B as f32 / 255.0);
        view.colors.tint_choice = GlassTint::Dark;
        render(&view, &mut cache).1.unwrap();
        assert_eq!(white(1.0).r, 1.0);
    }

    #[test]
    fn result_icon_is_drawn() {
        // An opaque white 32 px icon lands left in the first row.
        let icon = std::sync::Arc::new(Icon { size: 32, bgra: vec![255; 32 * 32 * 4] });
        let results = vec![AppEntry { name: "Calculadora".into(), id: "calc".into(), icon: Some(icon) }];
        let view = PanelView { query: "calc".into(), results, ..full_view(96) };
        let row = layout(&view).rect(Hit::Result(0)).unwrap();
        let (x, y) = ((row.left + INSET + ICON / 2.0) as i32, ((row.top + row.bottom) / 2.0) as i32);
        let mut cache = Cache::new();
        let (bmp, drawn) = render(&view, &mut cache);
        drawn.unwrap();
        let px = &bmp.pixels()[((y * bmp.width + x) * 4) as usize..][..4];
        assert_eq!(px, [255, 255, 255, 255]);
    }

    #[test]
    fn play_glyph_follows_playing() {
        assert_eq!(play_glyph(true), GLYPH_PAUSE);
        assert_eq!(play_glyph(false), GLYPH_PLAY);
    }

    #[test]
    fn cover_is_drawn() {
        // An opaque white cover lands at the left of the media card.
        let cover = vec![255; (COVER_PX * COVER_PX * 4) as usize];
        let media = NowPlaying { title: "Kick Back".into(), artist: "Kenshi Yonezu".into(), playing: false, cover: Some(cover) };
        let view = PanelView { media: Some(media), ..full_view(96) };
        let r = layout(&view).media;
        let (x, y) = ((r.left + COVER_INSET + COVER / 2.0) as i32, ((r.top + r.bottom) / 2.0) as i32);
        let mut cache = Cache::new();
        let (bmp, drawn) = render(&view, &mut cache);
        drawn.unwrap();
        let px = &bmp.pixels()[((y * bmp.width + x) * 4) as usize..][..4];
        assert_eq!(px, [255, 255, 255, 255]);
    }

    #[test]
    fn cover_bitmap_is_kept_while_the_cover_is_the_same() {
        let bmp = Bitmap::new(8, 8).unwrap();
        let rt = target(&bmp);
        let mut cache = Cache::new();
        let a = vec![255; (COVER_PX * COVER_PX * 4) as usize];
        let raw = |c: &Cache| c.cover.as_ref().map(|(_, b)| windows::core::Interface::as_raw(b));
        cache.sync_cover(&rt, Some(&a));
        let first = raw(&cache);
        assert!(first.is_some());
        cache.sync_cover(&rt, Some(&a.clone()));
        assert_eq!(raw(&cache), first);
        cache.sync_cover(&rt, Some(&vec![0; a.len()]));
        assert!(raw(&cache).is_some() && raw(&cache) != first);
        cache.sync_cover(&rt, None);
        assert!(cache.cover.is_none());
    }

    #[test]
    fn accent_windows_label_fits() {
        let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap() };
        for dpi in [96, 144] {
            let view = PanelView { dpi, gear_open: true, ..Default::default() };
            let r = layout(&view).rect(Hit::AccentWindows).unwrap();
            let fonts = Fonts::new(&dw, scale(dpi)).unwrap();
            let wide: Vec<u16> = ACCENT_WINDOWS.encode_utf16().collect();
            let mut m = DWRITE_TEXT_METRICS::default();
            unsafe { dw.CreateTextLayout(&wide, &fonts.text, 10_000.0, 100.0).unwrap().GetMetrics(&mut m).unwrap() };
            let room = r.right - r.left - 2.0 * 8.0 * scale(dpi);
            assert!(m.width <= room, "dpi {dpi}: text {} > room {room}", m.width);
        }
    }

    #[test]
    fn text_on_accent_follows_luminance() {
        assert_eq!(on_accent(0x0A84FF), 0xFFFFFF); // default blue
        assert_eq!(on_accent(0xE6E6E6), ON_LIGHT_ACCENT);
        assert_eq!(on_accent(0xF5E07A), ON_LIGHT_ACCENT);
    }

    #[test]
    fn output_icons_by_name() {
        assert_eq!(output_glyph("Fones de ouvido (Realtek)"), GLYPH_HEADPHONE);
        assert_eq!(output_glyph("LG TV (HDMI)"), GLYPH_SCREEN);
        assert_eq!(output_glyph("Alto-falantes"), GLYPH_SPEAKER);
    }

    #[test]
    fn numbers_use_decimal_comma() {
        assert_eq!(bytes(2_254_857_830), "2,1 GB");
        assert_eq!(bytes(600 << 20), "600 MB");
        assert_eq!(decimal(4.5), "4,5");
    }
}
