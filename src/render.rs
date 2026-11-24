//! Draws the strip (icon + percent per device) into a premultiplied BGRA DIB
//! with Direct2D/DirectWrite, ready for `UpdateLayeredWindow`.
#![allow(dead_code)]

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::PCWSTR;

use crate::model::{Charge, DeviceKind, ItemView, is_low};

pub struct Strip {
    pub items: Vec<(DeviceKind, ItemView)>,
    pub dark: bool,
}

/// 32 bpp top-down DIB selected into its own memory DC; freed on drop.
pub struct Bitmap {
    pub width: i32,
    pub height: i32,
    pub dib: HBITMAP,
    pub dc: HDC,
    bits: *mut u8,
    old: HGDIOBJ,
}

impl Bitmap {
    fn new(width: i32, height: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, dib.into());
            Some(Self { width, height, dib, dc, bits: bits.cast(), old })
        }
    }

    /// BGRA premultiplied, row-major, `width * height * 4` bytes.
    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize) }
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.dib.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

const TEXT_FONT: PCWSTR = windows::core::w!("Segoe UI Variable Text");
const ICON_FONT: PCWSTR = windows::core::w!("Segoe Fluent Icons");
const BOLT: char = '\u{E945}';

fn device_glyph(kind: DeviceKind) -> char {
    match kind {
        DeviceKind::Mouse => '\u{E962}',
        DeviceKind::Controller => '\u{E7FC}',
        DeviceKind::Phone => '\u{E8EA}',
    }
}

fn rgb(hex: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: alpha,
    }
}

/// One piece of text drawn in one font and color at `x`.
struct Run {
    layout: IDWriteTextLayout,
    width: f32,
    x: f32,
    color: D2D1_COLOR_F,
}

pub struct Renderer {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
}

