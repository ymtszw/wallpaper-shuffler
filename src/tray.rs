use crate::config::AppConfig;
use crate::logger::LogLevel;
use crate::startup;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    HICON, HMENU, ICONINFO, IDOK, MB_ICONINFORMATION, MB_OKCANCEL, MF_CHECKED, MF_DISABLED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MessageBoxW, SW_SHOWNORMAL,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TrackPopupMenu, WM_USER,
};
use windows::core::{PCWSTR, w};

pub const WM_TRAYICON: u32 = WM_USER + 101;

pub const IDM_SHUFFLE_NOW: usize = 2001;
pub const IDM_TOGGLE_PAUSE: usize = 2002;
pub const IDM_SETTINGS_INFO: usize = 2003;
pub const IDM_OPEN_CONFIG: usize = 2004;
pub const IDM_RELOAD_CONFIG: usize = 2005;
pub const IDM_OPEN_WALLPAPERS: usize = 2006;
pub const IDM_OPEN_LOGS: usize = 2007;
pub const IDM_TOGGLE_STARTUP: usize = 2008;

pub const IDM_LOG_DEBUG: usize = 2010;
pub const IDM_LOG_INFO: usize = 2011;
pub const IDM_LOG_WARN: usize = 2012;
pub const IDM_LOG_ERROR: usize = 2013;
pub const IDM_LOG_OFF: usize = 2014;

pub const IDM_STATUS_FOLDER: usize = 2020;
pub const IDM_STATUS_THRESHOLDS: usize = 2021;
pub const IDM_STATUS_COOLDOWN: usize = 2022;

pub const IDM_EXIT: usize = 2099;

