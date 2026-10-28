use std::path::Path;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
};

pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    let path_str = path.to_str().ok_or("Invalid UTF-8 path")?;
    let mut wide: Vec<u16> = path_str.encode_utf16().collect();
    wide.push(0);

    unsafe {
        let ret = SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(wide.as_ptr() as *mut _),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        );
        if ret.is_err() {
            return Err("Failed to set wallpaper via SystemParametersInfoW".to_string());
        }
    }
    Ok(())
}

pub struct ComGuard;

impl ComGuard {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            Ok(Self)
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
