#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod composer;
mod config;
mod detector;
mod fixtures;
mod hook;
mod logger;
mod startup;
mod taskbar;
mod tray;
mod wallpaper;

use logger::LogLevel;
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Instant;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, MSG,
    PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, WM_COMMAND, WM_CONTEXTMENU,
    WM_DESTROY, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);
static LAST_DEBOUNCE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(400);
static IS_COMPOSING: AtomicBool = AtomicBool::new(false);
static IS_APPLYING: AtomicBool = AtomicBool::new(false);
static IS_PAUSED: AtomicBool = AtomicBool::new(false);
static STANDBY_WALLPAPER: std::sync::Mutex<Option<composer::CompositionResult>> =
    std::sync::Mutex::new(None);
static WAS_DESKTOP_OBSCURED: AtomicBool = AtomicBool::new(true);
static PENDING_SYSTEM_TRIGGER: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

const TIMER_DEBOUNCE_ID: usize = 9001;
const TIMER_WATCH_ID: usize = 9002;
const RESUME_SETTLE_MIN_MS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum PendingSystemTrigger {
    None = 0,
    Startup = 1,
    PowerResume = 2,
}

impl PendingSystemTrigger {
    fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Startup,
            2 => Self::PowerResume,
            _ => Self::None,
        }
    }
}

static LAST_WATCH_STATE: std::sync::Mutex<Option<(usize, f64)>> = std::sync::Mutex::new(None);

fn on_window_activity_detected(source: &str) {
    if IS_PAUSED.load(Ordering::Relaxed) {
        return;
    }

    let target = TARGET_HWND.load(Ordering::Relaxed);
    if target == 0 {
        return;
    }

    log_debug!(
        "[{}] Window movement/resize/minimize detected! Pre-composing next wallpaper...",
        source
    );

    let base_debounce_ms = LAST_DEBOUNCE_MS.load(Ordering::Relaxed);
    let pending = PendingSystemTrigger::from_u8(PENDING_SYSTEM_TRIGGER.load(Ordering::Relaxed));
    let debounce_ms = if pending == PendingSystemTrigger::PowerResume {
        base_debounce_ms.max(RESUME_SETTLE_MIN_MS) as u32
    } else {
        base_debounce_ms as u32
    };
    unsafe {
        let _ = SetTimer(
            Some(HWND(target as *mut _)),
            TIMER_DEBOUNCE_ID,
            debounce_ms,
            None,
        );
    }

    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
    ensure_candidate_precomposed_async(&cfg);
}

fn on_power_resume_detected(hwnd: HWND) {
    if IS_PAUSED.load(Ordering::Relaxed) {
        log_debug!("[POWER RESUME] System resumed, but auto-shuffle is paused. Ignoring.");
        return;
    }

    PENDING_SYSTEM_TRIGGER.store(PendingSystemTrigger::PowerResume as u8, Ordering::SeqCst);
    let settle_ms = LAST_DEBOUNCE_MS
        .load(Ordering::Relaxed)
        .max(RESUME_SETTLE_MIN_MS) as u32;
    log_info!(
        "[POWER RESUME] System resumed from sleep/suspend. Checking desktop exposure after {}ms settle...",
        settle_ms
    );
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_DEBOUNCE_ID, settle_ms, None);
    }

    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
    ensure_candidate_precomposed_async(&cfg);
}

fn check_window_watch() {
    let visible_windows = detector::collect_visible_window_rects();
    let current_exposure = detector::calculate_desktop_exposure_rate();

    let mut guard = LAST_WATCH_STATE.lock().unwrap();
    if let Some((prev_count, prev_exp)) = *guard {
        // Window moved, minimized, closed, or resized (exposure or window count changed)
        if prev_count != visible_windows.len() || (prev_exp - current_exposure).abs() >= 1.0 {
            *guard = Some((visible_windows.len(), current_exposure));
            on_window_activity_detected(&format!(
                "WATCH (Windows: {} -> {}, Exposure: {:.1}% -> {:.1}%)",
                prev_count,
                visible_windows.len(),
                prev_exp,
                current_exposure
            ));
            return;
        }
    }
    *guard = Some((visible_windows.len(), current_exposure));
}

fn get_history_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .map(|p| PathBuf::from(p).join(".wallpaper-shuffler").join("history"))
        .unwrap_or_else(|_| PathBuf::from("target").join("history"))
}

/// Speculatively pre-composes the next wallpaper candidate in a background thread
/// as soon as window activity is detected, caching it in STANDBY_WALLPAPER.
fn ensure_candidate_precomposed_async(cfg: &config::AppConfig) {
    // 1. Check if an unconsumed standby candidate is already ready
    {
        let guard = STANDBY_WALLPAPER.lock().unwrap();
        if guard.is_some() {
            log_debug!(
                "[STANDBY CACHE] Ready candidate already standby. Skipping duplicate composition."
            );
            return;
        }
    }

    // 2. Prevent overlapping composition tasks
    if IS_COMPOSING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let wallpaper_dirs = cfg.wallpaper_dirs.clone();
    let history_count = cfg.history_count;
    let history_dir = get_history_dir();

    log_info!(">>> [GENERATE START] Generating wallpaper candidate in background thread...");

    std::thread::spawn(move || {
        let res =
            composer::execute_shuffle_and_compose(&wallpaper_dirs, &history_dir, history_count);
        match res {
            Ok(comp_res) => {
                log_info!(
                    "[GENERATE READY] Composed {:?} wallpaper in {}ms (Standby: {})",
                    comp_res.mode,
                    comp_res.duration_ms,
                    comp_res.output_path.display()
                );
                *STANDBY_WALLPAPER.lock().unwrap() = Some(comp_res);
            }
            Err(e) => {
                log_error!("[GENERATE ERROR] Background composition failed: {}", e);
            }
        }
        IS_COMPOSING.store(false, Ordering::SeqCst);
    });
}

fn attach_parent_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

