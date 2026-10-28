use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{
    NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::WM_USER;

pub const WM_TRAYICON: u32 = WM_USER + 101;
pub const IDM_SHUFFLE_NOW: usize = 2001;
pub const IDM_PAUSE: usize = 2002;
pub const IDM_OPEN_FOLDER: usize = 2003;
pub const IDM_EXIT: usize = 2004;

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
