//! Start menu apps (Win32 and Store), read from the `shell:AppsFolder`
//! virtual folder, with their icons and a way to open them.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits,
    GetObjectW, HBITMAP,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellItemImageFactory, KF_FLAG_DEFAULT, SIGDN,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY, SHCreateItemFromParsingName, SHGetKnownFolderItem,
    ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_SHOWNORMAL};
use windows::core::{HSTRING, w};

/// Icon size asked from the shell, in pixels.
const ICON_PX: i32 = 32;

/// An app icon as premultiplied BGRA pixels, top-down rows.
pub struct Icon {
    pub size: u32,
    pub bgra: Vec<u8>,
}

#[derive(Clone)]
pub struct AppEntry {
    pub name: String,
    /// AppUserModelID or parsing name.
    pub id: String,
    /// Loaded only for the visible results.
    pub icon: Option<Arc<Icon>>,
}

fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.filter(|s| !s.is_empty())
    }
}

/// Every app of the Start menu, without icons. Needs COM on the calling thread.
pub fn enumerate() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    unsafe {
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return apps };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return apps };
        loop {
            let mut slot = [None];
            let mut fetched = 0;
            if items.Next(&mut slot, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = slot[0].take() else { break };
            if let (Some(name), Some(id)) =
                (display_name(&item, SIGDN_NORMALDISPLAY), display_name(&item, SIGDN_PARENTRELATIVEPARSING))
            {
                apps.push(AppEntry { name, id, icon: None });
            }
        }
    }
    apps
}

/// Enumerates the apps on a worker thread; the list arrives on the returned
/// channel, announced to `owner` by `WM_PANEL_ASYNC` (`ASYNC_APPS`).
pub fn enumerate_async(owner: HWND) -> Receiver<Vec<AppEntry>> {
    let (tx, rx) = mpsc::channel();
    let owner = owner.0 as isize; // HWND is not Send
    std::thread::spawn(move || {
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let apps = enumerate();
        if com {
            unsafe { CoUninitialize() };
        }
        // A closed panel dropped the receiver: nothing to announce.
        if tx.send(apps).is_ok() {
            let (msg, source) = (crate::panel::WM_PANEL_ASYNC, WPARAM(crate::panel::ASYNC_APPS));
            let _ = unsafe { PostMessageW(Some(HWND(owner as _)), msg, source, LPARAM(0)) };
        }
    });
    rx
}

fn shell_path(id: &str) -> HSTRING {
    HSTRING::from(format!("shell:AppsFolder\\{id}"))
}

/// Opens the app (Store apps by AppUserModelID, Win32 ones by parsing name).
pub fn launch(id: &str) {
    unsafe {
        ShellExecuteW(None, w!("open"), &shell_path(id), None, None, SW_SHOWNORMAL);
    }
}

/// GDI icons carry straight alpha, premultiplied alpha, or none at all:
/// make them premultiplied, opaque when no pixel has alpha.
fn premultiply(bgra: &mut [u8]) {
    if bgra.chunks_exact(4).all(|p| p[3] == 0) {
        bgra.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        return;
    }
    // A color above its alpha can't be premultiplied: the image is straight.
    if bgra.chunks_exact(4).any(|p| p[..3].iter().any(|&c| c > p[3])) {
        for p in bgra.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

/// Copies a 32-bit bitmap out as top-down BGRA.
fn pixels(hbm: HBITMAP) -> Option<Icon> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(hbm.into(), size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as _)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || w != h {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, hbm, 0, h as u32, Some(bgra.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines != h {
            return None;
        }
        premultiply(&mut bgra);
        Some(Icon { size: w as u32, bgra })
    }
}

/// The app's 32 px icon. Needs COM on the calling thread.
pub fn icon(id: &str) -> Option<Icon> {
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&shell_path(id), None).ok()?;
        let hbm = factory.GetImage(SIZE { cx: ICON_PX, cy: ICON_PX }, SIIGBF_ICONONLY).ok()?;
        let icon = pixels(hbm);
        let _ = DeleteObject(hbm.into());
        icon
    }
}

/// The apps while the panel is open, with the icons loaded so far.
pub struct Catalog {
    apps: Vec<AppEntry>,
    /// By id; `None` when the shell gave no icon (not asked again).
    icons: HashMap<String, Option<Arc<Icon>>>,
}

impl Catalog {
    pub fn new(apps: Vec<AppEntry>) -> Self {
        Self { apps, icons: HashMap::new() }
    }

    /// The best `max` matches for `query`, loading the icons they still miss.
    pub fn search(&mut self, query: &str, max: usize) -> Vec<AppEntry> {
        let mut results: Vec<AppEntry> = crate::search::filter(&self.apps, query, max).into_iter().cloned().collect();
        for app in &mut results {
            let icon = self.icons.entry(app.id.clone()).or_insert_with(|| icon(&app.id).map(Arc::new));
            app.icon = icon.clone();
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiply_straight_alpha() {
        // Half-transparent white and a fully transparent pixel with color.
        let mut px = [255, 255, 255, 128, 200, 100, 50, 0];
        premultiply(&mut px);
        assert_eq!(px, [128, 128, 128, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn premultiplied_alpha_is_kept() {
        let mut px = [100, 50, 10, 128, 0, 0, 0, 0];
        premultiply(&mut px);
        assert_eq!(px, [100, 50, 10, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn no_alpha_is_opaque() {
        let mut px = [10, 20, 30, 0, 40, 50, 60, 0];
        premultiply(&mut px);
        assert_eq!(px, [10, 20, 30, 255, 40, 50, 60, 255]);
    }
}