fn event_name(event: u32) -> &'static str {
    match event {
        0x0003 => "FOREGROUND_CHANGE",
        0x000A => "MOVESIZE_START",
        0x000B => "MOVESIZE_END",
        0x0016 => "MINIMIZE_START",
        0x0017 => "MINIMIZE_END",
        _ => "WINDOW_ACTIVITY",
    }
}

unsafe extern "system" fn win_event_hook_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let target = TARGET_HWND.load(Ordering::Relaxed);
    if target == 0 || hwnd.0.is_null() {
        return;
    }

    // Only process window-level events (OBJID_WINDOW = 0).
    // Child controls (OBJID_CLIENT, OBJID_CURSOR, etc.) are ignored.
    if id_object != 0 {
        return;
    }

    // Ignore events originating from our own resident daemon window
    if hwnd.0 == target as *mut _ {
        return;
    }

    if event == 0x0003 && taskbar::is_taskbar_active() {
        taskbar::restore_taskbar_z_order();
    }

    on_window_activity_detected(&format!("HOOK: {}", event_name(event)));
}

const WM_POWERBROADCAST: u32 = 0x0218;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
const PBT_APMRESUMESUSPEND: usize = 0x0007;

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == TIMER_DEBOUNCE_ID {
                unsafe {
                    let _ = KillTimer(Some(hwnd), TIMER_DEBOUNCE_ID);
                }
                on_debounce_settled();
                return LRESULT(0);
            } else if wparam.0 == TIMER_WATCH_ID {
                check_window_watch();
                taskbar::tick_taskbar();
                return LRESULT(0);
            }
        }
        WM_POWERBROADCAST => {
            if wparam.0 == PBT_APMRESUMEAUTOMATIC || wparam.0 == PBT_APMRESUMESUSPEND {
                on_power_resume_detected(hwnd);
            }
        }
        tray::WM_TRAYICON => {
            let event = (lparam.0 as usize & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
                let is_paused = IS_PAUSED.load(Ordering::Relaxed);
                let log_level = logger::get_log_level();
                tray::show_tray_context_menu(hwnd, &cfg, is_paused, log_level);
            }
        }
        WM_COMMAND => match wparam.0 {
            tray::IDM_SHUFFLE_NOW => {
                trigger_manual_shuffle();
            }
            tray::IDM_TOGGLE_PAUSE => {
                let prev = IS_PAUSED.fetch_xor(true, Ordering::SeqCst);
                let now_paused = !prev;
                if now_paused {
                    log_info!("Auto-shuffle paused by user via tray menu.");
                    tray::update_tray_tooltip("Wallpaper Shuffler (Paused)");
                } else {
                    log_info!("Auto-shuffle resumed by user via tray menu.");
                    tray::update_tray_tooltip("Wallpaper Shuffler");
                }
                taskbar::set_taskbar_paused(now_paused);
            }
            tray::IDM_SETTINGS_INFO => {
                let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
                let config_path = get_config_path();
                tray::show_settings_dialog(hwnd, &cfg, &config_path);
            }
            tray::IDM_OPEN_CONFIG => {
                let config_path = get_config_path();
                tray::open_path_with_shell(hwnd, &config_path);
            }
            tray::IDM_RELOAD_CONFIG => {
                let config_path = get_config_path();
                match config::AppConfig::load_from_file(&config_path) {
                    Ok(new_cfg) => {
                        let new_log_level = LogLevel::parse(&new_cfg.log_level);
                        logger::set_log_level(new_log_level);
                        LAST_DEBOUNCE_MS.store(new_cfg.debounce_millis, Ordering::SeqCst);
                        *APP_CONFIG.lock().unwrap() = Some(new_cfg.clone());
                        apply_display_mode(new_cfg.display_mode);
                        log_info!(
                            "Configuration reloaded from {}: {:?}",
                            config_path.display(),
                            new_cfg
                        );
                    }
                    Err(e) => {
                        log_error!("Failed to reload config: {}", e);
                    }
                }
            }
            tray::IDM_OPEN_WALLPAPERS | tray::IDM_STATUS_FOLDER => {
                let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
                if let Some(folder) = cfg.wallpaper_dirs.first() {
                    tray::open_path_with_shell(hwnd, folder);
                }
            }
            tray::IDM_OPEN_LOGS => {
                let log_dir = logger::default_log_dir();
                tray::open_path_with_shell(hwnd, &log_dir);
            }
            tray::IDM_TOGGLE_STARTUP => {
                if startup::is_startup_enabled() {
                    match startup::unregister_startup() {
                        Ok(()) => log_info!("Windows startup disabled via tray menu."),
                        Err(e) => log_error!("Failed to unregister startup: {}", e),
                    }
                } else if let Ok(exe_path) = env::current_exe() {
                    match startup::register_startup(exe_path.as_os_str()) {
                        Ok(()) => log_info!("Windows startup enabled via tray menu."),
                        Err(e) => log_error!("Failed to register startup: {}", e),
                    }
                }
            }
            tray::IDM_LOG_DEBUG => update_active_log_level(LogLevel::Debug),
            tray::IDM_LOG_INFO => update_active_log_level(LogLevel::Info),
            tray::IDM_LOG_WARN => update_active_log_level(LogLevel::Warn),
            tray::IDM_LOG_ERROR => update_active_log_level(LogLevel::Error),
            tray::IDM_LOG_OFF => update_active_log_level(LogLevel::Off),
            tray::IDM_MODE_TRAY => update_display_mode(config::DisplayMode::Tray),
            tray::IDM_MODE_TASKBAR => update_display_mode(config::DisplayMode::Taskbar),
            tray::IDM_EXIT => unsafe {
                taskbar::destroy_taskbar();
                PostQuitMessage(0);
            },
            _ => {}
        },
        WM_DESTROY => {
            taskbar::destroy_taskbar();
            unsafe {
                PostQuitMessage(0);
            }
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

static PREVIOUS_EXPOSURE: std::sync::Mutex<Option<f64>> = std::sync::Mutex::new(None);
static LAST_SHUFFLE_TIME: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>> =
    std::sync::Mutex::new(None);
static APP_CONFIG: std::sync::Mutex<Option<config::AppConfig>> = std::sync::Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExposureDecision {
    TriggerShuffle {
        next_was_obscured: bool,
    },
    Skip {
        next_was_obscured: bool,
        cooldown_blocked: bool,
    },
}

fn decide_shuffle_on_exposure(
    system_trigger: PendingSystemTrigger,
    current_exposure: f64,
    was_obscured: bool,
    cooldown_elapsed: bool,
    obscured_threshold: f64,
    exposed_threshold: f64,
) -> ExposureDecision {
    match system_trigger {
        PendingSystemTrigger::Startup | PendingSystemTrigger::PowerResume => {
            if current_exposure >= exposed_threshold {
                if cooldown_elapsed {
                    ExposureDecision::TriggerShuffle {
                        next_was_obscured: false,
                    }
                } else {
                    ExposureDecision::Skip {
                        next_was_obscured: was_obscured,
                        cooldown_blocked: true,
                    }
                }
            } else {
                // Wallpaper is not exposed (< exposed_threshold): skip shuffle without burning cooldown,
                // and arm so the next reveal triggers immediately.
                ExposureDecision::Skip {
                    next_was_obscured: true,
                    cooldown_blocked: false,
                }
            }
        }
        PendingSystemTrigger::None => {
            if current_exposure < obscured_threshold {
                ExposureDecision::Skip {
                    next_was_obscured: true,
                    cooldown_blocked: false,
                }
            } else if current_exposure >= exposed_threshold {
                if was_obscured {
                    if cooldown_elapsed {
                        ExposureDecision::TriggerShuffle {
                            next_was_obscured: false,
                        }
                    } else {
                        ExposureDecision::Skip {
                            next_was_obscured: true,
                            cooldown_blocked: true,
                        }
                    }
                } else {
                    ExposureDecision::Skip {
                        next_was_obscured: false,
                        cooldown_blocked: false,
                    }
                }
            } else {
                ExposureDecision::Skip {
                    next_was_obscured: was_obscured,
                    cooldown_blocked: false,
                }
            }
        }
    }
}

fn on_debounce_settled() {
    let system_trigger = PendingSystemTrigger::from_u8(
        PENDING_SYSTEM_TRIGGER.swap(PendingSystemTrigger::None as u8, Ordering::SeqCst),
    );

    if IS_PAUSED.load(Ordering::Relaxed) {
        return;
    }

    let visible_windows = detector::collect_visible_window_rects();
    let current_exposure = detector::calculate_desktop_exposure_rate();
    let is_foreground = logger::is_foreground();

    let mut prev_guard = PREVIOUS_EXPOSURE.lock().unwrap();
    let prev_exposure = prev_guard.unwrap_or(current_exposure);
    *prev_guard = Some(current_exposure);
    drop(prev_guard);

    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
    let was_obscured = WAS_DESKTOP_OBSCURED.load(Ordering::SeqCst);
    let last_time = *LAST_SHUFFLE_TIME.lock().unwrap();
    let effective_cooldown = if is_foreground {
        5 // Short 5-second cooldown during foreground testing for instant responsive feedback
    } else {
        cfg.cooldown_seconds
    };
    let cooldown_elapsed = detector::has_cooldown_elapsed(last_time, effective_cooldown);

    let decision = decide_shuffle_on_exposure(
        system_trigger,
        current_exposure,
        was_obscured,
        cooldown_elapsed,
        cfg.obscured_threshold_percent,
        cfg.exposed_threshold_percent,
    );

    match (system_trigger, decision) {
        (PendingSystemTrigger::Startup, ExposureDecision::TriggerShuffle { next_was_obscured }) => {
            WAS_DESKTOP_OBSCURED.store(next_was_obscured, Ordering::SeqCst);
            log_info!(
                ">>> [TRIGGER: STARTUP] Desktop is exposed ({:.1}% >= {:.1}%). Applying initial wallpaper...",
                current_exposure,
                cfg.exposed_threshold_percent
            );
            apply_or_await_wallpaper(&cfg);
        }
        (
            PendingSystemTrigger::Startup,
            ExposureDecision::Skip {
                next_was_obscured,
                cooldown_blocked,
            },
        ) => {
            WAS_DESKTOP_OBSCURED.store(next_was_obscured, Ordering::SeqCst);
            if cooldown_blocked {
                log_info!(
                    "[STARTUP COOLDOWN] Desktop is exposed ({:.1}%), but cooldown ({}s) is active. Skipping shuffle.",
                    current_exposure,
                    effective_cooldown
                );
            } else {
                log_info!(
                    "[STARTUP ARMED] Desktop is covered ({:.1}% < {:.1}%). Skipping shuffle; armed for first reveal.",
                    current_exposure,
                    cfg.exposed_threshold_percent
                );
            }
        }
        (
            PendingSystemTrigger::PowerResume,
            ExposureDecision::TriggerShuffle { next_was_obscured },
        ) => {
            WAS_DESKTOP_OBSCURED.store(next_was_obscured, Ordering::SeqCst);
            log_info!(
                ">>> [TRIGGER: POWER RESUME] Desktop is exposed ({:.1}% >= {:.1}%) after resume! Applying wallpaper...",
                current_exposure,
                cfg.exposed_threshold_percent
            );
            apply_or_await_wallpaper(&cfg);
        }
        (
            PendingSystemTrigger::PowerResume,
            ExposureDecision::Skip {
                next_was_obscured,
                cooldown_blocked,
            },
        ) => {
            WAS_DESKTOP_OBSCURED.store(next_was_obscured, Ordering::SeqCst);
            if cooldown_blocked {
                log_info!(
                    "[POWER RESUME COOLDOWN] Desktop is exposed ({:.1}%), but cooldown ({}s) is active. Skipping shuffle.",
                    current_exposure,
                    effective_cooldown
                );
            } else {
                log_info!(
                    "[POWER RESUME ARMED] Desktop is covered ({:.1}% < {:.1}%). Skipping shuffle; armed for next reveal (cooldown preserved).",
                    current_exposure,
                    cfg.exposed_threshold_percent
                );
            }
        }
        (PendingSystemTrigger::None, ExposureDecision::TriggerShuffle { next_was_obscured }) => {
            log_debug!(
                "[DEBOUNCE SETTLED] Windows: {} | Exposure: {:.1}% (Prev: {:.1}%, Target: hidden < {:.1}%, exposed >= {:.1}%, Armed: {})",
                visible_windows.len(),
                current_exposure,
                prev_exposure,
                cfg.obscured_threshold_percent,
                cfg.exposed_threshold_percent,
                was_obscured
            );
            WAS_DESKTOP_OBSCURED.store(next_was_obscured, Ordering::SeqCst);
            log_info!(
                ">>> [TRIGGER: REVEAL] Desktop transitioned from covered to exposed ({:.1}% >= {:.1}%)! Applying wallpaper...",
                current_exposure,
                cfg.exposed_threshold_percent
            );
            apply_or_await_wallpaper(&cfg);
        }
        (
            PendingSystemTrigger::None,
            ExposureDecision::Skip {
                next_was_obscured,
                cooldown_blocked,
            },
        ) => {
            WAS_DESKTOP_OBSCURED.store(next_was_obscured, Ordering::SeqCst);
            if current_exposure < cfg.obscured_threshold_percent {
                log_debug!(
                    "[DESKTOP COVERED] Windows: {} | Exposure: {:.1}% (< {:.1}% threshold). Armed for next reveal!",
                    visible_windows.len(),
                    current_exposure,
                    cfg.obscured_threshold_percent
                );
            } else if current_exposure >= cfg.exposed_threshold_percent {
                if cooldown_blocked {
                    log_debug!(
                        "[COOLDOWN] Window settled & desktop exposed, but cooldown ({}s) has not elapsed yet. Standby candidate preserved.",
                        effective_cooldown
                    );
                } else {
                    log_debug!(
                        "[EXPOSED (NO TRIGGER)] Desktop remains exposed ({:.1}% >= {:.1}%). Skipping shuffle (waiting for desktop to be covered < {:.1}% first).",
                        current_exposure,
                        cfg.exposed_threshold_percent,
                        cfg.obscured_threshold_percent
                    );
                }
            } else {
                log_debug!(
                    "[INTERMEDIATE] Windows: {} | Exposure: {:.1}% (Between {:.1}% and {:.1}%). No state transition.",
                    visible_windows.len(),
                    current_exposure,
                    cfg.obscured_threshold_percent,
                    cfg.exposed_threshold_percent
                );
            }
        }
    }
}

fn trigger_manual_shuffle() {
    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
    log_info!(">>> [MANUAL TRIGGER] Shuffle requested from tray or CLI.");
    apply_or_await_wallpaper(&cfg);
}

struct ApplyingGuard;

impl Drop for ApplyingGuard {
    fn drop(&mut self) {
        IS_APPLYING.store(false, Ordering::SeqCst);
    }
}

fn apply_or_await_wallpaper(cfg: &config::AppConfig) {
    if IS_APPLYING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        log_debug!(
            "[SKIP REENTRANT] Wallpaper application already in progress. Skipping concurrent apply."
        );
        return;
    }
    let _apply_guard = ApplyingGuard;

    // 1. Take candidate from standby cache if ready
    let mut candidate = STANDBY_WALLPAPER.lock().unwrap().take();

    // 2. If background pre-composition is still running, wait for completion (up to 2000ms)
    if candidate.is_none() && IS_COMPOSING.load(Ordering::SeqCst) {
        log_debug!(
            "[AWAIT PRECOMP] Background composition in progress. Awaiting completion for instant application..."
        );
        let start = Instant::now();
        while start.elapsed().as_millis() < 2000 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            candidate = STANDBY_WALLPAPER.lock().unwrap().take();
            if candidate.is_some() || !IS_COMPOSING.load(Ordering::SeqCst) {
                if candidate.is_none() {
                    candidate = STANDBY_WALLPAPER.lock().unwrap().take();
                }
                break;
            }
        }
    }

    // 3. Fallback: synchronously compose if standby was empty
    let candidate = match candidate {
        Some(c) => {
            log_info!(
                "[STANDBY HIT] Applying pre-composed {:?} wallpaper with ZERO wait time! ({})",
                c.mode,
                c.output_path.display()
            );
            c
        }
        None => {
            log_info!("[SYNC GENERATION] No standby image ready, synthesizing synchronously...");
            let history_dir = get_history_dir();
            match composer::execute_shuffle_and_compose(
                &cfg.wallpaper_dirs,
                &history_dir,
                cfg.history_count,
            ) {
                Ok(c) => {
                    log_info!(
                        "[SYNC READY] Composed {:?} wallpaper in {}ms",
                        c.mode,
                        c.duration_ms
                    );
                    c
                }
                Err(e) => {
                    log_error!("[ERROR] Fallback composition failed: {}", e);
                    return;
                }
            }
        }
    };

    if let Err(e) = wallpaper::set_wallpaper(&candidate.output_path) {
        log_error!("[ERROR] Failed to set wallpaper: {}", e);
    } else {
        log_info!(
            "[WALLPAPER APPLIED] Successfully updated Windows wallpaper! (Used {} images)",
            candidate.source_images.len()
        );
        *LAST_SHUFFLE_TIME.lock().unwrap() = Some(chrono::Utc::now());
        // Update tray icon and taskbar mini UI with the newly applied wallpaper
        if let Ok(img) = image::open(&candidate.output_path) {
            let rgba = img.to_rgba8();
            tray::update_tray_icon(Some(&rgba));
            taskbar::update_taskbar_wallpaper(Some(&rgba));
        }
        tray::update_tray_tooltip(&format!("Wallpaper Shuffler ({:?})", candidate.mode));
        taskbar::set_taskbar_status(&format!("{:?}", candidate.mode));
        // Speculatively pre-compose the next candidate in advance so future reveals are instant
        ensure_candidate_precomposed_async(cfg);
    }
}

