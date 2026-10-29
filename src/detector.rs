use chrono::{DateTime, Utc};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetShellWindow, GetSystemMetrics,
    GetWindowLongW, GetWindowRect, IsIconic, IsWindowVisible, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WS_EX_TOOLWINDOW, WS_VISIBLE,
};
use windows::core::BOOL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl ScreenRect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn area(&self) -> i64 {
        let w = self.width().max(0) as i64;
        let h = self.height().max(0) as i64;
        w * h
    }

    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

pub fn get_virtual_screen_bounds() -> ScreenRect {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        ScreenRect {
            left: x,
            top: y,
            right: x + cx,
            bottom: y + cy,
        }
    }
}

pub fn is_window_valid_and_visible(hwnd: HWND) -> bool {
    unsafe {
        if hwnd == GetShellWindow() {
            return false;
        }

        let mut class_buf = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class_buf) as usize;
        if len > 0 {
            let class_name = String::from_utf16_lossy(&class_buf[..len]);
            if class_name == "Progman" || class_name == "WorkerW" {
                return false;
            }
        }

        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }

        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_VISIBLE.0 == 0 {
            return false;
        }

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }

        // Check if window is cloaked by DWM (e.g. background virtual desktop or suspended modern app)
        let mut cloaked: u32 = 0;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
        if hr.is_ok() && cloaked != 0 {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }

        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        w > 0 && h > 0
    }
}

pub fn collect_visible_window_rects() -> Vec<ScreenRect> {
    struct EnumState {
        windows: Vec<ScreenRect>,
    }

    let mut state = EnumState {
        windows: Vec::new(),
    };

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = unsafe { &mut *(lparam.0 as *mut EnumState) };
        if is_window_valid_and_visible(hwnd) {
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect).is_ok() } {
                state.windows.push(ScreenRect {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                });
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut state as *mut EnumState as isize),
        );
    }

    state.windows
}

/// Computes desktop exposure percentage (0.0% to 100.0%) via fast grid sampling
pub fn calculate_desktop_exposure_rate() -> f64 {
    let screen = get_virtual_screen_bounds();
    let sw = screen.width();
    let sh = screen.height();
    if sw <= 0 || sh <= 0 {
        return 100.0;
    }

    let window_rects = collect_visible_window_rects();
    if window_rects.is_empty() {
        return 100.0;
    }

    // Grid sampling: 48 x 27 grid = 1,296 sample points across 16:9 screen
    let grid_x = 48i32;
    let grid_y = 27i32;
    let total_samples = grid_x * grid_y;
    let mut exposed_samples = 0i32;

    let step_x = sw / grid_x;
    let step_y = sh / grid_y;

    for gx in 0..grid_x {
        let px = screen.left + gx * step_x + step_x / 2;
        for gy in 0..grid_y {
            let py = screen.top + gy * step_y + step_y / 2;
            let mut covered = false;
            for wr in &window_rects {
                if wr.contains_point(px, py) {
                    covered = true;
                    break;
                }
            }
            if !covered {
                exposed_samples += 1;
            }
        }
    }

    (exposed_samples as f64 / total_samples as f64) * 100.0
}

pub fn has_cooldown_elapsed(last_shuffle: Option<DateTime<Utc>>, cooldown_seconds: u64) -> bool {
    match last_shuffle {
        None => true,
        Some(t) => {
            let elapsed = Utc::now().signed_duration_since(t);
            elapsed.num_seconds().max(0) as u64 >= cooldown_seconds
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_rect_area() {
        let r = ScreenRect {
            left: 0,
            top: 0,
            right: 3840,
            bottom: 2160,
        };
        assert_eq!(r.width(), 3840);
        assert_eq!(r.height(), 2160);
        assert_eq!(r.area(), 3840 * 2160);
        assert!(r.contains_point(100, 100));
        assert!(!r.contains_point(4000, 100));
    }

    #[test]
    fn test_cooldown_logic() {
        assert!(has_cooldown_elapsed(None, 300));
        let past = Utc::now() - chrono::Duration::seconds(400);
        assert!(has_cooldown_elapsed(Some(past), 300));
        let recent = Utc::now() - chrono::Duration::seconds(10);
        assert!(!has_cooldown_elapsed(Some(recent), 300));
    }
}
