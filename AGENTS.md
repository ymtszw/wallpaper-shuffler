# AGENTS.md

Instructions for coding agents and contributors working on this repository.
User-facing docs live in [README.md](README.md). Keep README focused on end users and put development details here.

## 1. Project Overview

`wallpaper-shuffler` is an ultra-lightweight, event-driven Windows resident background daemon (written in Rust) designed for high-resolution (4K) multi-window desktop setups. It automatically shuffles wallpapers when the desktop transitions from covered (`< 5.0%` exposed) to visible (`>= 15.0%` exposed), such as upon minimizing windows or pressing `Win + D`, as well as on PC startup or sleep resume when the desktop is exposed.

### File Role Matrix

| Module / File | Responsibility |
|---|---|
| `src/main.rs` | Entry point, CLI flag dispatch, Win32 message pump, standby pre-composition cache, hysteresis/startup/resume trigger state machine |
| `src/hook.rs` | Win32 `SetWinEventHook` installation (`MOVESIZE`, `MINIMIZE`, `FOREGROUND` events) |
| `src/detector.rs` | Window enumeration (`EnumWindows` + desktop child fallback), shell/DWM cloaked filtering, 48x27 grid desktop exposure sampling, cooldown check |
| `src/wallpaper.rs` | Native Win32 wallpaper application (`IDesktopWallpaper` COM interface with `SystemParametersInfoW` fallback and `\\?\` path stripping) |
| `src/composer.rs` | Smart wallpaper selection, Single 2K+ landscape fit, 4K Collage (primary + underlay cards), 4K Irregular Tiling, history ring buffer |
| `src/tray.rs` | Dynamic 32-bit ARGB framed-miniature tray icon (`Shell_NotifyIconW`), full tray context menu & settings dialog |
| `src/config.rs` | User configuration loading/saving (`AppConfig`) and legacy threshold migration |
| `src/logger.rs` | Daily file logger (`wallpaper-shuffler_YYYY-MM-DD.log`), runtime `LogLevel` control, 7-day retention pruning |
| `src/startup.rs` | Windows startup registration via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` and `StartupApproved\Run` cleanup |
| `src/fixtures.rs` | Synthetic 2K–4K portrait/landscape/square test image generator (`--generate-fixtures`) |
| `deny.toml` | License compliance, security advisories, and dependency ban configurations |

### Runtime Data & Configuration Paths

| Artifact | Path |
|---|---|
| Primary Config | `%USERPROFILE%\.wallpaper-shuffler\config.json` (falls back to existing `%APPDATA%\wallpaper-shuffler\config.json` if present) |
| Composed History | `%USERPROFILE%\.wallpaper-shuffler\history\wallpaper_YYYYMMDD_HHMMSS_<Mode>.jpg` |
| Daily Logs | `%USERPROFILE%\.wallpaper-shuffler\logs\wallpaper-shuffler_YYYY-MM-DD.log` (7-day auto-prune) |

---

## 2. Strict Architectural Invariants (Non-Negotiables)

1. **Strictly Zero Python on Windows**:
   - All build steps, image compositing, scripts, and tests must use Rust or PowerShell. Never invoke Python.
2. **Headless Stealth Daemon & Minimal Footprint**:
   - Target idle CPU: **0.00%**. Single standalone `.exe` with `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`.
   - For CLI commands (`--help`, `--version`, `--run-foreground`, `--test-compose`, `--register-startup`, etc.), call `AttachConsole(ATTACH_PARENT_PROCESS)` so stdout reaches the parent console.
3. **Event-Driven Window Tracking (No High-Frequency Hook Spam)**:
   - Hook only `EVENT_SYSTEM_MOVESIZESTART..=EVENT_SYSTEM_MINIMIZEEND` (`0x000A..=0x0017`) and `EVENT_SYSTEM_FOREGROUND` (`0x0003`), filtering strictly to window-level objects (`id_object == 0`).
   - **Never** hook `EVENT_OBJECT_LOCATIONCHANGE` (`0x800B`) — it fires continuously on mouse hover, cursor blink, and child control updates. A lightweight 250ms state-diff timer (`TIMER_WATCH_ID`) covers shell transitions such as `Win + D`.
4. **Exposure-Verified Triggers, Settle Debounce & Cooldown Protection**:
   - Continuous window activity resets `TIMER_DEBOUNCE_ID` (`debounce_millis`, default 400ms; `>= 1000ms` on power resume).
   - **Startup & Power Resume must also verify exposure** (`current_exposure >= exposed_threshold_percent`) after window bounds settle. Never shuffle or consume `cooldown_seconds` while the desktop is covered (`< exposed_threshold_percent`); instead set `WAS_DESKTOP_OBSCURED = true` so the next reveal triggers immediately.
5. **Frozen v1 Composition Invariants**:
   - **Single Mode**: Strictly landscape (`width > height` and `width >= 1900`). Never crop or force portrait/square images into Single mode.
   - **Collage Mode**: 5–6 primary cards (`>= 70%` unoccluded each) + minimum 2 underlay gap-filler cards (`>= 10%` visible), bilinear rotation with sub-pixel SDF edge anti-aliasing, zero duplicate source images.
   - **Tiling Mode**: Lower-center jittered anchor (`y = 1550`), uniform `30px` gaps, zero duplicate source images, downscale-only (`fit_downscale_only`, never upscale).
6. **Public Repository Hygiene (Zero Local Path Leaks)**:
   - Never hardcode usernames, `C:\Users\<name>`, personal wallpaper folder names, or agent brain/artifact directories in source code, unit tests, or docs.
   - Unit tests and visual checks must write temporary files exclusively to `std::env::temp_dir()`, `target/`, or explicit CLI output arguments.
7. **Quality Gate Compliance**:
   - Code must pass formatting, dependency audits, unit tests, and compile with zero compiler or clippy warnings (`-D warnings`).

---

## 3. Environment & Toolchain Setup (Windows 11)

Toolchains are managed with `mise` / `rustup`:

```powershell
# Toolchain verification
rustup default stable-x86_64-pc-windows-gnu

# Quality Gate Tools
rustup component add rustfmt clippy
cargo install cargo-binstall --locked
cargo binstall -y cargo-machete cargo-audit cargo-deny cargo-watch
```

| Tool | Purpose |
|---|---|
| `rustfmt` | Code formatting checks |
| `clippy` | Idiomatic Rust linting (`-D warnings` zero warnings enforced) |
| `cargo-machete` | Detection of unused dependencies in `Cargo.toml` |
| `cargo-audit` | RustSec vulnerability advisory checks |
| `cargo-deny` | License compliance and crate ban verification (`deny.toml`) |
| `cargo-watch` | Fast hot-reload development loop |

---

## 4. Quality Gate & Verification Workflows

### Pre-Commit Quality Gate (Mandatory One-Liner)
Before every commit, stop any running daemon (Section 5) and run:

```powershell
cargo fmt --check; cargo clippy --all-targets --all-features -- -D warnings; cargo machete; cargo audit; cargo deny check; cargo test
```

### Visual Composition & Trigger Testing Workflow
1. **Generate synthetic test fixtures** (avoids needing personal image directories):
   ```powershell
   cargo run --release -- --generate-fixtures
   ```
2. **Render sample 4K outputs without changing the OS wallpaper**:
   ```powershell
   cargo run --release -- --test-compose tests/fixtures/images target/test_output
   ```
3. **Interactive foreground trigger debugging** (uses a short 5-second cooldown for rapid testing):
   ```powershell
   cargo run -- --run-foreground --debug
   ```

---

## 5. Process Lifecycle & Daemon Management

### Terminating Running Daemon Before Build
To prevent Windows executable file-lock errors (`os error 5`) during `cargo build` or `cargo test`:

```powershell
Get-Process -Name wallpaper-shuffler -ErrorAction SilentlyContinue | Stop-Process -Force -Confirm:$false
```

### Spawning Detached Daemon from Agent / ConPTY Shell
Terminal sessions under ConPTY / Windows Job Objects automatically kill child processes (`Start-Process` / `System.Diagnostics.Process::Start`) when the command exits. To spawn a truly detached resident daemon via WMI outside the shell Job Object:

```powershell
$exe = (Resolve-Path "target\release\wallpaper-shuffler.exe").Path
Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $exe }
```

---

## 6. External Subsystem Integrations & Hard-Won Gotchas

1. **Desktop Shell Surfaces & DWM Cloaked Windows (`detector.rs`)**:
   - `EnumWindows` enumerates full-screen desktop surfaces (`Progman`, `WorkerW`), taskbars (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`), suspended UWP/virtual-desktop windows (`DWMWA_CLOAKED != 0`), and tiny hidden helper windows (`< 100x100` or `WS_EX_TOOLWINDOW` without `WS_EX_APPWINDOW`). Failing to filter any of these causes desktop exposure to read `0.0%` permanently.
2. **Power Resume Dual Broadcasts & COM STA Re-Entrancy (`main.rs`, `wallpaper.rs`)**:
   - Resuming from sleep/hibernate emits both `PBT_APMRESUMEAUTOMATIC` and `PBT_APMRESUMESUSPEND` within ~200–500ms.
   - `IDesktopWallpaper::SetWallpaper` runs in a COM STA (`COINIT_APARTMENTTHREADED`) and pumps queued window messages internally. Never invoke `apply_or_await_wallpaper` directly inside `window_proc` on `WM_POWERBROADCAST`; always defer via `TIMER_DEBOUNCE_ID` (`>= 1000ms` settle) and keep `IS_APPLYING` re-entrancy protection active.
3. **Win32 Path Prefix Stripping (`wallpaper.rs`)**:
   - `std::fs::canonicalize` on Windows prepends `\\?\`, which causes `IDesktopWallpaper::SetWallpaper` and `SystemParametersInfoW(SPI_SETDESKWALLPAPER, ...)` to fail silently or return an error. Always strip `r"\\?\"` before passing paths to Win32 APIs.
4. **Speculative Standby Pre-Composition (`main.rs`)**:
   - 4K image decoding and compositing takes ~300–2500ms depending on source resolution. `ensure_candidate_precomposed_async` pre-generates one standby wallpaper in a background thread (`STANDBY_WALLPAPER`) on daemon startup, window movement start, and immediately after each apply so desktop reveals apply in ~0ms.
5. **Windows Startup Registry & `StartupApproved\Run` (`startup.rs`)**:
   - If a user previously disabled the app in Windows Settings (Apps > Startup) or Task Manager, Windows writes a binary disable flag under `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run` that overrides `CurrentVersion\Run`. `register_startup` deletes any stale `StartupApproved\Run` entry so re-enabling works reliably.
