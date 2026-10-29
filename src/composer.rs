use image::imageops::{FilterType, crop_imm, overlay, resize};
use image::{ImageReader, Rgb, RgbImage};
use rand::Rng;
use rand::seq::SliceRandom;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const CANVAS_WIDTH_4K: u32 = 3840;
pub const CANVAS_HEIGHT_4K: u32 = 2160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionMode {
    Single,
    Collage,
    Tiling,
}

#[derive(Debug, Clone)]
pub struct CompositionResult {
    pub mode: CompositionMode,
    pub output_path: PathBuf,
    pub source_images: Vec<PathBuf>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Copy)]
pub struct TileRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

pub fn is_single_candidate(path: &Path) -> bool {
    let Ok(reader) = ImageReader::open(path) else {
        return false;
    };
    let Ok(reader) = reader.with_guessed_format() else {
        return false;
    };
    let Ok((width, height)) = reader.into_dimensions() else {
        return false;
    };
    let aspect = width as f64 / height as f64;
    let target_aspect = 16.0 / 9.0;
    let aspect_diff = (aspect - target_aspect).abs();
    width >= 3000 && aspect_diff < 0.35
}

pub fn load_rgb_image(path: &Path) -> Result<RgbImage, String> {
    let reader = ImageReader::open(path)
        .map_err(|e| format!("Cannot open {}: {}", path.display(), e))?
        .with_guessed_format()
        .map_err(|e| format!("Format error {}: {}", path.display(), e))?;

    let dynamic = reader
        .decode()
        .map_err(|e| format!("Decode error {}: {}", path.display(), e))?;

    Ok(dynamic.into_rgb8())
}

/// Resize and center-crop to cover target dimensions exactly
pub fn fit_cover_and_crop(src: &RgbImage, target_w: u32, target_h: u32) -> RgbImage {
    let sw = src.width() as f64;
    let sh = src.height() as f64;
    let tw = target_w as f64;
    let th = target_h as f64;

    let scale = (tw / sw).max(th / sh);
    let nw = ((sw * scale).round() as u32).max(target_w);
    let nh = ((sh * scale).round() as u32).max(target_h);

    let resized = resize(src, nw, nh, FilterType::Triangle);

    let crop_x = (nw - target_w) / 2;
    let crop_y = (nh - target_h) / 2;

    let cropped = crop_imm(&resized, crop_x, crop_y, target_w, target_h);
    cropped.to_image()
}

/// Mode A: Single 4K wallpaper fit
pub fn compose_single(src_path: &Path) -> Result<RgbImage, String> {
    let src = load_rgb_image(src_path)?;
    Ok(fit_cover_and_crop(&src, CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K))
}

/// Generate dark ambient gradient background for collage
pub fn create_ambient_background(w: u32, h: u32) -> RgbImage {
    let mut bg = RgbImage::new(w, h);
    let wf = w as f32;
    let hf = h as f32;
    let cx = wf / 2.0;
    let cy = hf / 2.0;
    let max_dist = (cx * cx + cy * cy).sqrt();

    for (x, y, pixel) in bg.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let dist = (dx * dx + dy * dy).sqrt() / max_dist;

        // Dark radial slate vignette
        let t = dist.clamp(0.0, 1.0);
        let r = (24.0 * (1.0 - t) + 8.0 * t) as u8;
        let g = (28.0 * (1.0 - t) + 10.0 * t) as u8;
        let b = (38.0 * (1.0 - t) + 16.0 * t) as u8;

        *pixel = Rgb([r, g, b]);
    }
    bg
}

