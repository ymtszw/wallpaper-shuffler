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
    PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, WM_COMMAND, WM_DESTROY, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);
static LAST_DEBOUNCE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(400);
static FOREGROUND_LOGGING: AtomicBool = AtomicBool::new(false);

const TIMER_DEBOUNCE_ID: usize = 9001;

fn attach_parent_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

unsafe extern "system" fn win_event_hook_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    let target = TARGET_HWND.load(Ordering::Relaxed);
    if target == 0 {
        return;
    }

    if detector::is_window_valid_and_visible(hwnd) {
        if FOREGROUND_LOGGING.load(Ordering::Relaxed) {
            println!(
                "[EVENT 0x{:04X}] Window activity detected on HWND {:?}. Resetting debounce timer...",
                event, hwnd.0
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
    }
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
            }
        }
        tray::WM_TRAYICON => {
            // Context menu handling will be dispatched here
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
    let current_exposure = detector::calculate_desktop_exposure_rate();
    let is_foreground = FOREGROUND_LOGGING.load(Ordering::Relaxed);

    let mut prev_guard = PREVIOUS_EXPOSURE.lock().unwrap();
    let prev_exposure = prev_guard.unwrap_or(current_exposure);
    *prev_guard = Some(current_exposure);

    let cfg = APP_CONFIG.lock().unwrap().clone().unwrap_or_default();

    if is_foreground {
        println!(
            "[DEBOUNCE SETTLED] Desktop exposure: {:.1}% (Previous: {:.1}%, Thresholds: hidden < {:.1}%, visible >= {:.1}%)",
            current_exposure,
            prev_exposure,
            cfg.obscured_threshold_percent,
            cfg.exposed_threshold_percent
        );
    }

    let transitioned = prev_exposure < cfg.obscured_threshold_percent
        && current_exposure >= cfg.exposed_threshold_percent;

    if transitioned {
        let last_time = *LAST_SHUFFLE_TIME.lock().unwrap();
        if detector::has_cooldown_elapsed(last_time, cfg.cooldown_seconds) {
            if is_foreground {
                println!(
                    ">>> [TRIGGER] Exposure transition detected and cooldown passed! Shuffling wallpaper..."
                );
            }
            perform_shuffle(&cfg, is_foreground);
        } else if is_foreground {
            println!(
                "[COOLDOWN] Transition detected, but cooldown ({}s) has not yet elapsed.",
                cfg.cooldown_seconds
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
    perform_shuffle(&cfg, is_foreground);
}

fn perform_shuffle(cfg: &config::AppConfig, log: bool) {
    let history_dir = std::env::var("USERPROFILE")
        .map(|p| PathBuf::from(p).join(".wallpaper-shuffler").join("history"))
        .unwrap_or_else(|_| PathBuf::from("target").join("history"));

    match composer::execute_shuffle_and_compose(
        &cfg.wallpaper_dirs,
        &history_dir,
        cfg.history_count,
    ) {
        Ok(res) => {
            if log {
                println!(
                    "[COMPOSED] Mode: {:?} | Time: {}ms | Output: {}",
                    res.mode,
                    res.duration_ms,
                    res.output_path.display()
                );
                println!(
                    "           Used {} source image(s)",
                    res.source_images.len()
                );
            }

            if let Err(e) = wallpaper::set_wallpaper(&res.output_path) {
                if log {
                    eprintln!("[ERROR] Failed to apply wallpaper: {}", e);
                }
            } else if log {
                println!("[WALLPAPER APPLIED] Successfully updated Windows wallpaper!");
            }

            *LAST_SHUFFLE_TIME.lock().unwrap() = Some(chrono::Utc::now());
        }
        Err(e) => {
            if log {
                eprintln!("[ERROR] Shuffle composition failed: {}", e);
            }
        }
    }
}

fn run_daemon(foreground: bool) {
    if foreground {
        attach_parent_console();
        FOREGROUND_LOGGING.store(true, Ordering::SeqCst);
        println!("=== wallpaper-shuffler foreground daemon started ===");
        println!(
            "Listening for window movements, minimizes (Win+D), and desktop exposure changes..."
        );
        println!("Press Ctrl+C to stop.\n");
    }

    let cfg = config::AppConfig::default();
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

        let initial_exposure = detector::calculate_desktop_exposure_rate();
        *PREVIOUS_EXPOSURE.lock().unwrap() = Some(initial_exposure);
        if foreground {
            println!(
                "[INITIAL] Current desktop exposure rate: {:.1}%",
                initial_exposure
            );
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
        "  --run-foreground                    Run daemon in console with real-time event & trigger logging"
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
    if let Some(single_cand) = images.iter().find(|p| composer::is_single_candidate(p)) {
        let out_single = out_dir.join("01_sample_single.jpg");
        match composer::execute_mode_explicit(
            composer::CompositionMode::Single,
            std::slice::from_ref(single_cand),
            &out_single,
        ) {
            Ok(res) => println!(
                "  -> Generated {} in {}ms",
                res.output_path.display(),
                res.duration_ms
            ),
            Err(e) => eprintln!("  -> Failed: {}", e),
        }
    } else {
        println!(
            "  -> Skipped: No images qualify for Single 4K mode (requires landscape width > height and width >= 3000px)."
        );
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
        println!("Single 4K Fit:   skipped (no landscape images >= 3000px)");
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
                run_daemon(true);
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

    run_daemon(false);
}
