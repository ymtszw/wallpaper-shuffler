# AGENTS.md

Instructions for coding agents and contributors working on this repository.
User-facing docs live in [README.md](README.md). Keep README focused on end users and put development details here.

## 1. Project Overview

`wallpaper-shuffler` is an ultra-lightweight, event-driven Windows resident background daemon (written in Rust) designed for high-resolution (4K) multi-window desktop setups. It automatically shuffles wallpapers when the desktop transitions from mostly covered (<10% exposed) to visible (>=20% exposed), such as upon minimizing windows or using Win + D.

### File Role Matrix

| Module / File | Responsibility |
|---|---|
| `src/main.rs` | Entry point, CLI flag dispatch, Win32 message pump, main orchestrator loop |
| `src/hook.rs` | Win32 `SetWinEventHook` installation, window location change & minimize event capture with debounce timer |
| `src/detector.rs` | Window enumeration (`EnumWindows`), virtual screen rect calculation, DWM cloaked filter, desktop exposure ratio calculation |
| `src/wallpaper.rs` | Native Win32 wallpaper application (`IDesktopWallpaper` COM interface / `SystemParametersInfoW`) |
| `src/composer.rs` | Smart wallpaper selection, single-image fitting, dynamic 4K collage / tile composer via `image` crate, history ring buffer |
| `src/tray.rs` | Native Win32 system tray icon (`Shell_NotifyIconW`), tray menu (Shuffle Now, Pause, Config, Exit) |
| `src/config.rs` | User configuration loading/saving (wallpaper directories, thresholds, cooldown, history limit) |
| `src/startup.rs` | Windows startup registration via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` |
| `deny.toml` | License compliance, security advisories, and dependency ban configurations |

---

## 2. Strict Architectural Invariants (Non-Negotiables)

1. **Strictly Zero Python on Windows**:
   - All build steps, image compositing, scripts, and tests must use Rust or PowerShell. Never invoke Python.
2. **Headless Stealth Daemon & Minimal Footprint**:
   - Target memory consumption: **3 〜 8 MB working set**. CPU idle: **0.00%**.
   - Release builds must specify `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` to run with zero console windows.
   - For CLI commands (`--help`, `--status`, `--register-startup`), call `AttachConsole(ATTACH_PARENT_PROCESS)` so stdout reaches the terminal when run from console.
3. **Pure Event-Driven Architecture (Zero Polling)**:
   - Window movement/resize/minimize is monitored purely via Win32 `SetWinEventHook`.
   - Never use tight polling loops to query window positions. Debounce timer (300-500ms) resets on active window actions and evaluates exposure only when window activity settles.
4. **Debounce & Cooldown Protection**:
   - Continuous window movement must reset the debounce timer.
   - Once a shuffle is triggered, enforce a minimum cooldown period to prevent annoying rapid flips.
5. **Quality Gate Compliance**:
   - Code must compile with zero compiler warnings and zero clippy warnings (`-D warnings`).

---

## 3. Environment & Toolchain Setup (Windows 11)

Toolchains are managed with `mise` / `rustup`:

```powershell
# Toolchain & Linker verification
rustup default stable-x86_64-pc-windows-gnu

# Quality Gate Tools
rustup component add rustfmt clippy
cargo install cargo-binstall --locked
cargo binstall -y cargo-machete cargo-audit cargo-deny cargo-watch
```

| Tool | Purpose |
|---|---|
| `rustfmt` | Code formatting checks |
| `clippy` | Idiomatic Rust linting (zero warnings enforced) |
| `cargo-machete` | Detection of unused dependencies in `Cargo.toml` |
| `cargo-audit` | RustSec vulnerability advisory checks |
| `cargo-deny` | License compliance and crate ban verification (`deny.toml`) |
| `cargo-watch` | Fast hot-reload development loop |

---

## 4. Quality Gate (Pre-Commit One-Liner)

Before every commit, run and ensure zero errors and zero warnings:

```powershell
cargo fmt --check; cargo clippy --all-targets --all-features; cargo machete; cargo audit; cargo deny check; cargo test
```

---

## 5. Process Lifecycle & Daemon Management

### Terminating Running Daemon Before Build
To prevent Windows file-lock errors (`os error 5`) during `cargo build` or `cargo test`:

```powershell
Get-Process -Name wallpaper-shuffler -ErrorAction SilentlyContinue | Stop-Process -Force
```

### Spawning Detached Daemon from Shell
PowerShell jobs terminate child processes when the agent shell exits. To spawn a detached daemon:

```powershell
$proc = [System.Diagnostics.Process]::Start((New-Object System.Diagnostics.ProcessStartInfo -Property @{
    FileName = "target\release\wallpaper-shuffler.exe";
    UseShellExecute = $true
}))
```

---

## 6. External Subsystem Integrations & Gotchas

1. **WinEventHook & Message Pump**:
   - `SetWinEventHook` with `WINEVENT_OUTOFCONTEXT` requires an active Win32 message loop (`GetMessageW` or `PeekMessageW`) on the hooking thread.
2. **DWM Cloaked Windows**:
   - Modern Windows Store apps (UWP/XAML) and background virtual desktop windows remain enumerated by `EnumWindows` even when hidden. Check `DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, ...)` to ensure cloaked windows do not falsely count as covering the desktop.
3. **Wallpaper Setting Interface**:
   - `IDesktopWallpaper::SetWallpaper` (COM interface) supports per-monitor wallpaper setting and multi-monitor setups cleanly without blocking the UI thread.
   - Fallback: `SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, path, SPIF_UPDATEINIFILE | SPIF_SENDCHANGE)`.
4. **4K Canvas Compositing**:
   - 4K canvas size is 3840x2160 pixels. Direct compositing using `image` crate (e.g. `RgbaImage` or `RgbImage`) should perform memory-efficient buffer writes and crop/resize using Lanczos3 or Triangle filters.
