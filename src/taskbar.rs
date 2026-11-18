use crate::tray::WM_TRAYICON;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    COLORREF, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HGDIOBJ,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PtInRect, ReleaseDC, ScreenToClient,
    SelectObject,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
    UIA_AutomationIdPropertyId,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowExW, FindWindowW,
    GW_HWNDPREV, GWL_EXSTYLE, GetClassNameW, GetClientRect, GetCursorPos, GetWindow,
    GetWindowLongW, GetWindowRect, HCURSOR, HICON, HWND_TOPMOST, IDC_ARROW, IsWindow,
    IsWindowVisible, LoadCursorW, MA_NOACTIVATE, MoveWindow, PostMessageW, RegisterClassExW,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_LBUTTONUP, WM_MOUSEACTIVATE,
    WM_MOUSEMOVE, WM_RBUTTONUP, WM_SIZE, WM_USER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_LAYOUTRTL,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, BSTR, w};

pub const WM_TASKBAR_HOVER_CHANGED: u32 = WM_USER + 102;
const WM_MOUSELEAVE: u32 = 0x02A3;
const PREVIEW_CACHE_W: u32 = 160;
const PREVIEW_CACHE_H: u32 = 90;
const POPUP_LOGICAL_WIDTH: f32 = 600.0;
const POPUP_MAX_CACHE_W: u32 = 1200;

static TASKBAR_HOVER: AtomicBool = AtomicBool::new(false);
static TASKBAR_NOTIFY_HWND: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_NEEDS_REPAINT: AtomicBool = AtomicBool::new(false);
static CONTEXT_MENU_OPEN: AtomicBool = AtomicBool::new(false);
static TASKBAR_ACTIVE: AtomicBool = AtomicBool::new(false);
static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

pub static GLOBAL_TASKBAR: Mutex<Option<TaskbarWidget>> = Mutex::new(None);

fn is_light_taskbar_theme() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            Some(0),
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return false;
        }
        let mut val = 0u32;
        let mut len = std::mem::size_of::<u32>() as u32;
        let res = RegQueryValueExW(
            hkey,
            w!("SystemUsesLightTheme"),
            None,
            None,
            Some(&mut val as *mut u32 as *mut u8),
            Some(&mut len),
        );
        let _ = RegCloseKey(hkey);
        res == ERROR_SUCCESS && val != 0
    }
}

/// Computes the signed distance from a point `(px, py)` to a rounded rectangle
/// `[x, x + w] x [y, y + h]` with corner radius `r`. Negative values are inside.
fn sd_round_rect(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let half_w = (w * 0.5 - r).max(0.0);
    let half_h = (h * 0.5 - r).max(0.0);
    let dx = (px - cx).abs() - half_w;
    let dy = (py - cy).abs() - half_h;
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let inside = dx.max(dy).min(0.0);
    outside + inside - r
}