fn to_u16_vec(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn path_to_u16_vec(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Renders a 32x32 RGBA icon depicting a museum-framed artwork.
/// If `source` image is provided, it is scaled to fit within the frame.
/// If `None`, an elegant miniature landscape with mountains and sun is rendered.
pub fn render_framed_thumbnail(source: Option<&image::RgbaImage>) -> image::RgbaImage {
    let mut canvas = image::RgbaImage::new(32, 32);

    // 1. Drop shadow under bottom & right edge of frame
    for x in 3..=30 {
        canvas.put_pixel(x, 27, image::Rgba([0, 0, 0, 80]));
    }
    for x in 5..=29 {
        canvas.put_pixel(x, 28, image::Rgba([0, 0, 0, 40]));
    }
    for y in 5..=26 {
        canvas.put_pixel(31, y, image::Rgba([0, 0, 0, 70]));
    }

    // 2. Outer picture frame (x: 1..=30, y: 3..=26 -> 30x24 physical frame)
    for y in 3..=26 {
        for x in 1..=30 {
            let dist_left = x - 1;
            let dist_right = 30 - x;
            let dist_top = y - 3;
            let dist_bottom = 26 - y;
            let border_dist = dist_left.min(dist_right).min(dist_top).min(dist_bottom);

            match border_dist {
                0 => {
                    // Outer dark walnut frame rim
                    canvas.put_pixel(x, y, image::Rgba([45, 26, 12, 255]));
                }
                1 => {
                    // Mid wood tone with upper-left light source accent
                    if dist_top == 1 || dist_left == 1 {
                        canvas.put_pixel(x, y, image::Rgba([180, 130, 60, 255])); // Warm golden amber
                    } else {
                        canvas.put_pixel(x, y, image::Rgba([100, 65, 30, 255])); // Deep walnut shadow
                    }
                }
                2 => {
                    // Inner gold filet inlay / mat bevel
                    if dist_top == 2 || dist_left == 2 {
                        canvas.put_pixel(x, y, image::Rgba([225, 195, 90, 255])); // Gold luster highlight
                    } else {
                        canvas.put_pixel(x, y, image::Rgba([60, 38, 18, 255])); // Inner frame recess
                    }
                }
                _ => {
                    // Interior picture area (x in 4..=27, y in 6..=23 -> 24x18 pixels)
                }
            }
        }
    }

    // 3. Inner picture area (Width: 24, Height: 18)
    let inner_w = 24u32;
    let inner_h = 18u32;

    if let Some(src) = source {
        let resized =
            image::imageops::resize(src, inner_w, inner_h, image::imageops::FilterType::Triangle);

        for iy in 0..inner_h {
            for ix in 0..inner_w {
                let p = resized.get_pixel(ix, iy);
                let cx = 4 + ix;
                let cy = 6 + iy;

                // Apply subtle recessed shadow along top and left inside frame
                let (r, g, b) = if iy == 0 || ix == 0 {
                    (
                        (p[0] as f32 * 0.72) as u8,
                        (p[1] as f32 * 0.72) as u8,
                        (p[2] as f32 * 0.72) as u8,
                    )
                } else {
                    (p[0], p[1], p[2])
                };

                canvas.put_pixel(cx, cy, image::Rgba([r, g, b, 255]));
            }
        }
    } else {
        // Default miniature painting: gradient sky, mountains, and sun
        for iy in 0..inner_h {
            for ix in 0..inner_w {
                let cx = 4 + ix;
                let cy = 6 + iy;

                let sky_r = (140.0 + (iy as f32 / inner_h as f32) * 80.0) as u8;
                let sky_g = (175.0 + (iy as f32 / inner_h as f32) * 40.0) as u8;
                let sky_b = (235.0 - (iy as f32 / inner_h as f32) * 30.0) as u8;
                let mut pix = image::Rgba([sky_r, sky_g, sky_b, 255]);

                // Tiny golden sun at (ix=16..=18, iy=3..=5)
                let dx = ix as i32 - 17;
                let dy = iy as i32 - 4;
                if dx * dx + dy * dy <= 4 {
                    pix = image::Rgba([255, 220, 80, 255]);
                }

                // Mountain peaks in lower half
                let m1_h = 9 + (ix as i32 - 7).abs() * 2;
                if iy as i32 >= m1_h {
                    pix = image::Rgba([80, 95, 130, 255]);
                }

                let m2_h = 11 + (ix as i32 - 16).abs() * 2;
                if iy as i32 >= m2_h {
                    pix = image::Rgba([60, 75, 110, 255]);
                }

                if iy == 0 || ix == 0 {
                    pix[0] = (pix[0] as f32 * 0.72) as u8;
                    pix[1] = (pix[1] as f32 * 0.72) as u8;
                    pix[2] = (pix[2] as f32 * 0.72) as u8;
                }

                canvas.put_pixel(cx, cy, pix);
            }
        }
    }

    canvas
}

/// Converts an `image::RgbaImage` into a Win32 `HICON` with 32-bit ARGB alpha transparency.
pub fn rgba_to_hicon(img: &image::RgbaImage) -> Result<HICON, String> {
    let (width, height) = img.dimensions();
    let mut bgra_data = Vec::with_capacity((width * height * 4) as usize);

    for pixel in img.pixels() {
        let [r, g, b, a] = pixel.0;
        let alpha_f = a as f32 / 255.0;
        let pr = (r as f32 * alpha_f).round() as u8;
        let pg = (g as f32 * alpha_f).round() as u8;
        let pb = (b as f32 * alpha_f).round() as u8;
        bgra_data.push(pb);
        bgra_data.push(pg);
        bgra_data.push(pr);
        bgra_data.push(a);
    }

    unsafe {
        let hbm_color = CreateBitmap(
            width as i32,
            height as i32,
            1,
            32,
            Some(bgra_data.as_ptr() as *const _),
        );
        if hbm_color.is_invalid() {
            return Err("Failed to create color bitmap for icon".to_string());
        }

        let mask_bytes = vec![0u8; (width.div_ceil(16) * 2 * height) as usize];
        let hbm_mask = CreateBitmap(
            width as i32,
            height as i32,
            1,
            1,
            Some(mask_bytes.as_ptr() as *const _),
        );
        if hbm_mask.is_invalid() {
            let _ = DeleteObject(hbm_color.into());
            return Err("Failed to create mask bitmap for icon".to_string());
        }

        let icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };

        let hicon = CreateIconIndirect(&icon_info)
            .map_err(|e| format!("CreateIconIndirect failed: {:?}", e))?;

        let _ = DeleteObject(hbm_color.into());
        let _ = DeleteObject(hbm_mask.into());

        Ok(hicon)
    }
}

pub struct TrayManager {
    pub hwnd: HWND,
    nid: NOTIFYICONDATAW,
    current_hicon: Option<HICON>,
}

unsafe impl Send for TrayManager {}
unsafe impl Sync for TrayManager {}

impl TrayManager {
    pub fn new(hwnd: HWND, initial_image: Option<&image::RgbaImage>) -> Self {
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

        let framed_img = render_framed_thumbnail(initial_image);
        let hicon_opt = rgba_to_hicon(&framed_img).ok();

        if let Some(hicon) = hicon_opt {
            nid.uFlags |= NIF_ICON;
            nid.hIcon = hicon;
        }

        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }

        Self {
            hwnd,
            nid,
            current_hicon: hicon_opt,
        }
    }

    pub fn update_icon(&mut self, image: Option<&image::RgbaImage>) {
        let framed_img = render_framed_thumbnail(image);
        if let Ok(new_hicon) = rgba_to_hicon(&framed_img) {
            self.nid.uFlags = NIF_ICON;
            self.nid.hIcon = new_hicon;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
            }
            if let Some(old) = self.current_hicon.replace(new_hicon) {
                unsafe {
                    let _ = DestroyIcon(old);
                }
            }
        }
    }

    pub fn update_icon_from_file(&mut self, path: &Path) {
        if let Ok(img) = image::open(path) {
            self.update_icon(Some(&img.to_rgba8()));
        }
    }

    pub fn update_tooltip(&mut self, tip: &str) {
        self.nid.uFlags = NIF_TIP;
        self.nid.szTip.fill(0);
        for (i, c) in tip.encode_utf16().take(127).enumerate() {
            self.nid.szTip[i] = c;
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
            if let Some(hicon) = self.current_hicon {
                let _ = DestroyIcon(hicon);
            }
        }
    }
}

