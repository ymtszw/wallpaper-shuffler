# wallpaper-shuffler

A small resident Windows program that shuffles desktop wallpapers when the desktop becomes visible.

## Background

On a 4K display with multiple windows tiled across the screen, the wallpaper is hidden most of the day. Fixed-interval wallpaper changers keep rotating images while nobody can see them, and typical image folders mix wide landscapes with portrait illustrations or smaller images that do not fit a 16:9 screen cleanly on their own.

`wallpaper-shuffler` was built around three practical goals:

- **Shuffle on desktop reveal**: Uses Win32 window event hooks (`SetWinEventHook`) with debounce and a cooldown timer. It stays idle while windows cover the screen and switches the wallpaper when minimizing or closing windows (including `Win + D`), boot, or wake-from-sleep leaves the desktop exposed.
- **Layout by image dimensions**: Landscape images wide enough for a desktop (`>= 1900px`) are fitted as a single 4K wallpaper. Portrait or smaller images are combined onto a 4K (`3840x2160`) pastel backdrop as either a multi-card collage or a tiled layout without cropping the source images. Recent outputs are saved to a local history folder.
- **Minimal startup footprint**: Written in Rust with `windows-rs` as a single standalone `.exe` with no runtime or garbage collector, intended to stay registered in Windows startup.

## Features

- **Exposure-based trigger**: Triggers when visible desktop area transitions from covered (`< 5%`) to exposed (`>= 15%`), after window operations settle (`400ms` default debounce) and cooldown (`300s` default) has elapsed.
- **Three 4K output modes**:
  - **Single**: Center-fit for landscape images (`width > height`, `width >= 1900px`).
  - **Collage**: 5–6 framed cards with underlay gap-fillers rotated at slight angles.
  - **Tiling**: Uncropped images packed outward with uniform `30px` gaps.
  - **Supported formats**: `.jpg`, `.jpeg`, `.jfif`, `.jiff`, `.png`, `.webp`.
- **Background pre-generation**: Pre-composes the next candidate image in a background thread so switching on desktop reveal does not stall.
- **System tray controls**: Shows a miniature of the current wallpaper in the tray, with right-click actions to shuffle manually, pause/resume, view or reload `config.json`, open wallpaper/log folders, toggle Windows startup, and change log verbosity.
- **Daily logs**: Writes logs to `%USERPROFILE%\.wallpaper-shuffler\logs\` and deletes files older than 7 days.

## Usage

To keep maintenance overhead low, prebuilt release binaries and installers are not distributed. Building from source requires Windows 10/11 and a Rust toolchain.

### Setup with an AI Coding Agent

Because build and operational details are documented in [`AGENTS.md`](AGENTS.md), you can hand this repository to your coding agent (Antigravity, Claude Code, Codex, Cursor, etc.) and ask it to clone the repo, run a release build, point `wallpaper_dirs` in `%USERPROFILE%\.wallpaper-shuffler\config.json` to your image folder, and register Windows startup.

### Manual Build & Setup

```powershell
cargo build --release

# Launch resident tray app (creates default config on first run)
.\target\release\wallpaper-shuffler.exe

# Optional: register or remove Windows startup entry
.\target\release\wallpaper-shuffler.exe --register-startup
.\target\release\wallpaper-shuffler.exe --unregister-startup
```

## Configuration

Stored at `%USERPROFILE%\.wallpaper-shuffler\config.json` (editable and reloadable from the tray menu):

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

Generated wallpapers are kept in `%USERPROFILE%\.wallpaper-shuffler\history\`.

## CLI Commands (Testing & Diagnostics)

```powershell
wallpaper-shuffler.exe --generate-fixtures [OUT_DIR]        # Generate synthetic test images
wallpaper-shuffler.exe --test-compose <IMAGE_DIR> [OUT_DIR] # Render Single, Collage, and Tiling samples
wallpaper-shuffler.exe --shuffle-dry-run <IMAGE_DIR> [OUT_DIR]
wallpaper-shuffler.exe --run-foreground [IMAGE_DIR] [--debug]
wallpaper-shuffler.exe --help
```

## License

[MIT](LICENSE)