fn alpha_blend_pixel(dst: &mut [u8; 4], src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    if sa >= 1.0 {
        *dst = src;
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        *dst = [0, 0, 0, 0];
        return;
    }
    for c in 0..3 {
        let num = src[c] as f32 * sa + dst[c] as f32 * da * (1.0 - sa);
        dst[c] = (num / out_a).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Computes the taskbar mini UI strip dimensions from taskbar height and DPI scale.
pub fn compute_taskbar_strip_size(taskbar_height: u32, dpi_scale: f32) -> (u32, u32) {
    let h = taskbar_height.max(24);
    let inset_y = (4.0 * dpi_scale).round();
    let pad = (4.0 * dpi_scale).round();
    let inset_x = (2.0 * dpi_scale).round();

    let cap_h = (h as f32 - inset_y * 2.0).max(16.0);
    let thumb_h = (cap_h - pad * 2.0).max(10.0);
    let thumb_w = (thumb_h * (16.0 / 9.0)).round();
    let total_w = (thumb_w + pad * 2.0 + inset_x * 2.0).ceil() as u32;
    (total_w.max(24), h)
}

fn sample_default_landscape(nx: f32, ny: f32) -> [u8; 4] {
    let mut r = 140.0 + ny * 80.0;
    let mut g = 175.0 + ny * 40.0;
    let mut b = 235.0 - ny * 30.0;

    // Golden sun
    let sx = nx - 0.72;
    let sy = (ny - 0.25) * (9.0 / 16.0);
    if sx * sx + sy * sy <= 0.011 {
        r = 255.0;
        g = 220.0;
        b = 80.0;
    }

    // Back mountain
    let m1 = 0.48 + (nx - 0.30).abs() * 0.9;
    if ny >= m1 {
        r = 80.0;
        g = 95.0;
        b = 130.0;
    }

    // Front mountain
    let m2 = 0.56 + (nx - 0.68).abs() * 0.95;
    if ny >= m2 {
        r = 55.0;
        g = 70.0;
        b = 105.0;
    }

    [
        r.round().clamp(0.0, 255.0) as u8,
        g.round().clamp(0.0, 255.0) as u8,
        b.round().clamp(0.0, 255.0) as u8,
        255,
    ]
}

/// Renders the taskbar mini UI pill containing the currently applied wallpaper thumbnail.
pub fn render_taskbar_thumbnail(
    source: Option<&image::RgbaImage>,
    width: u32,
    height: u32,
    dpi_scale: f32,
    hovered: bool,
    light_theme: bool,
    is_paused: bool,
) -> image::RgbaImage {
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 1]));

    let w = width as f32;
    let h = height as f32;
    let inset_x = (2.0 * dpi_scale).round();
    let inset_y = (4.0 * dpi_scale).round();
    let cap_w = (w - inset_x * 2.0).max(1.0);
    let cap_h = (h - inset_y * 2.0).max(1.0);
    let cap_r = (7.0 * dpi_scale).min(cap_h * 0.45);
    let border_w = dpi_scale.max(1.0);

    let bg_color = if light_theme {
        if hovered {
            [255, 255, 255, 245]
        } else {
            [243, 244, 248, 220]
        }
    } else if hovered {
        [42, 48, 64, 240]
    } else {
        [24, 28, 38, 210]
    };

    let border_color = if light_theme {
        if hovered {
            [120, 135, 160, 220]
        } else {
            [180, 190, 205, 175]
        }
    } else if hovered {
        [130, 150, 185, 220]
    } else {
        [75, 86, 112, 150]
    };

    // 1. Pill background + anti-aliased border
    for y in 0..height {
        for x in 0..width {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = sd_round_rect(px, py, inset_x, inset_y, cap_w, cap_h, cap_r);
            if d < 0.75 {
                let cov = (0.75 - d).clamp(0.0, 1.0);
                let mut bg = bg_color;
                bg[3] = (bg[3] as f32 * cov).round() as u8;
                alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, bg);

                // Border ring around the outer edge
                let ring_d = d.abs();
                if ring_d < border_w + 0.5 {
                    let ring_cov = ((border_w + 0.5 - ring_d) * cov).clamp(0.0, 1.0);
                    let mut bd = border_color;
                    bd[3] = (bd[3] as f32 * ring_cov).round() as u8;
                    alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, bd);
                }
            }
        }
    }

    // 2. Inner wallpaper thumbnail with rounded corners
    let pad = (4.0 * dpi_scale).round();
    let thumb_x = inset_x + pad;
    let thumb_y = inset_y + pad;
    let thumb_w = (cap_w - pad * 2.0).max(4.0);
    let thumb_h = (cap_h - pad * 2.0).max(4.0);
    let thumb_r = (4.0 * dpi_scale).min(thumb_h * 0.35);

    let thumb_w_u32 = thumb_w.round().max(1.0) as u32;
    let thumb_h_u32 = thumb_h.round().max(1.0) as u32;

    let resized_src = source.map(|src| {
        image::imageops::resize(
            src,
            thumb_w_u32,
            thumb_h_u32,
            image::imageops::FilterType::Triangle,
        )
    });

    let x_start = thumb_x.floor().max(0.0) as u32;
    let x_end = (thumb_x + thumb_w).ceil().min(w) as u32;
    let y_start = thumb_y.floor().max(0.0) as u32;
    let y_end = (thumb_y + thumb_h).ceil().min(h) as u32;

    let inner_border = if light_theme {
        [0, 0, 0, 55]
    } else {
        [255, 255, 255, 55]
    };

    for y in y_start..y_end {
        for x in x_start..x_end {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = sd_round_rect(px, py, thumb_x, thumb_y, thumb_w, thumb_h, thumb_r);
            if d < 0.75 {
                let cov = (0.75 - d).clamp(0.0, 1.0);
                let u = ((px - thumb_x) / thumb_w).clamp(0.0, 1.0);
                let v = ((py - thumb_y) / thumb_h).clamp(0.0, 1.0);

                let mut color = if let Some(ref img) = resized_src {
                    let sx = ((u * thumb_w_u32 as f32) as u32).min(thumb_w_u32 - 1);
                    let sy = ((v * thumb_h_u32 as f32) as u32).min(thumb_h_u32 - 1);
                    let p = img.get_pixel(sx, sy).0;
                    [p[0], p[1], p[2], 255]
                } else {
                    sample_default_landscape(u, v)
                };

                if is_paused {
                    // Slightly dim thumbnail when auto-shuffle is paused
                    color[0] = (color[0] as f32 * 0.75) as u8;
                    color[1] = (color[1] as f32 * 0.75) as u8;
                    color[2] = (color[2] as f32 * 0.75) as u8;
                }

                color[3] = (255.0 * cov).round() as u8;
                alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, color);

                // Subtle 1px inner frame highlight/outline
                if d > -1.0 && d < 0.5 {
                    let edge_cov = (cov * 0.85).clamp(0.0, 1.0);
                    let mut ib = inner_border;
                    ib[3] = (ib[3] as f32 * edge_cov).round() as u8;
                    alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, ib);
                }
            }
        }
    }

    // 3. Paused indicator badge in bottom-right corner of thumbnail
    if is_paused {
        let badge_r = (5.0 * dpi_scale).max(4.0);
        let badge_cx = thumb_x + thumb_w - badge_r - (2.0 * dpi_scale);
        let badge_cy = thumb_y + thumb_h - badge_r - (2.0 * dpi_scale);
        let bx0 = (badge_cx - badge_r - 1.0).floor().max(0.0) as u32;
        let bx1 = (badge_cx + badge_r + 1.0).ceil().min(w) as u32;
        let by0 = (badge_cy - badge_r - 1.0).floor().max(0.0) as u32;
        let by1 = (badge_cy + badge_r + 1.0).ceil().min(h) as u32;

        for y in by0..by1 {
            for x in bx0..bx1 {
                let dx = (x as f32 + 0.5) - badge_cx;
                let dy = (y as f32 + 0.5) - badge_cy;
                let dist = (dx * dx + dy * dy).sqrt() - badge_r;
                if dist < 0.75 {
                    let cov = (0.75 - dist).clamp(0.0, 1.0);
                    let bar_w = (1.1 * dpi_scale).max(1.0);
                    let bar_gap = (1.6 * dpi_scale).max(1.2);
                    let bar_h = badge_r * 0.5;
                    let in_bars = dy.abs() <= bar_h
                        && ((dx + bar_gap).abs() <= bar_w * 0.5
                            || (dx - bar_gap).abs() <= bar_w * 0.5);
                    let px_color = if in_bars {
                        [255, 255, 255, (255.0 * cov).round() as u8]
                    } else {
                        [217, 119, 6, (240.0 * cov).round() as u8]
                    };
                    alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, px_color);
                }
            }
        }
    }

    canvas
}