pub static GLOBAL_TRAY: Mutex<Option<TrayManager>> = Mutex::new(None);

pub fn init_tray(hwnd: HWND, initial_image: Option<&image::RgbaImage>) {
    let tray = TrayManager::new(hwnd, initial_image);
    let mut guard = GLOBAL_TRAY.lock().unwrap();
    *guard = Some(tray);
}

pub fn update_tray_icon(image: Option<&image::RgbaImage>) {
    if let Some(tray) = GLOBAL_TRAY.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        tray.update_icon(image);
    }
}

pub fn update_tray_icon_from_file(path: &Path) {
    if let Some(tray) = GLOBAL_TRAY.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        tray.update_icon_from_file(path);
    }
}

pub fn update_tray_tooltip(tip: &str) {
    if let Some(tray) = GLOBAL_TRAY.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        tray.update_tooltip(tip);
    }
}

/// Displays the native Win32 tray context menu with full configuration view and controls.
pub fn show_tray_context_menu(
    hwnd: HWND,
    cfg: &AppConfig,
    is_paused: bool,
    active_log_level: LogLevel,
) {
    unsafe {
        let menu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        // 1. Primary Action: Shuffle Now
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            IDM_SHUFFLE_NOW,
            w!("Shuffle Wallpaper Now"),
        );

        // 2. Pause / Resume Toggle
        let pause_label = if is_paused {
            to_u16_vec("[✓] Paused (Click to Resume)")
        } else {
            to_u16_vec("Pause Auto-Shuffle")
        };
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            IDM_TOGGLE_PAUSE,
            PCWSTR(pause_label.as_ptr()),
        );

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        // 3. Current Settings Overview (Submenu & Dialog)
        let settings_submenu = match CreatePopupMenu() {
            Ok(sm) => {
                let folder_str = cfg
                    .wallpaper_dirs
                    .first()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "Not configured".to_string());
                let folder_label = to_u16_vec(&format!("Folder: {}", folder_str));
                let _ = AppendMenuW(
                    sm,
                    MF_STRING,
                    IDM_STATUS_FOLDER,
                    PCWSTR(folder_label.as_ptr()),
                );

                let thresh_label = to_u16_vec(&format!(
                    "Trigger: Covered < {:.1}% -> Exposed >= {:.1}%",
                    cfg.obscured_threshold_percent, cfg.exposed_threshold_percent
                ));
                let _ = AppendMenuW(
                    sm,
                    MF_GRAYED | MF_DISABLED,
                    IDM_STATUS_THRESHOLDS,
                    PCWSTR(thresh_label.as_ptr()),
                );

                let cooldown_label = to_u16_vec(&format!(
                    "Cooldown: {}s (Debounce: {}ms)",
                    cfg.cooldown_seconds, cfg.debounce_millis
                ));
                let _ = AppendMenuW(
                    sm,
                    MF_GRAYED | MF_DISABLED,
                    IDM_STATUS_COOLDOWN,
                    PCWSTR(cooldown_label.as_ptr()),
                );

                let _ = AppendMenuW(sm, MF_SEPARATOR, 0, PCWSTR::null());
                let _ = AppendMenuW(
                    sm,
                    MF_STRING,
                    IDM_SETTINGS_INFO,
                    w!("View Full Settings Details..."),
                );

                let _ = AppendMenuW(
                    menu,
                    MF_POPUP,
                    sm.0 as usize,
                    w!("Current Settings & Status"),
                );
                Some(sm)
            }
            Err(_) => None,
        };

        // 4. Configuration Editing & Folders
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            IDM_OPEN_CONFIG,
            w!("Edit Settings (config.json)"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            IDM_RELOAD_CONFIG,
            w!("Reload Configuration"),
        );
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            IDM_OPEN_WALLPAPERS,
            w!("Open Wallpapers Folder"),
        );
        let _ = AppendMenuW(menu, MF_STRING, IDM_OPEN_LOGS, w!("Open Logs Folder"));

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        // 5. Windows Startup Toggle
        let startup_enabled = startup::is_startup_enabled();
        let startup_flags = if startup_enabled {
            MF_CHECKED | MF_STRING
        } else {
            MF_UNCHECKED | MF_STRING
        };
        let _ = AppendMenuW(
            menu,
            startup_flags,
            IDM_TOGGLE_STARTUP,
            w!("Start with Windows"),
        );

        // 6. Log Level Submenu
        let log_submenu = match CreatePopupMenu() {
            Ok(lsm) => {
                let add_level_item = |sub: HMENU, id: usize, name: &[u16], level: LogLevel| {
                    let flag = if active_log_level == level {
                        MF_CHECKED | MF_STRING
                    } else {
                        MF_UNCHECKED | MF_STRING
                    };
                    let _ = AppendMenuW(sub, flag, id, PCWSTR(name.as_ptr()));
                };

                add_level_item(lsm, IDM_LOG_DEBUG, &to_u16_vec("Debug"), LogLevel::Debug);
                add_level_item(lsm, IDM_LOG_INFO, &to_u16_vec("Info"), LogLevel::Info);
                add_level_item(lsm, IDM_LOG_WARN, &to_u16_vec("Warn"), LogLevel::Warn);
                add_level_item(lsm, IDM_LOG_ERROR, &to_u16_vec("Error"), LogLevel::Error);
                add_level_item(lsm, IDM_LOG_OFF, &to_u16_vec("Off"), LogLevel::Off);

                let _ = AppendMenuW(menu, MF_POPUP, lsm.0 as usize, w!("Log Level"));
                Some(lsm)
            }
            Err(_) => None,
        };

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, IDM_EXIT, w!("Exit"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);

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

        if let Some(sm) = settings_submenu {
            let _ = DestroyMenu(sm);
        }
        if let Some(lsm) = log_submenu {
            let _ = DestroyMenu(lsm);
        }
        let _ = DestroyMenu(menu);
    }
}