impl Renderer {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Self { d2d, dw })
        }
    }

    fn format(&self, family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32) -> windows::core::Result<IDWriteTextFormat> {
        unsafe {
            let f = self.dw.CreateTextFormat(
                family,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                windows::core::w!(""),
            )?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(f)
        }
    }

    fn run(&self, text: &str, format: &IDWriteTextFormat, height: f32, x: f32, color: D2D1_COLOR_F) -> windows::core::Result<Run> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            let layout = self.dw.CreateTextLayout(&wide, format, 10_000.0, height)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            Ok(Run { layout, width: m.widthIncludingTrailingWhitespace, x, color })
        }
    }

    /// Draws `runs` into a fresh transparent bitmap of `width x height`.
    fn draw(&self, runs: &[Run], width: i32, height: i32) -> Option<Bitmap> {
        let bmp = Bitmap::new(width, height)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE, // no GPU device: keeps memory low
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        unsafe {
            let rt = self.d2d.CreateDCRenderTarget(&props).ok()?;
            let rect = windows::Win32::Foundation::RECT { left: 0, top: 0, right: width, bottom: height };
            rt.BindDC(bmp.dc, &rect).ok()?;
            rt.BeginDraw();
            rt.Clear(Some(&rgb(0, 0.0)));
            // ClearType can't blend onto transparent pixels.
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            for run in runs {
                let brush = rt.CreateSolidColorBrush(&run.color, None).ok()?;
                rt.DrawTextLayout(windows_numerics::Vector2 { X: run.x, Y: 0.0 }, &run.layout, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            rt.EndDraw(None, None).ok()?;
        }
        Some(bmp)
    }

    pub fn render(&self, strip: &Strip, dpi: u32, height_px: i32) -> Option<Bitmap> {
        let visible: Vec<_> = strip.items.iter().filter(|(_, v)| *v != ItemView::Hidden).collect();
        if visible.is_empty() || height_px <= 0 {
            return None;
        }
        let scale = dpi.max(96) as f32 / 96.0;
        let h = height_px as f32;
        let text_fmt = self.format(TEXT_FONT, DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.0 * scale).ok()?;
        let icon_fmt = self.format(ICON_FONT, DWRITE_FONT_WEIGHT_NORMAL, 14.0 * scale).ok()?;
        let (base, low) = if strip.dark { (0xFFFFFF, 0xFF6B5E) } else { (0x1A1A1A, 0xC42B1C) };

        let pad = 4.0 * scale;
        let mut x = pad;
        let mut runs = Vec::new();
        for (i, (kind, view)) in visible.iter().enumerate() {
            if i > 0 {
                x += 14.0 * scale;
            }
            let (text, color, charging) = match view {
                ItemView::Value(r) => {
                    let hex = if is_low(r) { low } else { base };
                    (format!("{}%", r.percent), rgb(hex, 1.0), r.charge == Charge::Charging)
                }
                _ => ("--%".to_string(), rgb(base, 0.55), false),
            };
            let icon = self.run(&device_glyph(*kind).to_string(), &icon_fmt, h, x, color).ok()?;
            x += icon.width + 4.0 * scale;
            runs.push(icon);
            let label = self.run(&text, &text_fmt, h, x, color).ok()?;
            x += label.width;
            runs.push(label);
            if charging {
                x += 2.0 * scale;
                let bolt = self.run(&BOLT.to_string(), &icon_fmt, h, x, color).ok()?;
                x += bolt.width;
                runs.push(bolt);
            }
        }
        let width = (x + pad).ceil() as i32;
        self.draw(&runs, width, height_px)
    }

    pub fn render_glyph_icon(&self, glyph: char, dark: bool, size_px: i32) -> HICON {
        let color = rgb(if dark { 0xFFFFFF } else { 0x1A1A1A }, 1.0);
        let size = size_px as f32;
        let bmp = self.format(ICON_FONT, DWRITE_FONT_WEIGHT_NORMAL, size * 0.85).ok().and_then(|fmt| {
            let wide: Vec<u16> = glyph.to_string().encode_utf16().collect();
            unsafe {
                fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER).ok()?;
                let layout = self.dw.CreateTextLayout(&wide, &fmt, size, size).ok()?;
                self.draw(&[Run { layout, width: size, x: 0.0, color }], size_px, size_px)
            }
        });
        let Some(bmp) = bmp else {
            return HICON::default();
        };
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
            let mask = CreateBitmap(size_px, size_px, 1, 1, None);
            let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: bmp.dib };
            let icon = CreateIconIndirect(&info).unwrap_or_default();
            let _ = DeleteObject(mask.into());
            icon
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Reading;

    fn value(p: u8) -> ItemView {
        ItemView::Value(Reading { percent: p, charge: Charge::Discharging })
    }

    fn max_alpha(b: &Bitmap) -> u8 {
        b.pixels().chunks(4).map(|px| px[3]).max().unwrap()
    }

    fn alpha_at(b: &Bitmap, x: i32, y: i32) -> u8 {
        b.pixels()[((y * b.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn empty_strip_renders_nothing() {
        let r = Renderer::new().unwrap();
        let strip = Strip { items: vec![(DeviceKind::Mouse, ItemView::Hidden)], dark: true };
        assert!(r.render(&strip, 96, 48).is_none());
    }

    #[test]
    fn renders_visible_pixels() {
        let r = Renderer::new().unwrap();
        let strip = Strip { items: vec![(DeviceKind::Controller, value(64))], dark: true };
        let b = r.render(&strip, 96, 48).unwrap();
        assert_eq!(b.height, 48);
        assert!(max_alpha(&b) > 200);
        for (x, y) in [(0, 0), (b.width - 1, 0), (0, 47), (b.width - 1, 47)] {
            assert_eq!(alpha_at(&b, x, y), 0, "corner {x},{y}");
        }
    }

    #[test]
    fn more_items_is_wider() {
        let r = Renderer::new().unwrap();
        let one = Strip { items: vec![(DeviceKind::Mouse, value(93))], dark: true };
        let three = Strip {
            items: vec![(DeviceKind::Mouse, value(93)), (DeviceKind::Controller, value(64)), (DeviceKind::Phone, value(30))],
            dark: true,
        };
        assert!(r.render(&three, 96, 48).unwrap().width > r.render(&one, 96, 48).unwrap().width);
    }

    #[test]
    fn scales_with_dpi() {
        let r = Renderer::new().unwrap();
        let strip = Strip { items: vec![(DeviceKind::Mouse, value(93)), (DeviceKind::Phone, value(30))], dark: true };
        let w96 = r.render(&strip, 96, 48).unwrap().width as f32;
        let w144 = r.render(&strip, 144, 72).unwrap().width as f32;
        assert!(w144 >= w96 * 1.4, "{w96} -> {w144}");
    }

    #[test]
    fn dash_is_dimmer() {
        let r = Renderer::new().unwrap();
        let v = r.render(&Strip { items: vec![(DeviceKind::Mouse, value(93))], dark: true }, 96, 48).unwrap();
        let d = r.render(&Strip { items: vec![(DeviceKind::Mouse, ItemView::Dash)], dark: true }, 96, 48).unwrap();
        assert!(max_alpha(&d) < max_alpha(&v), "{} vs {}", max_alpha(&d), max_alpha(&v));
    }

    #[test]
    fn charging_adds_bolt() {
        let r = Renderer::new().unwrap();
        let plain = r.render(&Strip { items: vec![(DeviceKind::Phone, value(50))], dark: true }, 96, 48).unwrap();
        let charging = ItemView::Value(Reading { percent: 50, charge: Charge::Charging });
        let bolt = r.render(&Strip { items: vec![(DeviceKind::Phone, charging)], dark: true }, 96, 48).unwrap();
        assert!(bolt.width > plain.width);
    }

    #[test]
    fn glyph_icon_is_created() {
        let r = Renderer::new().unwrap();
        let icon = r.render_glyph_icon('\u{E83F}', true, 32);
        assert!(!icon.is_invalid());
        unsafe { windows::Win32::UI::WindowsAndMessaging::DestroyIcon(icon).unwrap() };
    }
}