fn resize_popup_cache(img: &image::RgbaImage) -> image::RgbaImage {
    let w = img.width();
    let h = img.height();
    if w == 0 || h == 0 || w <= POPUP_MAX_CACHE_W {
        return img.clone();
    }
    let target_h = ((POPUP_MAX_CACHE_W as f32 * h as f32) / w as f32)
        .round()
        .max(1.0) as u32;
    image::imageops::resize(
        img,
        POPUP_MAX_CACHE_W,
        target_h,
        image::imageops::FilterType::Triangle,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupLayout {
    pub total_w: u32,
    pub total_h: u32,
    pub img_w: u32,
    pub img_h: u32,
    pub pad: u32,
    pub shadow: u32,
}

/// Computes the popup card and image dimensions (~600 logical px wide) clamped to monitor work area.
pub fn compute_popup_dimensions(
    source_dims: Option<(u32, u32)>,
    dpi_scale: f32,
    max_work_w: i32,
    max_work_h: i32,
) -> PopupLayout {
    let pad = (6.0 * dpi_scale).round().max(2.0) as u32;
    let shadow = (8.0 * dpi_scale).round().max(2.0) as u32;
    let chrome = (pad + shadow) * 2;

    let max_img_w = (max_work_w.max(200) as u32).saturating_sub(chrome).max(160);
    let img_w = ((POPUP_LOGICAL_WIDTH * dpi_scale).round() as u32).clamp(160, max_img_w);

    let max_img_h = ((max_work_h.max(240) as f32 * 0.65).round() as u32)
        .saturating_sub(chrome)
        .max(120);
    let raw_img_h = match source_dims {
        Some((sw, sh)) if sw > 0 && sh > 0 => {
            ((img_w as f32 * sh as f32) / sw as f32).round().max(90.0) as u32
        }
        _ => (img_w * 9 / 16).max(90),
    };
    let img_h = raw_img_h.min(max_img_h);

    PopupLayout {
        total_w: img_w + chrome,
        total_h: img_h + chrome,
        img_w,
        img_h,
        pad,
        shadow,
    }
}

/// Renders the hover preview popup card (soft shadow, rounded frame, and scaled wallpaper).
pub fn render_popup_preview(
    source: Option<&image::RgbaImage>,
    layout: PopupLayout,
    dpi_scale: f32,
    light_theme: bool,
) -> image::RgbaImage {
    let mut canvas =
        image::RgbaImage::from_pixel(layout.total_w, layout.total_h, image::Rgba([0, 0, 0, 0]));

    let shadow_f = layout.shadow as f32;
    let pad_f = layout.pad as f32;
    let card_x = shadow_f;
    let card_y = shadow_f;
    let card_w = (layout.img_w + layout.pad * 2) as f32;
    let card_h = (layout.img_h + layout.pad * 2) as f32;
    let card_r = (10.0 * dpi_scale).min(card_h * 0.25);
    let border_w = dpi_scale.max(1.0);

    let bg_color = if light_theme {
        [245, 246, 250, 248]
    } else {
        [28, 31, 40, 248]
    };
    let border_color = if light_theme {
        [0, 0, 0, 42]
    } else {
        [255, 255, 255, 48]
    };

    for y in 0..layout.total_h {
        for x in 0..layout.total_w {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = sd_round_rect(px, py, card_x, card_y, card_w, card_h, card_r);
            if d > 0.0 && d < shadow_f {
                let t = 1.0 - d / shadow_f;
                let a = (t * t * 90.0).round().clamp(0.0, 255.0) as u8;
                if a > 0 {
                    alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, [0, 0, 0, a]);
                }
            } else if d < 0.75 {
                let cov = (0.75 - d).clamp(0.0, 1.0);
                let mut bg = bg_color;
                bg[3] = (bg[3] as f32 * cov).round() as u8;
                alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, bg);

                let ring_d = d.abs();
                if ring_d < border_w + 0.5 {
                    let ring_cov = ((border_w + 0.5 - ring_d) * cov).clamp(0.0, 1.0);
                    let mut bd = border_color;
                    bd[3] = (bd[3] as f32 * ring_cov).round() as u8;
                    alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, bd);
                }
            }
        }
    }

    let img_x = shadow_f + pad_f;
    let img_y = shadow_f + pad_f;
    let img_w_f = layout.img_w as f32;
    let img_h_f = layout.img_h as f32;
    let img_r = (6.0 * dpi_scale).min(img_h_f * 0.2);

    let resized_src = source.map(|src| {
        image::imageops::resize(
            src,
            layout.img_w,
            layout.img_h,
            image::imageops::FilterType::Triangle,
        )
    });

    let x0 = (layout.shadow + layout.pad).min(layout.total_w);
    let y0 = (layout.shadow + layout.pad).min(layout.total_h);
    let x1 = (x0 + layout.img_w).min(layout.total_w);
    let y1 = (y0 + layout.img_h).min(layout.total_h);

    for y in y0..y1 {
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = sd_round_rect(px, py, img_x, img_y, img_w_f, img_h_f, img_r);
            if d < 0.75 {
                let cov = (0.75 - d).clamp(0.0, 1.0);
                let mut px_color = if let Some(ref img) = resized_src {
                    let sx = (x - x0).min(layout.img_w - 1);
                    let sy = (y - y0).min(layout.img_h - 1);
                    img.get_pixel(sx, sy).0
                } else {
                    let nx = (x - x0) as f32 / img_w_f;
                    let ny = (y - y0) as f32 / img_h_f;
                    sample_default_landscape(nx, ny)
                };
                px_color[3] = (px_color[3] as f32 * cov).round() as u8;
                alpha_blend_pixel(&mut canvas.get_pixel_mut(x, y).0, px_color);
            }
        }
    }

    canvas
}

/// Computes the screen-space `(left, top)` for the hover popup above (or below) the taskbar.
pub fn compute_popup_position(
    widget_rect: RECT,
    taskbar_rect: RECT,
    monitor_rect: RECT,
    popup_w: i32,
    popup_h: i32,
    shadow: i32,
    dpi_scale: f32,
) -> (i32, i32) {
    let margin = (8.0 * dpi_scale).round() as i32;
    let gap = (8.0 * dpi_scale).round() as i32 - shadow;

    let ideal_x = (widget_rect.left + widget_rect.right) / 2 - popup_w / 2;
    let min_x = monitor_rect.left + margin - shadow;
    let max_x = (monitor_rect.right - margin + shadow - popup_w).max(min_x);
    let x = ideal_x.clamp(min_x, max_x);

    let taskbar_at_top = taskbar_rect.top <= monitor_rect.top + 10;
    let raw_y = if taskbar_at_top {
        taskbar_rect.bottom + gap
    } else {
        taskbar_rect.top - popup_h - gap
    };
    let min_y = monitor_rect.top;
    let max_y = (monitor_rect.bottom - popup_h).max(min_y);
    let y = raw_y.clamp(min_y, max_y);

    (x, y)
}

