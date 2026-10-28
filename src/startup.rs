use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APP_REG_NAME: PCWSTR = w!("WallpaperShuffler");

fn str_to_u16_vec(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

pub fn is_startup_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, Some(0), KEY_READ, &mut hkey)
            != ERROR_SUCCESS
        {
            return false;
        }

        let mut data_len = 0u32;
        let res = RegQueryValueExW(hkey, APP_REG_NAME, None, None, None, Some(&mut data_len));
        let _ = RegCloseKey(hkey);
        res == ERROR_SUCCESS
    }
}

pub fn register_startup(exe_path: &OsStr) -> Result<(), String> {
    unsafe {
        let mut hkey = HKEY::default();
        let open_res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut hkey,
        );
        if open_res != ERROR_SUCCESS {
            return Err(format!("Failed to open registry key: {:?}", open_res));
        }

        let wide_path = str_to_u16_vec(exe_path);
        let byte_slice = std::slice::from_raw_parts(
            wide_path.as_ptr() as *const u8,
            wide_path.len() * std::mem::size_of::<u16>(),
        );

        let set_res = RegSetValueExW(hkey, APP_REG_NAME, Some(0), REG_SZ, Some(byte_slice));
        let _ = RegCloseKey(hkey);

        if set_res != ERROR_SUCCESS {
            return Err(format!("Failed to set registry value: {:?}", set_res));
        }

        Ok(())
    }
}

pub fn unregister_startup() -> Result<(), String> {
    unsafe {
        let mut hkey = HKEY::default();
        let open_res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut hkey,
        );
        if open_res != ERROR_SUCCESS {
            return Err(format!("Failed to open registry key: {:?}", open_res));
        }

        let del_res = RegDeleteValueW(hkey, APP_REG_NAME);
        let _ = RegCloseKey(hkey);

        if del_res != ERROR_SUCCESS && del_res != ERROR_FILE_NOT_FOUND {
            return Err(format!("Failed to delete registry value: {:?}", del_res));
        }

        Ok(())
    }
}
