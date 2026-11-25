//! What is playing, and its transport controls, from the system media
//! session (`GlobalSystemMediaTransportControlsSessionManager`), which
//! Spotify, browsers and other apps report to.
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// Side of the cover, in pixels.
pub const COVER_PX: u32 = 48;
/// Thumbnails larger than this are not read.
const MAX_THUMBNAIL: u64 = 16 << 20;
/// After a command, the player needs a moment before it reports the new state.
const SETTLE: Duration = Duration::from_millis(300);

#[derive(Clone, Default, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Premultiplied BGRA, `COVER_PX` x `COVER_PX`.
    pub cover: Option<Vec<u8>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaCmd {
    Prev,
    PlayPause,
    Next,
}

/// The centered square of a `width` x `height` image (covers are shown square).
pub fn center_square(width: u32, height: u32) -> WICRect {
    let side = width.min(height);
    WICRect { X: ((width - side) / 2) as i32, Y: ((height - side) / 2) as i32, Width: side as i32, Height: side as i32 }
}

/// The cover already read for the same track, so it is not decoded again.
pub fn known_cover(previous: Option<&NowPlaying>, title: &str, artist: &str) -> Option<Vec<u8>> {
    previous.filter(|p| p.title == title && p.artist == artist).and_then(|p| p.cover.clone())
}

fn session() -> Option<GlobalSystemMediaTransportControlsSession> {
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().ok()?.join().ok()?;
    manager.GetCurrentSession().ok()
}

/// What is playing now; `None` without a media session. Blocking: call from
/// a worker thread with COM initialized. `previous` lends its cover when the
/// track is the same.
pub fn current(previous: Option<&NowPlaying>) -> Option<NowPlaying> {
    let session = session()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    if title.is_empty() && artist.is_empty() {
        return None;
    }
    let playing = session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .is_ok_and(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
    let cover = known_cover(previous, &title, &artist).or_else(|| props.Thumbnail().ok().and_then(|t| cover(&t)));
    Some(NowPlaying { title, artist, playing, cover })
}

/// Sends `cmd` to the current media session (blocking; worker thread).
pub fn send(cmd: MediaCmd) {
    let Some(session) = session() else { return };
    let op = match cmd {
        MediaCmd::Prev => session.TrySkipPreviousAsync(),
        MediaCmd::PlayPause => session.TryTogglePlayPauseAsync(),
        MediaCmd::Next => session.TrySkipNextAsync(),
    };
    let _ = op.and_then(|op| op.join());
}

/// The thumbnail's bytes, decoded and reduced to the cover.
fn cover(thumbnail: &IRandomAccessStreamReference) -> Option<Vec<u8>> {
    let stream = thumbnail.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok().filter(|&n| n > 0 && n <= MAX_THUMBNAIL)? as u32;
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    decode(&bytes).ok()
}

/// Any image WIC reads → its center square scaled to `COVER_PX`, premultiplied BGRA.
fn decode(bytes: &[u8]) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        // The WIC stream reads `bytes` in place; it is dropped before them.
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let frame = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?.GetFrame(0)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let (mut w, mut h) = (0, 0);
        converter.GetSize(&mut w, &mut h)?;
        if w == 0 || h == 0 {
            return Err(windows::core::Error::empty());
        }
        let clipper = wic.CreateBitmapClipper()?;
        clipper.Initialize(&converter, &center_square(w, h))?;
        let scaler = wic.CreateBitmapScaler()?;
        scaler.Initialize(&clipper, COVER_PX, COVER_PX, WICBitmapInterpolationModeFant)?;
        let mut pixels = vec![0u8; (COVER_PX * COVER_PX * 4) as usize];
        scaler.CopyPixels(std::ptr::null(), COVER_PX * 4, &mut pixels)?;
        Ok(pixels)
    }
}

/// On a worker thread (own COM apartment): sends `cmd` if any, then reads
/// what is playing and announces it to `owner` by `WM_PANEL_ASYNC`.
/// Dropping the receiver (panel closed, or a newer request) discards the result.
pub fn spawn(owner: HWND, cmd: Option<MediaCmd>, previous: Option<NowPlaying>) -> Receiver<Option<NowPlaying>> {
    let (tx, rx) = mpsc::channel();
    let owner = owner.0 as isize; // HWND is not Send
    std::thread::spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        if let Some(cmd) = cmd {
            send(cmd);
            std::thread::sleep(SETTLE);
        }
        let now = current(previous.as_ref());
        if com {
            unsafe { CoUninitialize() };
        }
        if tx.send(now).is_ok() {
            let (msg, source) = (crate::panel::WM_PANEL_ASYNC, WPARAM(crate::panel::ASYNC_MEDIA));
            let _ = unsafe { PostMessageW(Some(HWND(owner as _)), msg, source, LPARAM(0)) };
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(r: WICRect) -> (i32, i32, i32, i32) {
        (r.X, r.Y, r.Width, r.Height)
    }

    #[test]
    fn square_is_centered() {
        assert_eq!(rect(center_square(300, 300)), (0, 0, 300, 300));
        assert_eq!(rect(center_square(1280, 720)), (280, 0, 720, 720));
        assert_eq!(rect(center_square(100, 151)), (0, 25, 100, 100));
    }

    #[test]
    fn cover_is_reused_for_the_same_track() {
        let prev = NowPlaying { title: "A".into(), artist: "B".into(), playing: true, cover: Some(vec![1, 2]) };
        assert_eq!(known_cover(Some(&prev), "A", "B"), Some(vec![1, 2]));
        assert_eq!(known_cover(Some(&prev), "C", "B"), None);
        assert_eq!(known_cover(Some(&prev), "A", "D"), None);
        assert_eq!(known_cover(None, "A", "B"), None);
        let bare = NowPlaying { cover: None, ..prev };
        assert_eq!(known_cover(Some(&bare), "A", "B"), None);
    }

    /// A `width` x 2 BMP (24 bpp, bottom-up), left half red, right half blue.
    fn two_color_bmp(width: u32) -> Vec<u8> {
        let stride = (width * 3).div_ceil(4) * 4;
        let mut row = vec![0u8; stride as usize];
        for x in 0..width {
            let bgr = if x < width / 2 { [0, 0, 255] } else { [255, 0, 0] };
            row[(x * 3) as usize..][..3].copy_from_slice(&bgr);
        }
        let data = stride * 2;
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + data).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(width as i32).to_le_bytes());
        bmp.extend_from_slice(&2i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&row);
        bmp.extend_from_slice(&row);
        bmp
    }

    #[test]
    fn decode_crops_and_scales_to_cover() {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let pixels = decode(&two_color_bmp(8));
        let garbage = decode(b"not an image");
        if com {
            unsafe { CoUninitialize() };
        }
        assert!(garbage.is_err());
        let pixels = pixels.unwrap();
        assert_eq!(pixels.len(), (COVER_PX * COVER_PX * 4) as usize);
        // The center 2x2 square (columns 3..5) is half red, half blue.
        let at = |x: u32, y: u32| &pixels[((y * COVER_PX + x) * 4) as usize..][..4];
        assert_eq!(at(0, 0), [0, 0, 255, 255]);
        assert_eq!(at(COVER_PX - 1, COVER_PX - 1), [255, 0, 0, 255]);
    }

    /// Read-only look at what is playing: `cargo test -- --ignored media_reads`.
    #[test]
    #[ignore]
    fn media_reads_current() {
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let now = current(None);
        println!("{:?}", now.as_ref().map(|n| (&n.title, &n.artist, n.playing, n.cover.as_ref().map(Vec::len))));
        if com {
            unsafe { CoUninitialize() };
        }
    }
}
