use std::path::Path;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW,
};
use windows::core::{HSTRING, PCWSTR};

pub fn set_wallpaper(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Wallpaper file does not exist: {}", path.display()));
    }

    let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = abs_path.to_str().ok_or("Invalid UTF-8 path")?;
    // Strip Windows extended path prefix "\\?\"
    let clean_str = path_str.strip_prefix(r"\\?\").unwrap_or(path_str);

    // 1. Try modern COM IDesktopWallpaper interface first (no broadcast stutter, smooth Explorer update)
    if set_wallpaper_com(clean_str).is_ok() {
        return Ok(());
    }

    // 2. Fallback to SystemParametersInfoW
    set_wallpaper_fallback(clean_str)
}

fn set_wallpaper_com(clean_str: &str) -> windows::core::Result<()> {
    unsafe {
        let _com = ComGuard::new()?;
        let wallpaper: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)?;
        let hstring = HSTRING::from(clean_str);
        wallpaper.SetWallpaper(PCWSTR::null(), &hstring)?;
        Ok(())
    }
}

fn set_wallpaper_fallback(clean_str: &str) -> Result<(), String> {
    let mut wide: Vec<u16> = clean_str.encode_utf16().collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_wallpaper_nonexistent_file() {
        let nonexistent = Path::new("nonexistent_wallpaper_file_xyz123.jpg");
        let res = set_wallpaper(nonexistent);
        assert!(res.is_err());
    }
}
