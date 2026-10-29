use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, MF_SEPARATOR, MF_STRING,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TrackPopupMenu, WM_USER,
};
use windows::core::w;

pub const WM_TRAYICON: u32 = WM_USER + 101;
pub const IDM_SHUFFLE_NOW: usize = 2001;
pub const IDM_PAUSE: usize = 2002;
pub const IDM_OPEN_FOLDER: usize = 2003;
pub const IDM_EXIT: usize = 2004;

pub fn show_tray_context_menu(hwnd: HWND) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let _ = AppendMenuW(
            menu,
            MF_STRING,
            IDM_SHUFFLE_NOW,
            w!("Shuffle Wallpaper Now"),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

        // Required by Win32 before TrackPopupMenu to dismiss correctly
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
    }
}

pub struct TrayManager {
    pub hwnd: HWND,
    nid: NOTIFYICONDATAW,
}

impl TrayManager {
    pub fn new(hwnd: HWND) -> Self {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAYICON,
            ..Default::default()
        };

        let tip = "Wallpaper Shuffler";
        for (i, c) in tip.encode_utf16().take(127).enumerate() {
            nid.szTip[i] = c;
        }

        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }

        Self { hwnd, nid }
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
        }
    }
}
