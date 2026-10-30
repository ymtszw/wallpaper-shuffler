#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

mod composer;
mod config;
mod detector;
mod fixtures;
mod hook;
mod startup;
mod tray;
mod wallpaper;

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
static FOREGROUND_LOGGING: AtomicBool = AtomicBool::new(false);
static IS_COMPOSING: AtomicBool = AtomicBool::new(false);
static STANDBY_WALLPAPER: std::sync::Mutex<Option<composer::CompositionResult>> =
    std::sync::Mutex::new(None);

const TIMER_DEBOUNCE_ID: usize = 9001;
const TIMER_WATCH_ID: usize = 9002;

static LAST_WATCH_STATE: std::sync::Mutex<Option<(usize, f64)>> = std::sync::Mutex::new(None);

fn on_window_activity_detected(source: &str) {
    let target = TARGET_HWND.load(Ordering::Relaxed);
    if target == 0 {
        return;
    }

    if FOREGROUND_LOGGING.load(Ordering::Relaxed) {
        println!(
            "[{}] Window movement/resize/minimize detected! Pre-composing next wallpaper...",
            source
        );
    }

    let debounce_ms = LAST_DEBOUNCE_MS.load(Ordering::Relaxed) as u32;
    unsafe {
        let _ = SetTimer(
            Some(HWND(target as *mut _)),
            TIMER_DEBOUNCE_ID,
            debounce_ms,
            None,
        );
    }

    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
    ensure_candidate_precomposed_async(&cfg, FOREGROUND_LOGGING.load(Ordering::Relaxed));
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
fn ensure_candidate_precomposed_async(cfg: &config::AppConfig, log: bool) {
    // 1. Check if an unconsumed standby candidate is already ready
    {
        let guard = STANDBY_WALLPAPER.lock().unwrap();
        if guard.is_some() {
            if log {
                println!(
                    "[STANDBY CACHE] Ready candidate already standby. Skipping duplicate composition."
                );
            }
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

    if log {
        println!(
            ">>> [SPECULATIVE PRECOMP] Window activity detected. Generating next wallpaper candidate in background thread..."
        );
    }

    std::thread::spawn(move || {
        let res =
            composer::execute_shuffle_and_compose(&wallpaper_dirs, &history_dir, history_count);
        match res {
            Ok(comp_res) => {
                let log = FOREGROUND_LOGGING.load(Ordering::Relaxed);
                if log {
                    println!(
                        "[PRECOMP READY] Next candidate ({:?}) prepared in {}ms! Stored in standby cache: {}",
                        comp_res.mode,
                        comp_res.duration_ms,
                        comp_res.output_path.display()
                    );
                }
                *STANDBY_WALLPAPER.lock().unwrap() = Some(comp_res);
            }
            Err(e) => {
                let log = FOREGROUND_LOGGING.load(Ordering::Relaxed);
                if log {
                    eprintln!("[PRECOMP ERROR] Background composition failed: {}", e);
                }
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
        0x800B => "LOCATION_CHANGE",
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

    // Only process window-level events (OBJID_WINDOW = 0, OBJID_CLIENT = -4)
    if id_object != 0 && id_object != -4 {
        return;
    }

    // Ignore events originating from our own resident daemon window
    if hwnd.0 == target as *mut _ {
        return;
    }

    on_window_activity_detected(&format!("HOOK: {}", event_name(event)));
}

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
                return LRESULT(0);
            }
        }
        tray::WM_TRAYICON => {
            let event = (lparam.0 as usize & 0xFFFF) as u32;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                tray::show_tray_context_menu(hwnd);
            }
        }
        WM_COMMAND => match wparam.0 {
            tray::IDM_SHUFFLE_NOW => {
                trigger_manual_shuffle();
            }
            tray::IDM_EXIT => unsafe {
                PostQuitMessage(0);
            },
            _ => {}
        },
        WM_DESTROY => {
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

fn on_debounce_settled() {
    let visible_windows = detector::collect_visible_window_rects();
    let current_exposure = detector::calculate_desktop_exposure_rate();
    let is_foreground = FOREGROUND_LOGGING.load(Ordering::Relaxed);

    let mut prev_guard = PREVIOUS_EXPOSURE.lock().unwrap();
    let prev_exposure = prev_guard.unwrap_or(current_exposure);
    *prev_guard = Some(current_exposure);

    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();

    if is_foreground {
        println!(
            "[DEBOUNCE SETTLED] Windows: {} | Exposure: {:.1}% (Prev: {:.1}%, Target: hidden < {:.1}%, exposed >= {:.1}%)",
            visible_windows.len(),
            current_exposure,
            prev_exposure,
            cfg.obscured_threshold_percent,
            cfg.exposed_threshold_percent
        );
    }

    // Trigger condition:
    // 1. Classical transition: hidden (< 10%) -> exposed (>= 20%)
    // 2. Window rearrangement / resizing / moving while desktop is exposed (>= 20%)
    let is_desktop_exposed = current_exposure >= cfg.exposed_threshold_percent;
    let is_transition = prev_exposure < cfg.obscured_threshold_percent && is_desktop_exposed;
    let is_rearranged = is_desktop_exposed; // Window moved or resized while desktop is visible

    let should_trigger = is_transition || is_rearranged;

    if should_trigger {
        let last_time = *LAST_SHUFFLE_TIME.lock().unwrap();
        let effective_cooldown = if is_foreground {
            5 // Short 5-second cooldown during foreground testing for instant responsive feedback
        } else {
            cfg.cooldown_seconds
        };

        if detector::has_cooldown_elapsed(last_time, effective_cooldown) {
            if is_foreground {
                let reason = if is_transition {
                    format!(
                        "Hidden-to-exposed transition ({:.1}% -> {:.1}%)",
                        prev_exposure, current_exposure
                    )
                } else {
                    format!(
                        "Window moved/resized while desktop visible ({:.1}%)",
                        current_exposure
                    )
                };
                println!(
                    ">>> [TRIGGER: {}] Window settled & cooldown passed! Applying wallpaper...",
                    reason
                );
            }
            apply_or_await_wallpaper(&cfg, is_foreground);
        } else if is_foreground {
            println!(
                "[COOLDOWN] Window settled, but cooldown ({}s) has not elapsed yet. Standby candidate preserved.",
                effective_cooldown
            );
        }
    } else if is_foreground {
        let has_standby = STANDBY_WALLPAPER.lock().unwrap().is_some();
        if has_standby {
            println!(
                "[STANDBY RETAINED] Desktop covered ({:.1}% < {:.1}%). Standby wallpaper held in memory for next reveal.",
                current_exposure, cfg.exposed_threshold_percent
            );
        }
    }
}

fn trigger_manual_shuffle() {
    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();
    let is_foreground = FOREGROUND_LOGGING.load(Ordering::Relaxed);
    if is_foreground {
        println!(">>> [MANUAL TRIGGER] Shuffle requested from tray or CLI.");
    }
    apply_or_await_wallpaper(&cfg, is_foreground);
}

fn apply_or_await_wallpaper(cfg: &config::AppConfig, log: bool) {
    // 1. Take candidate from standby cache if ready
    let mut candidate = STANDBY_WALLPAPER.lock().unwrap().take();

    // 2. If background pre-composition is still running, wait for completion (up to 2000ms)
    if candidate.is_none() && IS_COMPOSING.load(Ordering::SeqCst) {
        if log {
            println!(
                "[AWAIT PRECOMP] Background composition in progress. Awaiting completion for instant application..."
            );
        }
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
            if log {
                println!(
                    "[STANDBY HIT] Applying pre-composed {:?} wallpaper with ZERO wait time! ({})",
                    c.mode,
                    c.output_path.display()
                );
            }
            c
        }
        None => {
            if log {
                println!("[SYNC FALLBACK] No standby image ready, synthesizing synchronously...");
            }
            let history_dir = get_history_dir();
            match composer::execute_shuffle_and_compose(
                &cfg.wallpaper_dirs,
                &history_dir,
                cfg.history_count,
            ) {
                Ok(c) => c,
                Err(e) => {
                    if log {
                        eprintln!("[ERROR] Fallback composition failed: {}", e);
                    }
                    return;
                }
            }
        }
    };

    if let Err(e) = wallpaper::set_wallpaper(&candidate.output_path) {
        if log {
            eprintln!("[ERROR] Failed to set wallpaper: {}", e);
        }
    } else {
        if log {
            println!(
                "[WALLPAPER APPLIED] Successfully updated Windows wallpaper! (Used {} images)",
                candidate.source_images.len()
            );
        }
        *LAST_SHUFFLE_TIME.lock().unwrap() = Some(chrono::Utc::now());
    }
}

fn get_config_path() -> PathBuf {
    std::env::var("APPDATA")
        .map(|p| {
            PathBuf::from(p)
                .join("wallpaper-shuffler")
                .join("config.json")
        })
        .unwrap_or_else(|_| PathBuf::from("config.json"))
}

fn run_daemon(foreground: bool, override_dir: Option<PathBuf>) {
    if foreground {
        attach_parent_console();
        FOREGROUND_LOGGING.store(true, Ordering::SeqCst);
        println!("=== wallpaper-shuffler foreground daemon started ===");
        println!(
            "Listening for window movements, minimizes (Win+D), and desktop exposure changes..."
        );
        println!("Press Ctrl+C to stop.\n");
    }

    let config_path = get_config_path();
    let mut cfg = config::AppConfig::load_from_file(&config_path).unwrap_or_else(|_| {
        let default_cfg = config::AppConfig::default();
        let _ = default_cfg.save_to_file(&config_path);
        default_cfg
    });

    if let Some(custom_dir) = override_dir {
        cfg.wallpaper_dirs = vec![custom_dir];
    }

    if foreground {
        println!("Configuration:");
        println!("  Source directories: {:?}", cfg.wallpaper_dirs);
        println!("  History directory:  {}", get_history_dir().display());
        println!(
            "  Thresholds:         hidden < {:.1}%, exposed >= {:.1}%",
            cfg.obscured_threshold_percent, cfg.exposed_threshold_percent
        );
        println!("  Cooldown:           {} seconds", cfg.cooldown_seconds);
        println!("  Debounce:           {} ms\n", cfg.debounce_millis);
    }

    LAST_DEBOUNCE_MS.store(cfg.debounce_millis, Ordering::SeqCst);
    *APP_CONFIG.lock().unwrap() = Some(cfg.clone());

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
            eprintln!("Failed to create message window");
            return;
        }
        let hwnd = hwnd.unwrap();
        TARGET_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

        let _tray = tray::TrayManager::new(hwnd);
        let _hook_guard = hook::install_window_event_hook(win_event_hook_proc);

        // Start lightweight safety watch timer (250ms) to guarantee 100% window tracking reliability
        let _ = SetTimer(Some(hwnd), TIMER_WATCH_ID, 250, None);

        let visible_windows = detector::collect_visible_window_rects();
        let initial_exposure = detector::calculate_desktop_exposure_rate();
        *PREVIOUS_EXPOSURE.lock().unwrap() = Some(initial_exposure);
        if foreground {
            println!(
                "[INITIAL] Recognized {} visible window(s). Desktop exposure rate: {:.1}%",
                visible_windows.len(),
                initial_exposure
            );
            if _hook_guard.is_none() {
                eprintln!("[ERROR] Failed to install SetWinEventHook! Check privileges.");
            } else {
                println!(
                    "[READY] SetWinEventHook active (monitoring move, resize, minimize, foreground)."
                );
            }
        }

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
        "  --run-foreground [DIR]              Run daemon in console with real-time logs (optional: override wallpaper DIR)"
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
                let override_dir = if args.len() > 2 {
                    Some(PathBuf::from(&args[2]))
                } else {
                    None
                };
                run_daemon(true, override_dir);
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

    run_daemon(false, None);
}