unsafe extern "system" fn popup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn push_rgba_to_layered_window(hwnd: HWND, img: &image::RgbaImage, alpha: u8) {
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w <= 0 || h <= 0 {
        return;
    }

    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        if let Ok(hbitmap) =
            CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            let old_bmp = SelectObject(mem_dc, HGDIOBJ(hbitmap.0));
            let src_slice = img.as_raw();
            let dst_slice = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);

            // Convert straight RGBA -> premultiplied BGRA for AC_SRC_ALPHA
            for (src_chunk, dst_chunk) in src_slice
                .as_chunks::<4>()
                .0
                .iter()
                .zip(dst_slice.as_chunks_mut::<4>().0.iter_mut())
            {
                let a = src_chunk[3] as f32 / 255.0;
                dst_chunk[0] = (src_chunk[2] as f32 * a).round() as u8;
                dst_chunk[1] = (src_chunk[1] as f32 * a).round() as u8;
                dst_chunk[2] = (src_chunk[0] as f32 * a).round() as u8;
                dst_chunk[3] = src_chunk[3];
            }

            let pt_src = POINT { x: 0, y: 0 };
            let size_wnd = SIZE { cx: w, cy: h };
            let blend = BLENDFUNCTION {
                BlendOp: 0, // AC_SRC_OVER
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: 1, // AC_SRC_ALPHA
            };

            let _ = UpdateLayeredWindow(
                hwnd,
                Some(screen_dc),
                None,
                Some(&size_wnd),
                Some(mem_dc),
                Some(&pt_src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );

            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(hbitmap.into());
        }

        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);
    }
}

