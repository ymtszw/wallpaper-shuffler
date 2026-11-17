use crate::tray::WM_TRAYICON;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    COLORREF, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, ScreenToClient,
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
use windows::Win32::UI::Controls::{
    ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASSW, TTF_IDISHWND,
    TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, FindWindowExW, FindWindowW,
    GW_HWNDPREV, GWL_EXSTYLE, GetClassNameW, GetClientRect, GetWindow, GetWindowLongW,
    GetWindowRect, HCURSOR, HICON, HMENU, HWND_TOPMOST, IDC_ARROW, IsWindow, IsWindowVisible,
    LoadCursorW, MA_NOACTIVATE, MoveWindow, PostMessageW, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SendMessageW,
    SetWindowPos, ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WINDOW_STYLE, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SIZE, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_LAYOUTRTL, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{BOOL, BSTR, PWSTR, w};

const WM_MOUSELEAVE: u32 = 0x02A3;
const PREVIEW_CACHE_W: u32 = 160;
const PREVIEW_CACHE_H: u32 = 90;

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
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            TASKBAR_HOVER.store(false, Ordering::SeqCst);
            TASKBAR_NEEDS_REPAINT.store(true, Ordering::SeqCst);
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
    taskbar_hwnd: Option<HWND>,
    rebar_hwnd: Option<HWND>,
    tip_hwnd: Option<HWND>,
    notify_hwnd: HWND,
    preview_image: Option<image::RgbaImage>,
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
    last_tip_text: String,
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
        Self {
            hwnd: None,
            taskbar_hwnd: None,
            rebar_hwnd: None,
            tip_hwnd: None,
            notify_hwnd,
            preview_image: cached,
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
            last_tip_text: String::new(),
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
                    if RegisterClassExW(&wc) != 0 {
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

            let icc = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_WIN95_CLASSES,
            };
            let _ = InitCommonControlsEx(&icc);

            if let Ok(tip) = CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                0,
                0,
                0,
                0,
                Some(self.notify_hwnd),
                Some(HMENU::default()),
                Some(hinst.into()),
                None,
            ) {
                let mut title: Vec<u16> =
                    "Wallpaper Shuffler".encode_utf16().chain(Some(0)).collect();
                let ti = TTTOOLINFOW {
                    cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                    uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                    hwnd: self.notify_hwnd,
                    uId: wnd.0 as usize,
                    lpszText: PWSTR(title.as_mut_ptr()),
                    ..Default::default()
                };
                SendMessageW(
                    tip,
                    TTM_ADDTOOLW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&ti as *const _ as isize)),
                );
                SendMessageW(tip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(400)));
                self.tip_hwnd = Some(tip);
            }

            self.last_dpi = self.current_dpi();
            self.paint();
            self.update_tooltip();
            self.last_w = -1;
            self.reposition();
            let _ = ShowWindow(wnd, SW_SHOWNOACTIVATE);
            true
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
            if let Some(tip) = self.tip_hwnd.take() {
                let _ = DestroyWindow(tip);
            }
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
        if self.is_alive() && self.visible {
            self.paint();
            self.reposition();
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.is_paused == paused {
            return;
        }
        self.is_paused = paused;
        self.update_tooltip();
        if self.is_alive() && self.visible {
            self.paint();
        }
    }

    pub fn set_status_text(&mut self, status: &str) {
        self.status_text = status.to_string();
        self.update_tooltip();
    }

    pub fn tick(&mut self) {
        if !self.visible {
            return;
        }
        if CONTEXT_MENU_OPEN.load(Ordering::SeqCst) {
            return;
        }
        if !self.is_alive() {
            let _ = self.create();
            return;
        }
        if TASKBAR_NEEDS_REPAINT.swap(false, Ordering::SeqCst) {
            self.paint();
        }
        self.reposition();
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

    fn update_tooltip(&mut self) {
        let Some(tip) = self.tip_hwnd else {
            return;
        };
        let Some(wnd) = self.hwnd else {
            return;
        };
        let state = if self.is_paused {
            "Paused"
        } else {
            self.status_text.as_str()
        };
        let text = format!("Wallpaper Shuffler ({})\nClick: Open Menu", state);
        if text == self.last_tip_text {
            return;
        }
        self.last_tip_text = text.clone();
        let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let ti = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            hwnd: self.notify_hwnd,
            uId: wnd.0 as usize,
            lpszText: PWSTR(wide.as_mut_ptr()),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                tip,
                TTM_UPDATETIPTEXTW,
                Some(WPARAM(0)),
                Some(LPARAM(&ti as *const _ as isize)),
            );
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
}
