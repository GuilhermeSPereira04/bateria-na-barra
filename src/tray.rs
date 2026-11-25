//! Notification-area icon and its menu.
use windows::Win32::Foundation::{HWND, POINT, WPARAM, LPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// Callback message for the tray icon (lParam: the mouse message).
pub const WM_TRAY: u32 = WM_APP + 3;

pub const CMD_REFRESH: u32 = 1;
pub const CMD_STARTUP: u32 = 2;
pub const CMD_QUIT: u32 = 3;
pub const CMD_PANEL: u32 = 4;

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    tip: String,
}

fn fill_tip(dst: &mut [u16; 128], tip: &str) {
    let wide: Vec<u16> = tip.encode_utf16().take(127).collect();
    dst[..wide.len()].copy_from_slice(&wide);
    dst[wide.len()] = 0;
}

impl Tray {
    pub fn new(hwnd: HWND, icon: HICON) -> Self {
        let tray = Self { hwnd, icon, tip: "battery-bar".into() };
        tray.notify(NIM_ADD);
        tray
    }

    fn notify(&self, op: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE) {
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        fill_tip(&mut nid.szTip, &self.tip);
        unsafe {
            let _ = Shell_NotifyIconW(op, &nid);
        }
    }

    pub fn set_tip(&mut self, tip: &str) {
        if self.tip != tip {
            self.tip = tip.to_string();
            self.notify(NIM_MODIFY);
        }
    }

    pub fn set_icon(&mut self, icon: HICON) {
        let old = std::mem::replace(&mut self.icon, icon);
        self.notify(NIM_MODIFY);
        unsafe {
            let _ = DestroyIcon(old);
        }
    }

    /// After Explorer restarts (`TaskbarCreated`) the icon must be added again.
    pub fn readd(&self) {
        self.notify(NIM_ADD);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.notify(NIM_DELETE);
        unsafe {
            let _ = DestroyIcon(self.icon);
        }
    }
}

/// Shows the menu at the cursor; returns the chosen `CMD_*` (0 if dismissed).
pub fn show_menu(hwnd: HWND, startup_on: bool) -> u32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        let item = |id: u32, text: &str, checked: bool| {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(wide.as_ptr()));
        };
        item(CMD_PANEL, "Abrir painel", false);
        item(CMD_REFRESH, "Atualizar agora", false);
        item(CMD_STARTUP, "Iniciar com o Windows", startup_on);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(CMD_QUIT, "Sair", false);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Required so the menu closes when clicking elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd.0 as u32
    }
}