fn apply_display_mode(mode: config::DisplayMode) {
    match mode {
        config::DisplayMode::Taskbar => {
            if taskbar::set_taskbar_visible(true) {
                tray::set_tray_visible(false);
            } else {
                log_warn!(
                    "Taskbar Mini UI unavailable (e.g., vertical taskbar); falling back to tray icon."
                );
                tray::set_tray_visible(true);
            }
        }
        config::DisplayMode::Tray => {
            let _ = taskbar::set_taskbar_visible(false);
            tray::set_tray_visible(true);
        }
    }
}

fn update_display_mode(mode: config::DisplayMode) {
    log_info!("Display mode changed to {:?}", mode);
    apply_display_mode(mode);
    let config_path = get_config_path();
    let mut guard = APP_CONFIG.lock().unwrap();
    if let Some(cfg) = guard.as_mut() {
        cfg.display_mode = mode;
        let _ = cfg.save_to_file(&config_path);
    }
}

fn update_active_log_level(level: LogLevel) {
    logger::set_log_level(level);
    log_info!("Log level changed to {:?}", level);
    let config_path = get_config_path();
    let mut guard = APP_CONFIG.lock().unwrap();
    if let Some(cfg) = guard.as_mut() {
        cfg.log_level = level.as_str().to_lowercase();
        let _ = cfg.save_to_file(&config_path);
    }
}