fn notify_hover_changed() {
    let notify_raw = TASKBAR_NOTIFY_HWND.load(Ordering::SeqCst);
    if notify_raw != 0 {
        let notify_hwnd = HWND(notify_raw as *mut std::ffi::c_void);
        unsafe {
            let _ = PostMessageW(
                Some(notify_hwnd),
                WM_TASKBAR_HOVER_CHANGED,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

unsafe extern "system" fn taskbar_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            if !TASKBAR_HOVER.swap(true, Ordering::SeqCst) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
                TASKBAR_NEEDS_REPAINT.store(true, Ordering::SeqCst);
                notify_hover_changed();
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            TASKBAR_HOVER.store(false, Ordering::SeqCst);
            TASKBAR_NEEDS_REPAINT.store(true, Ordering::SeqCst);
            notify_hover_changed();
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let notify_raw = TASKBAR_NOTIFY_HWND.load(Ordering::SeqCst);
            if notify_raw != 0 {
                let notify_hwnd = HWND(notify_raw as *mut std::ffi::c_void);
                unsafe {
                    let _ = PostMessageW(
                        Some(notify_hwnd),
                        WM_TRAYICON,
                        WPARAM(0),
                        LPARAM(WM_RBUTTONUP as isize),
                    );
                }
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub struct TaskbarWidget {
    hwnd: Option<HWND>,
    popup_hwnd: Option<HWND>,
    taskbar_hwnd: Option<HWND>,
    rebar_hwnd: Option<HWND>,
    notify_hwnd: HWND,
    preview_image: Option<image::RgbaImage>,
    popup_image: Option<image::RgbaImage>,
    cached_popup_render: Option<(u32, bool, PopupLayout, image::RgbaImage)>,
    popup_visible: bool,
    is_paused: bool,
    status_text: String,
    visible: bool,
    last_tray: RECT,
    last_bar: RECT,
    last_widgets: RECT,
    last_peers: Vec<RECT>,
    last_w: i32,
    last_dpi: u32,
    last_left: i32,
    width: i32,
    height: i32,
    uia: Option<IUIAutomation>,
    widgets_btn: Option<IUIAutomationElement>,
    last_uia_check: Option<Instant>,
    cached_widgets_rect: RECT,
}

unsafe impl Send for TaskbarWidget {}
unsafe impl Sync for TaskbarWidget {}

/// Resolves a horizontal screen `left` coordinate for a taskbar widget of `width`
/// starting from `edge` (tray inner edge) while avoiding all `obstacles` with `gap`.
fn resolve_non_overlapping_left(
    edge: i32,
    width: i32,
    gap: i32,
    rtl: bool,
    obstacles: &[RECT],
) -> i32 {
    if rtl {
        let mut left = edge + gap;
        for _ in 0..16 {
            let right = left + width;
            if let Some(hit) = obstacles
                .iter()
                .filter(|r| r.right > r.left && r.left < right && r.right > left)
                .max_by_key(|r| r.right)
            {
                left = hit.right + gap;
            } else {
                break;
            }
        }
        left
    } else {
        let mut right = edge - gap;
        let mut left = right - width;
        for _ in 0..16 {
            if let Some(hit) = obstacles
                .iter()
                .filter(|r| r.right > r.left && r.left < right && r.right > left)
                .min_by_key(|r| r.left)
            {
                right = hit.left - gap;
                left = right - width;
            } else {
                break;
            }
        }
        left
    }
}

/// Enumerates visible peer taskbar mini-UI windows sitting on the same taskbar band.
fn collect_peer_taskbar_rects(self_hwnd: HWND, tb_hwnd: HWND, bar: RECT) -> Vec<RECT> {
    struct EnumCtx {
        self_hwnd: HWND,
        tb_hwnd: HWND,
        bar: RECT,
        rects: Vec<RECT>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let ctx = &mut *(lparam.0 as *mut EnumCtx);
            if hwnd == ctx.self_hwnd || hwnd == ctx.tb_hwnd {
                return BOOL(1);
            }
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let required = WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0;
            if (ex_style & required) != required {
                return BOOL(1);
            }

            let mut class_buf = [0u16; 64];
            let len = GetClassNameW(hwnd, &mut class_buf) as usize;
            if len > 0 {
                let name = String::from_utf16_lossy(&class_buf[..len]);
                if name.eq_ignore_ascii_case("tooltips_class32")
                    || name == "Shell_TrayWnd"
                    || name == "Shell_SecondaryTrayWnd"
                    || name == "Progman"
                    || name == "WorkerW"
                    || name == "NotifyIconOverflowWindow"
                {
                    return BOOL(1);
                }
            }

            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_err() {
                return BOOL(1);
            }
            let w = r.right - r.left;
            let h = r.bottom - r.top;
            let bar_w = ctx.bar.right - ctx.bar.left;
            if w >= 16
                && w < bar_w / 2
                && h >= 20
                && r.top >= ctx.bar.top - 4
                && r.bottom <= ctx.bar.bottom + 4
            {
                ctx.rects.push(r);
            }
            BOOL(1)
        }
    }

    let mut ctx = EnumCtx {
        self_hwnd,
        tb_hwnd,
        bar,
        rects: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut EnumCtx as isize));
    }
    ctx.rects
}

impl TaskbarWidget {
    pub fn new(notify_hwnd: HWND, initial_image: Option<&image::RgbaImage>) -> Self {
        TASKBAR_NOTIFY_HWND.store(notify_hwnd.0 as isize, Ordering::SeqCst);
        let cached = initial_image.map(|img| {
            image::imageops::resize(
                img,
                PREVIEW_CACHE_W,
                PREVIEW_CACHE_H,
                image::imageops::FilterType::Triangle,
            )
        });
        let popup_cached = initial_image.map(resize_popup_cache);
        Self {
            hwnd: None,
            popup_hwnd: None,
            taskbar_hwnd: None,
            rebar_hwnd: None,
            notify_hwnd,
            preview_image: cached,
            popup_image: popup_cached,
            cached_popup_render: None,
            popup_visible: false,
            is_paused: false,
            status_text: "Ready".to_string(),
            visible: false,
            last_tray: RECT::default(),
            last_bar: RECT::default(),
            last_widgets: RECT::default(),
            last_peers: Vec::new(),
            last_w: -1,
            last_dpi: 0,
            last_left: -1,
            width: 0,
            height: 0,
            uia: None,
            widgets_btn: None,
            last_uia_check: None,
            cached_widgets_rect: RECT::default(),
        }
    }

    pub fn is_alive(&self) -> bool {
        unsafe {
            match (self.hwnd, self.taskbar_hwnd) {
                (Some(w), Some(tb)) => IsWindow(Some(w)).as_bool() && IsWindow(Some(tb)).as_bool(),
                _ => false,
            }
        }
    }

    pub fn create(&mut self) -> bool {
        self.destroy();
        unsafe {
            let Ok(tb) = FindWindowW(w!("Shell_TrayWnd"), None) else {
                return false;
            };
            if tb.0.is_null() {
                return false;
            }
            let Ok(tray) = FindWindowExW(Some(tb), None, w!("TrayNotifyWnd"), None) else {
                return false;
            };
            if tray.0.is_null() {
                return false;
            }

            let mut tb_rect = RECT::default();
            if GetWindowRect(tb, &mut tb_rect).is_err() {
                return false;
            }
            // Skip vertical taskbars
            if tb_rect.bottom - tb_rect.top > tb_rect.right - tb_rect.left {
                return false;
            }

            // Detect Win11 XAML taskbar vs Win10 ReBarWindow32
            let is_win11_xaml = FindWindowExW(
                Some(tb),
                None,
                w!("Windows.UI.Composition.DesktopWindowContentBridge"),
                None,
            )
            .map(|h| !h.0.is_null())
            .unwrap_or(false);

            self.rebar_hwnd = if is_win11_xaml {
                None
            } else {
                FindWindowExW(Some(tb), None, w!("ReBarWindow32"), None)
                    .ok()
                    .filter(|h| !h.0.is_null())
            };

            let Ok(hinst) = GetModuleHandleW(None) else {
                return false;
            };

            {
                let mut reg = CLASS_REGISTERED.lock().unwrap();
                if !*reg {
                    let wc = WNDCLASSEXW {
                        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                        lpfnWndProc: Some(taskbar_wnd_proc),
                        hInstance: hinst.into(),
                        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR::default()),
                        lpszClassName: w!("WallpaperShufflerTaskbarWidget"),
                        hIcon: HICON::default(),
                        ..Default::default()
                    };
                    let popup_wc = WNDCLASSEXW {
                        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                        lpfnWndProc: Some(popup_wnd_proc),
                        hInstance: hinst.into(),
                        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR::default()),
                        lpszClassName: w!("WallpaperShufflerTaskbarPreview"),
                        hIcon: HICON::default(),
                        ..Default::default()
                    };
                    if RegisterClassExW(&wc) != 0 && RegisterClassExW(&popup_wc) != 0 {
                        *reg = true;
                    }
                }
            }

            let Ok(wnd) = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
                w!("WallpaperShufflerTaskbarWidget"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                None,
            ) else {
                return false;
            };

            self.hwnd = Some(wnd);
            self.taskbar_hwnd = Some(tb);
            self.visible = true;
            TASKBAR_ACTIVE.store(true, Ordering::Relaxed);

            self.last_dpi = self.current_dpi();
            self.paint();
            self.last_w = -1;
            self.reposition();
            let _ = ShowWindow(wnd, SW_SHOWNOACTIVATE);
            true
        }
    }

    fn ensure_popup_window(&mut self) -> Option<HWND> {
        unsafe {
            if let Some(pwnd) = self.popup_hwnd
                && IsWindow(Some(pwnd)).as_bool()
            {
                return Some(pwnd);
            }
            let Ok(hinst) = GetModuleHandleW(None) else {
                return None;
            };
            let Ok(pwnd) = CreateWindowExW(
                WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_NOACTIVATE
                    | WS_EX_TRANSPARENT,
                w!("WallpaperShufflerTaskbarPreview"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                None,
            ) else {
                return None;
            };
            self.popup_hwnd = Some(pwnd);
            Some(pwnd)
        }
    }

    fn show_popup(&mut self) {
        let (Some(wnd), Some(tb)) = (self.hwnd, self.taskbar_hwnd) else {
            return;
        };
        let Some(pwnd) = self.ensure_popup_window() else {
            return;
        };

        unsafe {
            let mut widget_rect = RECT::default();
            let mut tb_rect = RECT::default();
            if GetWindowRect(wnd, &mut widget_rect).is_err()
                || GetWindowRect(tb, &mut tb_rect).is_err()
            {
                return;
            }

            let hmon = MonitorFromWindow(tb, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let mon_rect = if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                mi.rcMonitor
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                }
            };

            let dpi = self.current_dpi();
            let dpi_scale = dpi as f32 / 96.0;
            let light = is_light_taskbar_theme();

            let need_render = !matches!(
                self.cached_popup_render,
                Some((cached_dpi, cached_light, _, _)) if cached_dpi == dpi && cached_light == light
            );
            if need_render {
                let src_dims = self.popup_image.as_ref().map(|i| (i.width(), i.height()));
                let layout = compute_popup_dimensions(
                    src_dims,
                    dpi_scale,
                    mon_rect.right - mon_rect.left,
                    mon_rect.bottom - mon_rect.top,
                );
                let rendered =
                    render_popup_preview(self.popup_image.as_ref(), layout, dpi_scale, light);
                push_rgba_to_layered_window(pwnd, &rendered, 255);
                self.cached_popup_render = Some((dpi, light, layout, rendered));
            } else if !self.popup_visible
                && let Some((_, _, _, ref rendered)) = self.cached_popup_render
            {
                push_rgba_to_layered_window(pwnd, rendered, 255);
            }

            let Some((_, _, layout, _)) = self.cached_popup_render else {
                return;
            };
            let (x, y) = compute_popup_position(
                widget_rect,
                tb_rect,
                mon_rect,
                layout.total_w as i32,
                layout.total_h as i32,
                layout.shadow as i32,
                dpi_scale,
            );
            let _ = SetWindowPos(
                pwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                layout.total_w as i32,
                layout.total_h as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            self.popup_visible = true;
        }
    }

    fn hide_popup(&mut self) {
        if let Some(pwnd) = self.popup_hwnd
            && self.popup_visible
        {
            unsafe {
                let _ = ShowWindow(pwnd, SW_HIDE);
            }
        }
        self.popup_visible = false;
    }

    fn update_popup_visibility(&mut self) {
        let should_show = self.visible && self.is_alive() && TASKBAR_HOVER.load(Ordering::SeqCst);
        if should_show {
            if !self.popup_visible {
                self.show_popup();
            }
        } else {
            self.hide_popup();
        }
    }

    pub fn destroy(&mut self) {
        unsafe {
            if let (Some(rebar), Some(tb)) = (self.rebar_hwnd, self.taskbar_hwnd)
                && IsWindow(Some(rebar)).as_bool()
                && IsWindow(Some(tb)).as_bool()
            {
                let mut r = RECT::default();
                let _ = GetClientRect(tb, &mut r);
                let _ = PostMessageW(
                    Some(tb),
                    WM_SIZE,
                    WPARAM(0),
                    LPARAM((((r.bottom & 0xffff) << 16) | (r.right & 0xffff)) as isize),
                );
            }
            self.rebar_hwnd = None;
            self.widgets_btn = None;
            if let Some(pwnd) = self.popup_hwnd.take() {
                let _ = DestroyWindow(pwnd);
            }
            self.popup_visible = false;
            if let Some(wnd) = self.hwnd.take() {
                let _ = DestroyWindow(wnd);
            }
            self.taskbar_hwnd = None;
            self.visible = false;
            TASKBAR_ACTIVE.store(false, Ordering::Relaxed);
            TASKBAR_HOVER.store(false, Ordering::SeqCst);
        }
    }

    pub fn set_visible(&mut self, show: bool) -> bool {
        if self.visible == show && (!show || self.is_alive()) {
            return true;
        }
        if show {
            if !self.is_alive() {
                return self.create();
            } else if let Some(wnd) = self.hwnd {
                unsafe {
                    let _ = ShowWindow(wnd, SW_SHOWNOACTIVATE);
                }
                self.visible = true;
                TASKBAR_ACTIVE.store(true, Ordering::Relaxed);
                self.paint();
                self.reposition();
            }
            true
        } else {
            self.hide_popup();
            if let Some(wnd) = self.hwnd {
                unsafe {
                    let _ = ShowWindow(wnd, SW_HIDE);
                }
            }
            self.visible = false;
            TASKBAR_ACTIVE.store(false, Ordering::Relaxed);
            true
        }
    }

    pub fn update_wallpaper(&mut self, image: Option<&image::RgbaImage>) {
        self.preview_image = image.map(|img| {
            image::imageops::resize(
                img,
                PREVIEW_CACHE_W,
                PREVIEW_CACHE_H,
                image::imageops::FilterType::Triangle,
            )
        });
        self.popup_image = image.map(resize_popup_cache);
        self.cached_popup_render = None;
        if self.is_alive() && self.visible {
            self.paint();
            self.reposition();
            if self.popup_visible {
                self.show_popup();
            }
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.is_paused == paused {
            return;
        }
        self.is_paused = paused;
        if self.is_alive() && self.visible {
            self.paint();
        }
    }

    pub fn set_status_text(&mut self, status: &str) {
        self.status_text = status.to_string();
    }

    pub fn tick(&mut self) {
        if !self.visible {
            self.hide_popup();
            return;
        }
        if !self.is_alive() {
            let _ = self.create();
            return;
        }
        let menu_open = CONTEXT_MENU_OPEN.load(Ordering::SeqCst);
        // Track cursor bounds via GetCursorPos while context menu captures input or as a fallback
        if (menu_open || TASKBAR_HOVER.load(Ordering::SeqCst))
            && let Some(wnd) = self.hwnd
        {
            unsafe {
                let mut pt = POINT::default();
                let mut rect = RECT::default();
                if GetCursorPos(&mut pt).is_ok() && GetWindowRect(wnd, &mut rect).is_ok() {
                    let over = PtInRect(&rect, pt).as_bool();
                    if TASKBAR_HOVER.load(Ordering::SeqCst) != over {
                        TASKBAR_HOVER.store(over, Ordering::SeqCst);
                        TASKBAR_NEEDS_REPAINT.store(true, Ordering::SeqCst);
                    }
                }
            }
        }
        if TASKBAR_NEEDS_REPAINT.swap(false, Ordering::SeqCst) {
            self.paint();
        }
        if !menu_open {
            self.reposition();
        }
        self.update_popup_visibility();
    }

    /// Lightweight Z-order-only check & restore (no UIA, no EnumWindows, no repaint).
    pub fn restore_z_order_if_needed(&mut self) {
        if !self.visible || CONTEXT_MENU_OPEN.load(Ordering::SeqCst) || !self.is_alive() {
            return;
        }
        let (Some(wnd), Some(tb)) = (self.hwnd, self.taskbar_hwnd) else {
            return;
        };
        unsafe {
            let mut tb_is_above = false;
            let mut cur = GetWindow(wnd, GW_HWNDPREV).ok();
            let mut steps = 0;
            while let Some(prev) = cur {
                if prev.0.is_null() || steps >= 32 {
                    break;
                }
                if prev == tb {
                    tb_is_above = true;
                    break;
                }
                cur = GetWindow(prev, GW_HWNDPREV).ok();
                steps += 1;
            }
            if tb_is_above || !IsWindowVisible(wnd).as_bool() {
                let _ = SetWindowPos(
                    wnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
        }
    }

    fn current_dpi(&self) -> u32 {
        unsafe {
            self.taskbar_hwnd
                .map(|tb| GetDpiForWindow(tb))
                .filter(|&d| d > 0)
                .unwrap_or(96)
        }
    }

    fn paint(&mut self) {
        let (Some(wnd), Some(tb)) = (self.hwnd, self.taskbar_hwnd) else {
            return;
        };
        let mut bar = RECT::default();
        unsafe {
            if GetClientRect(tb, &mut bar).is_err() {
                return;
            }
        }
        let bar_h = (bar.bottom - bar.top).max(1) as u32;
        let dpi_scale = self.current_dpi() as f32 / 96.0;
        let (w, h) = compute_taskbar_strip_size(bar_h, dpi_scale);
        if w == 0 || h == 0 {
            return;
        }

        let hovered = TASKBAR_HOVER.load(Ordering::SeqCst);
        let light = is_light_taskbar_theme();
        let img = render_taskbar_thumbnail(
            self.preview_image.as_ref(),
            w,
            h,
            dpi_scale,
            hovered,
            light,
            self.is_paused,
        );
        push_rgba_to_layered_window(wnd, &img, 255);
        self.width = w as i32;
        self.height = h as i32;
        self.last_w = -1;
    }

    fn widgets_button_rect(&mut self) -> RECT {
        if let Some(last) = self.last_uia_check
            && last.elapsed() < Duration::from_secs(5)
        {
            return self.cached_widgets_rect;
        }
        self.last_uia_check = Some(Instant::now());
        self.cached_widgets_rect = RECT::default();

        let Some(tb) = self.taskbar_hwnd else {
            return RECT::default();
        };

        unsafe {
            if self.uia.is_none() {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                if let Ok(uia) =
                    CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                {
                    self.uia = Some(uia);
                } else {
                    return RECT::default();
                }
            }

            let Some(uia) = self.uia.as_ref() else {
                return RECT::default();
            };

            if self.widgets_btn.is_none() {
                let Ok(root) = uia.ElementFromHandle(tb) else {
                    return RECT::default();
                };
                let bstr = BSTR::from("WidgetsButton");
                let var = VARIANT::from(bstr);
                if let Ok(cond) = uia.CreatePropertyCondition(UIA_AutomationIdPropertyId, &var)
                    && let Ok(btn) = root.FindFirst(TreeScope_Descendants, &cond)
                {
                    self.widgets_btn = Some(btn);
                }
            }

            if let Some(btn) = self.widgets_btn.as_ref() {
                if let Ok(rect) = btn.CurrentBoundingRectangle() {
                    self.cached_widgets_rect = rect;
                } else {
                    self.widgets_btn = None;
                }
            }
        }

        self.cached_widgets_rect
    }

    fn reposition(&mut self) {
        if !self.is_alive() {
            return;
        }
        let dpi = self.current_dpi();
        if dpi != self.last_dpi {
            self.last_dpi = dpi;
            self.paint();
        }
        let Some(tb) = self.taskbar_hwnd else {
            return;
        };
        let Some(wnd) = self.hwnd else {
            return;
        };

        unsafe {
            let Ok(tray) = FindWindowExW(Some(tb), None, w!("TrayNotifyWnd"), None) else {
                return;
            };
            if tray.0.is_null() {
                return;
            }

            let mut tr = RECT::default();
            let mut bar = RECT::default();
            if GetWindowRect(tray, &mut tr).is_err() || GetWindowRect(tb, &mut bar).is_err() {
                return;
            }

            let rtl = (GetWindowLongW(tb, GWL_EXSTYLE) as u32 & WS_EX_LAYOUTRTL.0) != 0;
            let edge = if rtl { tr.right } else { tr.left };

            // Collect obstacles on the taskbar band: Win11 Widgets button + peer taskbar widgets
            let wb = self.widgets_button_rect();
            let peers = collect_peer_taskbar_rects(wnd, tb, bar);
            let px2 = ((2 * dpi as i32) / 96).max(2);

            let mut obstacles = peers.clone();
            if wb.right > wb.left && wb.top < bar.bottom && wb.bottom > bar.top {
                obstacles.push(wb);
            }

            let screen_left = resolve_non_overlapping_left(edge, self.width, px2, rtl, &obstacles);

            // Win10 ReBarWindow32 adjustment
            if let Some(rebar) = self.rebar_hwnd
                && IsWindow(Some(rebar)).as_bool()
            {
                let mut rr = RECT::default();
                if GetWindowRect(rebar, &mut rr).is_ok() {
                    let mut tl = POINT {
                        x: if rtl { rr.right } else { rr.left },
                        y: rr.top,
                    };
                    let _ = ScreenToClient(tb, &mut tl);
                    let mut lc = POINT {
                        x: screen_left,
                        y: bar.top,
                    };
                    let _ = ScreenToClient(tb, &mut lc);
                    if tl.x + (rr.right - rr.left) > lc.x && lc.x > tl.x {
                        let _ =
                            MoveWindow(rebar, tl.x, tl.y, lc.x - tl.x, rr.bottom - rr.top, true);
                    }
                }
            }

            // Ensure our widget remains above the taskbar in Z-order
            let mut tb_is_above = false;
            let mut cur = GetWindow(wnd, GW_HWNDPREV).ok();
            let mut steps = 0;
            while let Some(prev) = cur {
                if prev.0.is_null() || steps >= 32 {
                    break;
                }
                if prev == tb {
                    tb_is_above = true;
                    break;
                }
                cur = GetWindow(prev, GW_HWNDPREV).ok();
                steps += 1;
            }
            let needs_z_restore = tb_is_above || !IsWindowVisible(wnd).as_bool();

            let rect_unchanged = tr == self.last_tray
                && bar == self.last_bar
                && wb == self.last_widgets
                && peers == self.last_peers
                && self.last_w == self.width
                && self.last_left == screen_left;

            if rect_unchanged && !needs_z_restore {
                return;
            }

            if rect_unchanged && needs_z_restore {
                let _ = SetWindowPos(
                    wnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
                );
                return;
            }

            self.last_tray = tr;
            self.last_bar = bar;
            self.last_widgets = wb;
            self.last_peers = peers;
            self.last_w = self.width;
            self.last_left = screen_left;

            let _ = SetWindowPos(
                wnd,
                Some(HWND_TOPMOST),
                screen_left,
                bar.top,
                self.width,
                self.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

impl Drop for TaskbarWidget {
    fn drop(&mut self) {
        self.destroy();
    }
}

pub fn init_taskbar(hwnd: HWND, initial_image: Option<&image::RgbaImage>) {
    let widget = TaskbarWidget::new(hwnd, initial_image);
    let mut guard = GLOBAL_TASKBAR.lock().unwrap();
    *guard = Some(widget);
}

pub fn set_taskbar_visible(show: bool) -> bool {
    if let Some(widget) = GLOBAL_TASKBAR.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        return widget.set_visible(show);
    }
    false
}

pub fn update_taskbar_wallpaper(image: Option<&image::RgbaImage>) {
    if let Some(widget) = GLOBAL_TASKBAR.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        widget.update_wallpaper(image);
    }
}

pub fn set_taskbar_paused(paused: bool) {
    if let Some(widget) = GLOBAL_TASKBAR.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        widget.set_paused(paused);
    }
}

pub fn set_taskbar_status(status: &str) {
    if let Some(widget) = GLOBAL_TASKBAR.lock().ok().as_mut().and_then(|g| g.as_mut()) {
        widget.set_status_text(status);
    }
}

pub fn set_context_menu_open(open: bool) {
    CONTEXT_MENU_OPEN.store(open, Ordering::SeqCst);
}

pub fn is_taskbar_active() -> bool {
    TASKBAR_ACTIVE.load(Ordering::Relaxed)
}

pub fn restore_taskbar_z_order() {
    if let Ok(mut guard) = GLOBAL_TASKBAR.try_lock()
        && let Some(widget) = guard.as_mut()
    {
        widget.restore_z_order_if_needed();
    }
}

pub fn tick_taskbar() {
    if let Ok(mut guard) = GLOBAL_TASKBAR.try_lock()
        && let Some(widget) = guard.as_mut()
    {
        widget.tick();
    }
}

pub fn destroy_taskbar() {
    let mut guard = GLOBAL_TASKBAR.lock().unwrap();
    *guard = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_taskbar_strip_size() {
        let (w, h) = compute_taskbar_strip_size(48, 1.0);
        assert_eq!(h, 48);
        assert!((60..=80).contains(&w), "unexpected strip width: {}", w);

        let (w_hi, h_hi) = compute_taskbar_strip_size(72, 1.5);
        assert_eq!(h_hi, 72);
        assert!(w_hi > w);
    }

    #[test]
    fn test_render_taskbar_thumbnail_default_and_custom() {
        let (w, h) = compute_taskbar_strip_size(48, 1.0);
        let default_img = render_taskbar_thumbnail(None, w, h, 1.0, false, false, false);
        assert_eq!(default_img.dimensions(), (w, h));
        // Outer corner maintains hit-test alpha = 1
        assert_eq!(default_img.get_pixel(0, 0)[3], 1);
        // Center pixel inside thumbnail is opaque
        assert_eq!(default_img.get_pixel(w / 2, h / 2)[3], 255);

        let src = image::RgbaImage::from_pixel(320, 180, image::Rgba([64, 128, 220, 255]));
        let custom_img = render_taskbar_thumbnail(Some(&src), w, h, 1.0, true, true, true);
        assert_eq!(custom_img.dimensions(), (w, h));
        assert_eq!(custom_img.get_pixel(w / 2, h / 2)[3], 255);
    }

    #[test]
    fn test_resolve_non_overlapping_left_avoids_multiple_peers() {
        let obs = vec![
            RECT {
                left: 1900,
                top: 1032,
                right: 2100,
                bottom: 1080,
            },
            RECT {
                left: 1810,
                top: 1032,
                right: 1898,
                bottom: 1080,
            },
        ];
        let left = resolve_non_overlapping_left(2102, 72, 2, false, &obs);
        // Must shift past both peers: 1810 - 2 - 72 = 1736
        assert_eq!(left, 1736);

        // RTL test
        let rtl_obs = vec![RECT {
            left: 100,
            top: 1032,
            right: 250,
            bottom: 1080,
        }];
        let rtl_left = resolve_non_overlapping_left(98, 72, 2, true, &rtl_obs);
        assert_eq!(rtl_left, 252);
    }

    #[test]
    fn test_compute_popup_dimensions_scales_and_clamps() {
        let layout_100 = compute_popup_dimensions(Some((3840, 2160)), 1.0, 1920, 1080);
        assert_eq!(layout_100.img_w, 600);
        assert_eq!(layout_100.img_h, 338);
        assert_eq!(
            layout_100.total_w,
            layout_100.img_w + (layout_100.pad + layout_100.shadow) * 2
        );

        let layout_150 = compute_popup_dimensions(Some((3840, 2160)), 1.5, 3840, 2160);
        assert_eq!(layout_150.img_w, 900);
        assert_eq!(layout_150.img_h, 506);
    }

    #[test]
    fn test_render_popup_preview_dimensions_and_pixels() {
        let layout = compute_popup_dimensions(Some((1920, 1080)), 1.0, 1920, 1080);
        let src = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([30, 140, 220, 255]));
        let rendered = render_popup_preview(Some(&src), layout, 1.0, false);
        assert_eq!(rendered.dimensions(), (layout.total_w, layout.total_h));
        // Outer corner is transparent
        assert_eq!(rendered.get_pixel(0, 0)[3], 0);
        // Center pixel matches source color
        let center = rendered.get_pixel(layout.total_w / 2, layout.total_h / 2);
        assert_eq!(center[3], 255);
        assert_eq!(center[0], 30);
        assert_eq!(center[1], 140);
        assert_eq!(center[2], 220);
    }

    #[test]
    fn test_compute_popup_position_clamps_to_monitor() {
        let widget = RECT {
            left: 1750,
            top: 1032,
            right: 1820,
            bottom: 1080,
        };
        let taskbar = RECT {
            left: 0,
            top: 1032,
            right: 1920,
            bottom: 1080,
        };
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let (x, y) = compute_popup_position(widget, taskbar, monitor, 628, 366, 8, 1.0);
        // Clamped so right edge stays inside monitor (1920 - 8 + 8 - 628 = 1292)
        assert_eq!(x, 1292);
        // Placed above bottom taskbar: 1032 - 366 - (8 - 8) = 666
        assert_eq!(y, 666);
    }
}
