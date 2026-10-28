# wallpaper-shuffler

Ultra-lightweight, event-driven Windows resident background daemon that intelligently shuffles wallpapers when your desktop is actually revealed.

Designed for 4K displays and heavy multi-window multitasking where wallpapers are hidden behind windows most of the day.

---

## Features

- **Desktop Exposure Trigger**: Automatically detects when your desktop transitions from hidden (<10% exposed) to visible (>=20% exposed), such as upon minimizing windows or pressing <kbd>Win</kbd> + <kbd>D</kbd>.
- **Event-Driven & 0% CPU Idle**: Listens to native OS window movement and minimize events via Win32 `SetWinEventHook`. No continuous polling, zero background CPU drain.
- **Smart Image Fitting & 4K Collage**:
  - High-res wallpaper (matching aspect ratio and width >= 3000px): Applied directly as a single pristine image.
  - Portrait or smaller images: Dynamically arranged into an aesthetic collage or dynamic tiles on a 4K canvas.
  - History cache: Keeps recent generated collage wallpapers in a local history ring buffer.
- **Anti-Churn Debounce & Cooldown**: Waits for window rearranging to settle before calculating coverage, and enforces a cooldown period to avoid repetitive shuffling.
- **System Tray & Windows Startup**: Discreet tray icon with quick controls (Shuffle Now, Pause, Config, Exit) and optional Windows startup integration.
- **Minimal Footprint**: Native Rust binary with 3–8 MB memory usage and single `.exe` deployment.

---

## Quick Start

### Installation & Run

1. Download or build `wallpaper-shuffler.exe`.
2. Place candidate wallpaper images in your configured wallpaper folder (default: `%USERPROFILE%\Pictures\Wallpapers`).
3. Launch `wallpaper-shuffler.exe`:
   - Runs silently in the system tray.
   - Right-click tray icon to manually shuffle, pause detection, or configure settings.

### Auto-start on Boot

Register to start automatically with Windows:

```powershell
wallpaper-shuffler.exe --register-startup
```

To remove from startup:

```powershell
wallpaper-shuffler.exe --unregister-startup
```

---

## Configuration

Settings are saved in `%APPDATA%\wallpaper-shuffler\config.json`:

```json
{
  "wallpaper_dirs": [
    "C:\\Users\\<Username>\\Pictures\\Wallpapers"
  ],
  "exposed_threshold_percent": 20.0,
  "obscured_threshold_percent": 10.0,
  "cooldown_seconds": 300,
  "debounce_millis": 400,
  "history_count": 10
}
```

---

## Build from Source

Requirements:
- Windows 10 / 11
- Rust toolchain (`stable-x86_64-pc-windows-gnu` or `stable-x86_64-pc-windows-msvc`)

```powershell
cargo build --release
```

The compiled binary will be placed at `target\release\wallpaper-shuffler.exe`.

---

## License

[MIT](LICENSE)