fn get_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("USERPROFILE") {
        let user_cfg = PathBuf::from(&p)
            .join(".wallpaper-shuffler")
            .join("config.json");
        if user_cfg.exists() {
            return user_cfg;
        }
    }
    if let Ok(p) = std::env::var("APPDATA") {
        let appdata_cfg = PathBuf::from(&p)
            .join("wallpaper-shuffler")
            .join("config.json");
        if appdata_cfg.exists() {
            return appdata_cfg;
        }
    }
    std::env::var("USERPROFILE")
        .map(|p| {
            PathBuf::from(p)
                .join(".wallpaper-shuffler")
                .join("config.json")
        })
        .unwrap_or_else(|_| PathBuf::from("config.json"))
}

fn run_daemon(
    foreground: bool,
    override_dir: Option<PathBuf>,
    log_level_override: Option<LogLevel>,
) {
    let config_path = get_config_path();
    let mut cfg = config::AppConfig::load_from_file(&config_path).unwrap_or_else(|_| {
        let default_cfg = config::AppConfig::default();
        let _ = default_cfg.save_to_file(&config_path);
        default_cfg
    });

    if let Some(custom_dir) = override_dir {
        cfg.wallpaper_dirs = vec![custom_dir];
    }

    let active_log_level = log_level_override.unwrap_or_else(|| LogLevel::parse(&cfg.log_level));
    logger::init(foreground, active_log_level);

    if foreground {
        attach_parent_console();
        println!("=== wallpaper-shuffler foreground daemon started ===");
        println!(
            "Listening for window movements, minimizes (Win+D), and desktop exposure changes..."
        );
        println!("Press Ctrl+C to stop.\n");

        println!("Configuration:");
        println!("  Source directories: {:?}", cfg.wallpaper_dirs);
        println!("  History directory:  {}", get_history_dir().display());
        println!(
            "  Log directory:      {}",
            logger::default_log_dir().display()
        );
        println!(
            "  Thresholds:         hidden < {:.1}%, exposed >= {:.1}%",
            cfg.obscured_threshold_percent, cfg.exposed_threshold_percent
        );
        println!("  Cooldown:           {} seconds", cfg.cooldown_seconds);
        println!("  Debounce:           {} ms", cfg.debounce_millis);
        println!("  Log level:          {:?}\n", active_log_level);
    } else {
        log_info!("=== wallpaper-shuffler background daemon started ===");
        log_info!("Log level: {:?}", active_log_level);
        log_info!("Source directories: {:?}", cfg.wallpaper_dirs);
        log_info!("History directory:  {}", get_history_dir().display());
        log_info!(
            "Log directory:      {}",
            logger::default_log_dir().display()
        );
    }

    LAST_DEBOUNCE_MS.store(cfg.debounce_millis, Ordering::SeqCst);
    *APP_CONFIG.lock().unwrap() = Some(cfg.clone());

    // Pre-compose 1 standby wallpaper immediately on daemon startup so the very first trigger is instant (0ms latency)
    ensure_candidate_precomposed_async(&cfg);

    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class_name = w!("WallpaperShufflerClass");

        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            w!("WallpaperShufflerWindow"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        );

        if hwnd.is_err() {
            log_error!("Failed to create message window");
            return;
        }
        let hwnd = hwnd.unwrap();
        TARGET_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

        // Load most recent wallpaper from history if present to display initial framed tray icon
        let initial_thumb = if let Ok(entries) = std::fs::read_dir(get_history_dir()) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("png")
                    })
                })
                .collect();
            files.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
            files
                .last()
                .and_then(|p| image::open(p).ok().map(|img| img.to_rgba8()))
        } else {
            None
        };
        let show_tray_initially = cfg.display_mode == config::DisplayMode::Tray;
        tray::init_tray(hwnd, initial_thumb.as_ref(), show_tray_initially);
        taskbar::init_taskbar(hwnd, initial_thumb.as_ref());
        apply_display_mode(cfg.display_mode);
        let _hook_guard = hook::install_window_event_hook(win_event_hook_proc);

        // Start lightweight safety watch timer (250ms) to guarantee 100% window tracking reliability
        let _ = SetTimer(Some(hwnd), TIMER_WATCH_ID, 250, None);

        let visible_windows = detector::collect_visible_window_rects();
        let initial_exposure = detector::calculate_desktop_exposure_rate();
        *PREVIOUS_EXPOSURE.lock().unwrap() = Some(initial_exposure);
        *LAST_WATCH_STATE.lock().unwrap() = Some((visible_windows.len(), initial_exposure));

        log_info!(
            "[INITIAL] Recognized {} visible window(s). Desktop exposure rate: {:.1}%",
            visible_windows.len(),
            initial_exposure
        );
        if _hook_guard.is_none() {
            log_error!("[ERROR] Failed to install SetWinEventHook! Check privileges.");
        } else {
            log_info!(
                "[READY] SetWinEventHook active (monitoring move, resize, minimize, foreground)."
            );
        }

        // Schedule initial startup check through debounce timer so startup windows can settle
        // and exposure is verified before deciding whether to shuffle.
        PENDING_SYSTEM_TRIGGER.store(PendingSystemTrigger::Startup as u8, Ordering::SeqCst);
        let startup_settle_ms = cfg.debounce_millis.max(50) as u32;
        log_info!(
            "[STARTUP] Scheduling initial desktop exposure check after {}ms settle...",
            startup_settle_ms
        );
        let _ = SetTimer(Some(hwnd), TIMER_DEBOUNCE_ID, startup_settle_ms, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn print_help() {
    attach_parent_console();
    println!("wallpaper-shuffler v{}", env!("CARGO_PKG_VERSION"));
    println!("Lightweight event-driven Windows wallpaper shuffler\n");
    println!("Usage: wallpaper-shuffler [COMMAND/OPTION] [ARGS...]\n");
    println!("Image Generation & Testing Commands:");
    println!(
        "  --shuffle-dry-run <DIR> [OUT_DIR]   Test full auto-shuffle pipeline on DIR (no wallpaper change)"
    );
    println!("  --compose-collage <DIR> [OUT_FILE]  Generate a 4K Collage from DIR to OUT_FILE");
    println!("  --compose-tiling  <DIR> [OUT_FILE]  Generate a 4K Tiling from DIR to OUT_FILE");
    println!("  --compose-single  <DIR> [OUT_FILE]  Generate a 4K Single Fit from DIR to OUT_FILE");
    println!(
        "  --test-compose    <DIR> [OUT_DIR]   Generate all 3 sample styles (Single, Collage, Tiling)"
    );
    println!(
        "  --benchmark       <DIR>             Benchmark performance on DIR across all styles"
    );
    println!(
        "  --generate-fixtures [DIR]           Generate 30 synthetic 2K-4K test images (default: tests/fixtures/images)\n"
    );
    println!("Resident Daemon & System Commands:");
    println!(
        "  --run-foreground [DIR] [--debug]    Run daemon in console with logs (default: info, --debug: verbose)"
    );
    println!(
        "  --log-level <debug|info|warn|error> Set log verbosity level (can also be configured in config.json)"
    );
    println!(
        "  --register-startup                  Register application to start automatically on Windows boot"
    );
    println!("  --unregister-startup                Remove application from Windows startup");
    println!("  --help, -h                          Print this help message");
    println!("  --version, -v                       Print version information");
}

fn handle_generate_fixtures(args: &[String]) {
    attach_parent_console();
    let target_dir = if args.len() > 2 {
        PathBuf::from(&args[2])
    } else {
        PathBuf::from("tests").join("fixtures").join("images")
    };

    println!(
        "Generating 30 test fixtures into {} ...",
        target_dir.display()
    );
    let start = Instant::now();
    match fixtures::generate_all_fixtures(&target_dir) {
        Ok(paths) => {
            println!(
                "Successfully generated {} test images in {:.2}s!",
                paths.len(),
                start.elapsed().as_secs_f32()
            );
        }
        Err(e) => eprintln!("Error generating fixtures: {}", e),
    }
}

fn handle_shuffle_dry_run(args: &[String]) {
    attach_parent_console();
    if args.len() < 3 {
        eprintln!("Usage: wallpaper-shuffler --shuffle-dry-run <IMAGE_DIR> [OUT_DIR]");
        return;
    }
    let in_dir = PathBuf::from(&args[2]);
    let out_dir = if args.len() > 3 {
        PathBuf::from(&args[3])
    } else {
        PathBuf::from("target").join("test_output")
    };

    println!("=== Wallpaper Shuffle Dry Run (No OS wallpaper change) ===");
    println!("Source Directory: {}", in_dir.display());
    println!("Output Directory: {}\n", out_dir.display());

    match composer::execute_shuffle_and_compose(&[in_dir], &out_dir, 10) {
        Ok(res) => {
            println!(">>> Composition Successful!");
            println!("  Selected Mode:   {:?}", res.mode);
            println!("  Processing Time: {}ms", res.duration_ms);
            println!("  Saved Image:     {}", res.output_path.display());
            println!("  Sources Used:    {} image(s)", res.source_images.len());
            for (i, p) in res.source_images.iter().enumerate() {
                println!(
                    "    [{}] {}",
                    i + 1,
                    p.file_name().and_then(|s| s.to_str()).unwrap_or("")
                );
            }
            println!("\nYou can inspect the generated 4K wallpaper at:");
            println!(
                "file:///{}",
                res.output_path
                    .canonicalize()
                    .unwrap_or(res.output_path)
                    .display()
            );
        }
        Err(e) => eprintln!("Error during shuffle dry-run: {}", e),
    }
}

fn handle_single_mode(mode: composer::CompositionMode, args: &[String], default_filename: &str) {
    attach_parent_console();
    if args.len() < 3 {
        eprintln!(
            "Usage: wallpaper-shuffler --compose-{:?} <IMAGE_DIR> [OUT_FILE]",
            mode
        );
        return;
    }
    let in_dir = PathBuf::from(&args[2]);
    let out_file = if args.len() > 3 {
        PathBuf::from(&args[3])
    } else {
        PathBuf::from("target")
            .join("test_output")
            .join(default_filename)
    };

    println!("=== Compose Mode: {:?} ===", mode);
    println!("Source: {}", in_dir.display());
    println!("Output: {}\n", out_file.display());

    let images = composer::collect_image_files(&[in_dir]);
    if images.is_empty() {
        eprintln!("No valid image files found in specified directory.");
        return;
    }

    match composer::execute_mode_explicit(mode, &images, &out_file) {
        Ok(res) => {
            println!(
                ">>> Successfully composed {:?} in {}ms!",
                mode, res.duration_ms
            );
            println!("Saved to: {}", res.output_path.display());
            println!(
                "file:///{}",
                res.output_path
                    .canonicalize()
                    .unwrap_or(res.output_path)
                    .display()
            );
        }
        Err(e) => eprintln!("Failed to compose {:?}: {}", mode, e),
    }
}

fn handle_test_compose(args: &[String]) {
    attach_parent_console();
    let in_dir = if args.len() > 2 {
        PathBuf::from(&args[2])
    } else {
        PathBuf::from("tests").join("fixtures").join("images")
    };

    let out_dir = if args.len() > 3 {
        PathBuf::from(&args[3])
    } else {
        PathBuf::from("target").join("test_output")
    };

    println!("=== Testing Composition Modes ===");
    println!("Input Directory:  {}", in_dir.display());
    println!("Output Directory: {}\n", out_dir.display());

    let images = composer::collect_image_files(&[in_dir]);
    if images.is_empty() {
        eprintln!("No images found. Please run with --generate-fixtures first.");
        return;
    }

    // 1. Single Mode Test
    println!("[1/3] Testing Single Image Fit Mode (4K 3840x2160)...");
    let out_single = out_dir.join("01_sample_single.jpg");
    match composer::execute_mode_explicit(composer::CompositionMode::Single, &images, &out_single) {
        Ok(res) => println!(
            "  -> Generated {} in {}ms (used {})",
            res.output_path.display(),
            res.duration_ms,
            res.source_images
                .first()
                .and_then(|p| p.file_name())
                .unwrap_or_default()
                .to_string_lossy()
        ),
        Err(e) => println!("  -> Skipped: {}", e),
    }

    // 2. Collage Mode Test
    println!("[2/3] Testing Collage Mode (Gallery Cards)...");
    let out_collage = out_dir.join("02_sample_collage.jpg");
    match composer::execute_mode_explicit(composer::CompositionMode::Collage, &images, &out_collage)
    {
        Ok(res) => println!(
            "  -> Generated {} in {}ms (used {} images)",
            res.output_path.display(),
            res.duration_ms,
            res.source_images.len()
        ),
        Err(e) => eprintln!("  -> Failed: {}", e),
    }

    // 3. Tiling Mode Test
    println!("[3/3] Testing Dynamic Irregular Tiling Mode (Treemap / Bento)...");
    let out_tiling = out_dir.join("03_sample_tiling.jpg");
    match composer::execute_mode_explicit(composer::CompositionMode::Tiling, &images, &out_tiling) {
        Ok(res) => println!(
            "  -> Generated {} in {}ms (used {} images)",
            res.output_path.display(),
            res.duration_ms,
            res.source_images.len()
        ),
        Err(e) => eprintln!("  -> Failed: {}", e),
    }

    println!("\nAll sample images generated! Please visually inspect the results in:");
    println!(
        "file:///{}",
        out_dir.canonicalize().unwrap_or(out_dir).display()
    );
}

fn handle_benchmark(args: &[String]) {
    attach_parent_console();
    let in_dir = if args.len() > 2 {
        PathBuf::from(&args[2])
    } else {
        PathBuf::from("tests").join("fixtures").join("images")
    };

    println!("=== Benchmarking 4K Wallpaper Composition Modes ===");
    println!("Source: {}\n", in_dir.display());

    let images = composer::collect_image_files(&[in_dir]);
    if images.is_empty() {
        eprintln!("No images found. Please run with --generate-fixtures first.");
        return;
    }

    let temp_out = PathBuf::from("target").join("benchmark_temp.jpg");
    let iterations = 5;

    // Benchmark Single
    if let Some(single_cand) = images.iter().find(|p| composer::is_single_candidate(p)) {
        let mut single_times = Vec::new();
        for _ in 0..iterations {
            let res = composer::execute_mode_explicit(
                composer::CompositionMode::Single,
                std::slice::from_ref(single_cand),
                &temp_out,
            )
            .expect("benchmark single");
            single_times.push(res.duration_ms);
        }
        let avg_single = single_times.iter().sum::<u128>() as f64 / iterations as f64;
        println!(
            "Single 4K Fit:   avg {:.1}ms (runs: {:?})",
            avg_single, single_times
        );
    } else {
        println!(
            "Single Fit:      skipped (no landscape images >= {}px)",
            composer::MIN_SINGLE_LANDSCAPE_WIDTH
        );
    }

    // Benchmark Collage
    let mut collage_times = Vec::new();
    for _ in 0..iterations {
        let res =
            composer::execute_mode_explicit(composer::CompositionMode::Collage, &images, &temp_out)
                .expect("benchmark collage");
        collage_times.push(res.duration_ms);
    }
    let avg_collage = collage_times.iter().sum::<u128>() as f64 / iterations as f64;
    println!(
        "Collage (6 imgs): avg {:.1}ms (runs: {:?})",
        avg_collage, collage_times
    );

    // Benchmark Tiling
    let mut tiling_times = Vec::new();
    for _ in 0..iterations {
        let res =
            composer::execute_mode_explicit(composer::CompositionMode::Tiling, &images, &temp_out)
                .expect("benchmark tiling");
        tiling_times.push(res.duration_ms);
    }
    let avg_tiling = tiling_times.iter().sum::<u128>() as f64 / iterations as f64;
    println!(
        "Dynamic Tiling:  avg {:.1}ms (runs: {:?})",
        avg_tiling, tiling_times
    );

    let _ = std::fs::remove_file(&temp_out);
    println!("\nBenchmark completed.");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--shuffle-dry-run" => {
                handle_shuffle_dry_run(&args);
                return;
            }
            "--compose-collage" => {
                handle_single_mode(
                    composer::CompositionMode::Collage,
                    &args,
                    "collage_custom.jpg",
                );
                return;
            }
            "--compose-tiling" => {
                handle_single_mode(
                    composer::CompositionMode::Tiling,
                    &args,
                    "tiling_custom.jpg",
                );
                return;
            }
            "--compose-single" => {
                handle_single_mode(
                    composer::CompositionMode::Single,
                    &args,
                    "single_custom.jpg",
                );
                return;
            }
            "--generate-fixtures" => {
                handle_generate_fixtures(&args);
                return;
            }
            "--test-compose" => {
                handle_test_compose(&args);
                return;
            }
            "--benchmark" => {
                handle_benchmark(&args);
                return;
            }
            "--run-foreground" => {
                let mut override_dir = None;
                let mut override_log_level = None;
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--debug" => {
                            override_log_level = Some(LogLevel::Debug);
                        }
                        "--log-level" => {
                            if i + 1 < args.len() {
                                i += 1;
                                override_log_level = Some(LogLevel::parse(&args[i]));
                            }
                        }
                        other if !other.starts_with("--") && override_dir.is_none() => {
                            override_dir = Some(PathBuf::from(other));
                        }
                        _ => {}
                    }
                    i += 1;
                }
                run_daemon(true, override_dir, override_log_level);
                return;
            }
            "--register-startup" => {
                attach_parent_console();
                if let Ok(exe_path) = env::current_exe() {
                    match startup::register_startup(exe_path.as_os_str()) {
                        Ok(()) => println!(
                            "Successfully registered wallpaper-shuffler in Windows startup."
                        ),
                        Err(e) => eprintln!("Error registering startup: {}", e),
                    }
                }
                return;
            }
            "--unregister-startup" => {
                attach_parent_console();
                match startup::unregister_startup() {
                    Ok(()) => println!(
                        "Successfully unregistered wallpaper-shuffler from Windows startup."
                    ),
                    Err(e) => eprintln!("Error unregistering startup: {}", e),
                }
                return;
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            "--version" | "-v" => {
                attach_parent_console();
                println!("wallpaper-shuffler v{}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {}
        }
    }

    run_daemon(false, None, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resume_skips_when_covered_and_arms_for_next_reveal() {
        // Fully covered (0.0% < 15.0%): must NOT shuffle, must arm was_obscured = true
        let dec_zero = decide_shuffle_on_exposure(
            PendingSystemTrigger::PowerResume,
            0.0,
            false,
            true,
            5.0,
            15.0,
        );
        assert_eq!(
            dec_zero,
            ExposureDecision::Skip {
                next_was_obscured: true,
                cooldown_blocked: false,
            }
        );

        // Intermediate coverage (10.0% < 15.0%): still not exposed, must NOT shuffle, must arm
        let dec_mid = decide_shuffle_on_exposure(
            PendingSystemTrigger::PowerResume,
            10.0,
            false,
            true,
            5.0,
            15.0,
        );
        assert_eq!(
            dec_mid,
            ExposureDecision::Skip {
                next_was_obscured: true,
                cooldown_blocked: false,
            }
        );

        // Subsequent reveal (25.0% >= 15.0%) immediately triggers since cooldown was preserved!
        let dec_reveal =
            decide_shuffle_on_exposure(PendingSystemTrigger::None, 25.0, true, true, 5.0, 15.0);
        assert_eq!(
            dec_reveal,
            ExposureDecision::TriggerShuffle {
                next_was_obscured: false,
            }
        );
    }

    #[test]
    fn test_resume_triggers_when_exposed_and_deduplicates_within_cooldown() {
        // Desktop exposed on resume (60.0% >= 15.0%) with cooldown elapsed -> triggers shuffle
        let first = decide_shuffle_on_exposure(
            PendingSystemTrigger::PowerResume,
            60.0,
            false,
            true,
            5.0,
            15.0,
        );
        assert_eq!(
            first,
            ExposureDecision::TriggerShuffle {
                next_was_obscured: false,
            }
        );

        // Second PBT_APMRESUMESUSPEND arriving right after shuffle (cooldown_elapsed = false, was_obscured = false)
        // must skip and must NOT re-arm was_obscured!
        let second = decide_shuffle_on_exposure(
            PendingSystemTrigger::PowerResume,
            60.0,
            false,
            false,
            5.0,
            15.0,
        );
        assert_eq!(
            second,
            ExposureDecision::Skip {
                next_was_obscured: false,
                cooldown_blocked: true,
            }
        );
    }

    #[test]
    fn test_startup_checks_exposure_before_shuffling() {
        // Covered on startup -> skip and arm
        assert_eq!(
            decide_shuffle_on_exposure(PendingSystemTrigger::Startup, 3.7, true, true, 5.0, 15.0),
            ExposureDecision::Skip {
                next_was_obscured: true,
                cooldown_blocked: false,
            }
        );

        // Exposed on startup -> trigger shuffle and disarm
        assert_eq!(
            decide_shuffle_on_exposure(PendingSystemTrigger::Startup, 40.0, true, true, 5.0, 15.0),
            ExposureDecision::TriggerShuffle {
                next_was_obscured: false,
            }
        );
    }

    #[test]
    fn test_normal_hysteresis_transition() {
        // Covered (< 5%) -> arms
        assert_eq!(
            decide_shuffle_on_exposure(PendingSystemTrigger::None, 2.0, false, true, 5.0, 15.0),
            ExposureDecision::Skip {
                next_was_obscured: true,
                cooldown_blocked: false,
            }
        );

        // Intermediate (10%) preserves existing armed state
        assert_eq!(
            decide_shuffle_on_exposure(PendingSystemTrigger::None, 10.0, false, true, 5.0, 15.0),
            ExposureDecision::Skip {
                next_was_obscured: false,
                cooldown_blocked: false,
            }
        );
        assert_eq!(
            decide_shuffle_on_exposure(PendingSystemTrigger::None, 10.0, true, true, 5.0, 15.0),
            ExposureDecision::Skip {
                next_was_obscured: true,
                cooldown_blocked: false,
            }
        );

        // Exposed (>= 15%) while disarmed -> skip
        assert_eq!(
            decide_shuffle_on_exposure(PendingSystemTrigger::None, 30.0, false, true, 5.0, 15.0),
            ExposureDecision::Skip {
                next_was_obscured: false,
                cooldown_blocked: false,
            }
        );
    }
}