/// Mode B: Collage mode with card frames
pub fn compose_collage(images: &[PathBuf]) -> Result<RgbImage, String> {
    if images.is_empty() {
        return Err("No images provided for collage".to_string());
    }

    let mut canvas = create_ambient_background(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K);

    let count = images.len().clamp(3, 6);
    let cols = 3u32;
    let rows = if count <= 3 { 1 } else { 2 };

    let margin_x = 160u32;
    let margin_y = 120u32;
    let gap_x = 60u32;
    let gap_y = 60u32;

    let avail_w = CANVAS_WIDTH_4K - margin_x * 2 - gap_x * (cols - 1);
    let avail_h = CANVAS_HEIGHT_4K - margin_y * 2 - gap_y * (rows - 1);

    let card_w = avail_w / cols;
    let card_h = avail_h / rows;
    let border = 12u32;

    for (i, img_path) in images.iter().take(count).enumerate() {
        let c = (i as u32) % cols;
        let r = (i as u32) / cols;

        let x = margin_x + c * (card_w + gap_x);
        let y = margin_y + r * (card_h + gap_y);

        if let Ok(src) = load_rgb_image(img_path) {
            let inner_w = card_w.saturating_sub(border * 2);
            let inner_h = card_h.saturating_sub(border * 2);
            let photo = fit_cover_and_crop(&src, inner_w, inner_h);

            // Draw white card frame
            let mut card = RgbImage::from_pixel(card_w, card_h, Rgb([245, 245, 248]));
            overlay(&mut card, &photo, border as i64, border as i64);

            overlay(&mut canvas, &card, x as i64, y as i64);
        }
    }

    Ok(canvas)
}

/// Recursive binary partitioning for irregular dynamic tiles
pub fn partition_tiles(
    rect: TileRect,
    target_count: usize,
    rng: &mut impl Rng,
    out: &mut Vec<TileRect>,
) {
    if target_count <= 1 || rect.w < 500 || rect.h < 400 {
        out.push(rect);
        return;
    }

    let split_vertical = if rect.w as f32 > rect.h as f32 * 1.3 {
        true
    } else if (rect.h as f32) > (rect.w as f32) * 1.3 {
        false
    } else {
        rng.gen_bool(0.5)
    };

    let ratio: f32 = rng.gen_range(0.38..0.62);

    let left_count = target_count / 2;
    let right_count = target_count - left_count;

    if split_vertical {
        let w1 = ((rect.w as f32) * ratio).round() as u32;
        let w2 = rect.w.saturating_sub(w1);
        let r1 = TileRect {
            x: rect.x,
            y: rect.y,
            w: w1,
            h: rect.h,
        };
        let r2 = TileRect {
            x: rect.x + w1,
            y: rect.y,
            w: w2,
            h: rect.h,
        };
        partition_tiles(r1, left_count, rng, out);
        partition_tiles(r2, right_count, rng, out);
    } else {
        let h1 = ((rect.h as f32) * ratio).round() as u32;
        let h2 = rect.h.saturating_sub(h1);
        let r1 = TileRect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: h1,
        };
        let r2 = TileRect {
            x: rect.x,
            y: rect.y + h1,
            w: rect.w,
            h: h2,
        };
        partition_tiles(r1, left_count, rng, out);
        partition_tiles(r2, right_count, rng, out);
    }
}

/// Mode C: Irregular Dynamic Tiling on 4K canvas
pub fn compose_tiling(images: &[PathBuf]) -> Result<RgbImage, String> {
    if images.is_empty() {
        return Err("No images provided for tiling".to_string());
    }

    let mut canvas = RgbImage::from_pixel(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K, Rgb([16, 18, 24]));
    let mut rng = rand::thread_rng();

    let target_tiles = images.len().clamp(5, 8);
    let mut tiles = Vec::new();

    let base_rect = TileRect {
        x: 0,
        y: 0,
        w: CANVAS_WIDTH_4K,
        h: CANVAS_HEIGHT_4K,
    };
    partition_tiles(base_rect, target_tiles, &mut rng, &mut tiles);

    let gap = 16u32;

    for (i, tile) in tiles.iter().enumerate() {
        let img_path = &images[i % images.len()];
        if let Ok(src) = load_rgb_image(img_path) {
            let inner_w = tile.w.saturating_sub(gap * 2);
            let inner_h = tile.h.saturating_sub(gap * 2);

            if inner_w > 0 && inner_h > 0 {
                let cell_img = fit_cover_and_crop(&src, inner_w, inner_h);
                let place_x = (tile.x + gap) as i64;
                let place_y = (tile.y + gap) as i64;
                overlay(&mut canvas, &cell_img, place_x, place_y);
            }
        }
    }

    Ok(canvas)
}

