# wallpaper-shuffler

Ultra-lightweight, event-driven Windows resident background daemon that intelligently shuffles wallpapers when your desktop is actually revealed.

Designed for 4K displays and heavy multi-window multitasking where wallpapers are hidden behind windows most of the day.

---

## Features

- **Desktop Exposure Trigger**: Automatically detects when your desktop transitions from hidden (<5% exposed) to visible (>=15% exposed), such as upon minimizing windows or pressing <kbd>Win</kbd> + <kbd>D</kbd>.
- **Event-Driven & 0% CPU Idle**: Listens to native OS window movement and minimize events via Win32 `SetWinEventHook`. No continuous polling, zero background CPU drain.
- **Smart Composition Engine (v1 Frozen)**:
  - **Single 2K+ Fit**: Strictly landscape images ($\ge 1900\text{px}$) are center-fit to 4K (3840x2160) preserving aspect ratio. Portrait photos are never trimmed or forced.
  - **Large-Print Collage**: 5 primary cards with underlay gap-fillers scattered across 4K with gentle organic tilts ($\pm 3^\circ \sim \pm 9^\circ$), smooth anti-aliased subpixel edges, and wood-framed borders.
  - **Dynamic Irregular Tiling**: Lower-center anchored placement with natural jitter, 30px gaps, and duplicate-free image selection.
  - **5-Color Rotating Pastel Ambient Backdrops**: Cycles through elegant soft tints (Lavender, Sakura Blossom, Sunset Apricot, Sky Mist, Sage Mint) with subtle spatial gradients behind photos.
  - **Broad Format Support**: Seamlessly reads `.jpg`, `.jpeg`, `.jfif`, `.jiff`, `.png`, and `.webp`.
- **Anti-Churn Debounce & Cooldown**: Waits for window rearranging to settle before calculating coverage, and enforces a cooldown period to avoid repetitive shuffling.
- **System Tray & Windows Startup**: Discreet tray icon with quick controls (Shuffle Now, Pause, Config, Exit) and native Windows startup integration manageable directly in Windows Settings (Apps > Startup) or Task Manager.
- **Persistent Headless Logging & Rotation**: Daily log files (`wallpaper-shuffler_YYYY-MM-DD.log`) stored in `%USERPROFILE%\.wallpaper-shuffler\logs\` with automatic 7-day cleanup.
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

## Testing & Image Generation Commands

You can test composition algorithms or generate wallpapers on any directory without changing your Windows wallpaper:

```powershell
# Generate all 3 sample styles (Single, Collage, Tiling) with random sampling
wallpaper-shuffler.exe --test-compose <IMAGE_DIR> [OUT_DIR]

# Generate a specific style directly
wallpaper-shuffler.exe --compose-collage <IMAGE_DIR> [OUT_FILE]
wallpaper-shuffler.exe --compose-tiling  <IMAGE_DIR> [OUT_FILE]
wallpaper-shuffler.exe --compose-single  <IMAGE_DIR_OR_FILE> [OUT_FILE]

# Measure composition speeds across all 3 modes
wallpaper-shuffler.exe --benchmark <IMAGE_DIR>

# Run resident daemon in foreground console with real-time exposure & trigger logs
wallpaper-shuffler.exe --run-foreground
```

---

## Configuration

Settings are saved in `%USERPROFILE%\.wallpaper-shuffler\config.json`:

```json
{
  "wallpaper_dirs": [
    "C:\\Users\\<Username>\\Pictures\\Wallpapers"
  ],
  "exposed_threshold_percent": 15.0,
  "obscured_threshold_percent": 5.0,
  "cooldown_seconds": 300,
  "debounce_millis": 400,
  "history_count": 10,
  "log_level": "info"
}
```

Logs are stored in `%USERPROFILE%\.wallpaper-shuffler\logs\wallpaper-shuffler_YYYY-MM-DD.log` and automatically pruned after 7 days.

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