/// Opens a path in Windows Explorer or default text editor via `ShellExecuteW`.
pub fn open_path_with_shell(hwnd: HWND, path: &Path) {
    let wide_path = path_to_u16_vec(path);
    unsafe {
        let _ = ShellExecuteW(
            Some(hwnd),
            w!("open"),
            PCWSTR(wide_path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Displays an informative Win32 MessageBox with current settings and an option to open config.json.
pub fn show_settings_dialog(hwnd: HWND, cfg: &AppConfig, config_path: &Path) {
    let folders = cfg
        .wallpaper_dirs
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    let startup_state = if startup::is_startup_enabled() {
        "Enabled (有効)"
    } else {
        "Disabled (無効)"
    };

    let msg = format!(
        "=== Wallpaper Shuffler Configuration ===\n\n\
        Wallpaper Directories:\n{}\n\n\
        Desktop Exposure Thresholds:\n\
          - Hidden:   < {:.1}%\n\
          - Revealed: >= {:.1}%\n\n\
        Timing:\n\
          - Cooldown: {} seconds\n\
          - Debounce: {} ms\n\n\
        History limit: {} images\n\
        Log level:     {}\n\
        Windows Startup: {}\n\n\
        Configuration file: {}\n\n\
        Click OK to open 'config.json' in Notepad to edit settings.\n\
        (Changes can be applied immediately using 'Reload Configuration' in tray menu)",
        folders,
        cfg.obscured_threshold_percent,
        cfg.exposed_threshold_percent,
        cfg.cooldown_seconds,
        cfg.debounce_millis,
        cfg.history_count,
        cfg.log_level,
        startup_state,
        config_path.display()
    );

    let title_wide = to_u16_vec("Wallpaper Shuffler - Settings");
    let msg_wide = to_u16_vec(&msg);

    unsafe {
        let res = MessageBoxW(
            Some(hwnd),
            PCWSTR(msg_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OKCANCEL | MB_ICONINFORMATION,
        );
        if res == IDOK {
            open_path_with_shell(hwnd, config_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_framed_thumbnail_dimensions() {
        // Test default miniature
        let default_thumb = render_framed_thumbnail(None);
        assert_eq!(default_thumb.dimensions(), (32, 32));
        // Verify frame top-left pixel is painted
        let outer_frame = default_thumb.get_pixel(1, 3);
        assert_eq!(outer_frame[3], 255); // Non-transparent

        // Test with a sample source image
        let sample_source = image::RgbaImage::new(100, 100);
        let custom_thumb = render_framed_thumbnail(Some(&sample_source));
        assert_eq!(custom_thumb.dimensions(), (32, 32));
    }

    #[test]
    fn test_rgba_to_hicon_creation_and_destruction() {
        let thumb = render_framed_thumbnail(None);
        let hicon_res = rgba_to_hicon(&thumb);
        assert!(hicon_res.is_ok(), "HICON creation should succeed");
        let hicon = hicon_res.unwrap();
        unsafe {
            let res = DestroyIcon(hicon);
            assert!(res.is_ok(), "DestroyIcon should succeed");
        }
    }
}
