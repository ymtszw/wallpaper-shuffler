# wallpaper-shuffler

Ultra-lightweight, event-driven Windows resident background daemon that intelligently shuffles wallpapers when your desktop is actually revealed.

Designed for 4K displays and heavy multi-window multitasking where wallpapers are hidden behind windows most of the day.

---

## Features

- **Desktop Exposure Trigger**: Automatically detects when your desktop transitions from covered (<5% exposed) to visible (>=15% exposed), such as upon minimizing windows or pressing <kbd>Win</kbd> + <kbd>D</kbd>.
- **Exposure-Aware Startup & Sleep Resume**: Checks desktop visibility after startup or wake-from-sleep windows settle—shuffling when your desktop is visible, and preserving cooldown when covered behind maximized windows.
- **Event-Driven & 0% CPU Idle**: Listens to native OS window events via Win32 `SetWinEventHook` with speculative background pre-composition for instant (zero-wait) wallpaper switching.
- **Smart 4K Composition Engine**:
  - **Single 2K+ Fit**: Strictly landscape images ($\ge 1900\text{px}$) are center-fit to 4K (3840x2160) preserving aspect ratio. Portrait photos are never trimmed or forced.
  - **Large-Print Collage**: 5–6 primary framed cards ($\ge 70\%$ visible each) with underlay gap-fillers scattered across 4K at gentle organic tilts, smooth sub-pixel anti-aliased edges, and soft drop shadows.
  - **Dynamic Irregular Tiling**: Lower-center anchored placement with natural jitter, uniform 30px gaps, and strict duplicate-free image selection.
  - **5-Color Rotating Pastel Ambient Backdrops**: Cycles through soft tints (Lavender, Sakura Blossom, Sunset Apricot, Sky Mist, Sage Mint) with subtle spatial gradients behind photos.
  - **Broad Format Support**: Seamlessly reads `.jpg`, `.jpeg`, `.jfif`, `.jiff`, `.png`, and `.webp`.
- **Rich System Tray & Startup Integration**:
  - Dynamic 32x32 framed miniature icon displaying the currently applied wallpaper.
  - Right-click menu to **Shuffle Now**, **Pause/Resume**, inspect **Current Settings & Status**, **Edit / Reload `config.json`**, open **Wallpapers / Logs** folders, toggle **Start with Windows**, or switch **Log Level** on the fly.
- **Persistent Headless Logging & Rotation**: Daily log files (`wallpaper-shuffler_YYYY-MM-DD.log`) stored in `%USERPROFILE%\.wallpaper-shuffler\logs\` with automatic 7-day cleanup.
- **Minimal Footprint**: Native Rust binary with single `.exe` deployment and zero runtime dependencies.

---

## Quick Start

### Installation & Run

1. Download or build `wallpaper-shuffler.exe`.
2. Place candidate wallpaper images in your configured wallpaper folder (default: `%USERPROFILE%\Pictures\Wallpapers`), or edit `config.json` from the tray menu.
3. Launch `wallpaper-shuffler.exe`:
   - Runs silently in the system tray.
   - Right-click the framed tray icon to shuffle manually, pause auto-detection, reload configuration, or enable **Start with Windows**.

### Auto-start on Boot (CLI)

You can toggle startup directly from the tray menu (**Start with Windows**), or via CLI:

```powershell
wallpaper-shuffler.exe --register-startup
wallpaper-shuffler.exe --unregister-startup
```

---

## Testing & Image Generation Commands

You can generate synthetic test images, preview composition modes, or benchmark performance on any directory without changing your Windows wallpaper:

```powershell
# Generate 30 synthetic 2K-4K test images (default: tests/fixtures/images)
wallpaper-shuffler.exe --generate-fixtures [OUT_DIR]

# Generate all 3 sample styles (Single, Collage, Tiling) with random sampling
wallpaper-shuffler.exe --test-compose <IMAGE_DIR> [OUT_DIR]

# Test the full auto-shuffle selection & history pipeline without changing OS wallpaper
wallpaper-shuffler.exe --shuffle-dry-run <IMAGE_DIR> [OUT_DIR]

# Generate a specific style directly
wallpaper-shuffler.exe --compose-collage <IMAGE_DIR> [OUT_FILE]
wallpaper-shuffler.exe --compose-tiling  <IMAGE_DIR> [OUT_FILE]
wallpaper-shuffler.exe --compose-single  <IMAGE_DIR_OR_FILE> [OUT_FILE]

# Measure composition speeds across all 3 modes
wallpaper-shuffler.exe --benchmark <IMAGE_DIR>

# Run resident daemon in foreground console with real-time exposure & trigger logs
wallpaper-shuffler.exe --run-foreground [IMAGE_DIR] [--debug | --log-level <debug|info|warn|error>]
```

---

## Configuration

Settings are saved in `%USERPROFILE%\.wallpaper-shuffler\config.json` (editable via the tray menu's **Edit Settings** + **Reload Configuration**):

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