pub fn collect_image_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let valid_exts = ["jpg", "jpeg", "png", "webp"];

    for dir in dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let has_valid_ext = path
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|ext| valid_exts.contains(&ext.to_lowercase().as_str()))
                        .unwrap_or(false);
                    if has_valid_ext {
                        files.push(path);
                    }
                }
            }
        }
    }
    files
}

pub fn manage_history(history_dir: &Path, max_history: usize) {
    if let Ok(entries) = fs::read_dir(history_dir) {
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.is_file() { Some(p) } else { None }
            })
            .collect();

        files.sort_by(|a, b| {
            let ma = fs::metadata(a).and_then(|m| m.modified()).ok();
            let mb = fs::metadata(b).and_then(|m| m.modified()).ok();
            mb.cmp(&ma)
        });

        if files.len() > max_history {
            for old in &files[max_history..] {
                let _ = fs::remove_file(old);
            }
        }
    }
}

pub fn execute_shuffle_and_compose(
    wallpaper_dirs: &[PathBuf],
    history_dir: &Path,
    max_history: usize,
) -> Result<CompositionResult, String> {
    let start = Instant::now();
    let mut images = collect_image_files(wallpaper_dirs);
    if images.is_empty() {
        return Err("No valid wallpaper images found in configured directories".to_string());
    }

    let mut rng = rand::thread_rng();
    images.shuffle(&mut rng);

    fs::create_dir_all(history_dir).map_err(|e| format!("Failed to create history dir: {}", e))?;

    let primary = &images[0];
    let (mode, output_image, used_sources) = if is_single_candidate(primary) {
        let img = compose_single(primary)?;
        (CompositionMode::Single, img, vec![primary.clone()])
    } else {
        // Randomly choose between Collage and Tiling
        let use_tiling = rng.gen_bool(0.5);
        if use_tiling {
            let count = rng.gen_range(5..=7).min(images.len());
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_tiling(&selected)?;
            (CompositionMode::Tiling, img, selected)
        } else {
            let count = rng.gen_range(3..=6).min(images.len());
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_collage(&selected)?;
            (CompositionMode::Collage, img, selected)
        }
    };

    let filename = format!(
        "wallpaper_{}_{:?}.jpg",
        chrono::Utc::now().format("%Y%m%d_%H%M%S"),
        mode
    );
    let output_path = history_dir.join(&filename);

    output_image.save(&output_path).map_err(|e| {
        format!(
            "Failed to save composed wallpaper {}: {}",
            output_path.display(),
            e
        )
    })?;

    manage_history(history_dir, max_history);

    let duration_ms = start.elapsed().as_millis();

    Ok(CompositionResult {
        mode,
        output_path,
        source_images: used_sources,
        duration_ms,
    })
}

pub fn execute_mode_explicit(
    mode: CompositionMode,
    images: &[PathBuf],
    output_path: &Path,
) -> Result<CompositionResult, String> {
    let start = Instant::now();
    if images.is_empty() {
        return Err("No images provided".to_string());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create dir {}: {}", parent.display(), e))?;
    }

    let (composed, used) = match mode {
        CompositionMode::Single => {
            let img = compose_single(&images[0])?;
            (img, vec![images[0].clone()])
        }
        CompositionMode::Collage => {
            let count = images.len().clamp(3, 6);
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_collage(&selected)?;
            (img, selected)
        }
        CompositionMode::Tiling => {
            let count = images.len().clamp(5, 8);
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_tiling(&selected)?;
            (img, selected)
        }
    };

    composed
        .save(output_path)
        .map_err(|e| format!("Failed to save {}: {}", output_path.display(), e))?;

    let duration_ms = start.elapsed().as_millis();

    Ok(CompositionResult {
        mode,
        output_path: output_path.to_path_buf(),
        source_images: used,
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canvas_dimensions() {
        let bg = create_ambient_background(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K);
        assert_eq!(bg.width(), 3840);
        assert_eq!(bg.height(), 2160);
    }

    #[test]
    fn test_fit_cover_and_crop() {
        let src = RgbImage::from_pixel(1000, 2000, Rgb([255, 0, 0]));
        let cropped = fit_cover_and_crop(&src, 800, 600);
        assert_eq!(cropped.width(), 800);
        assert_eq!(cropped.height(), 600);
    }
}
