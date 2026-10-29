use image::imageops::{FilterType, crop_imm, resize};
use image::{ImageReader, Rgb, RgbImage, Rgba, RgbaImage};
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
pub struct RectI32 {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl RectI32 {
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }
    pub fn intersects(&self, other: &RectI32) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
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

/// Scale down image ONLY if larger than max_dim, preserving exact original aspect ratio. Never upscales.
pub fn fit_downscale_only(src: &RgbImage, max_dim: u32) -> RgbImage {
    let w = src.width();
    let h = src.height();
    if w <= max_dim && h <= max_dim {
        return src.clone();
    }
    let scale = (max_dim as f64) / (w.max(h) as f64);
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    resize(src, nw, nh, FilterType::Triangle)
}

/// Mode A: Single 4K wallpaper fit (only for candidates >= 3000px 16:9 approx)
pub fn compose_single(src_path: &Path) -> Result<RgbImage, String> {
    let src = load_rgb_image(src_path)?;
    let sw = src.width() as f64;
    let sh = src.height() as f64;
    let tw = CANVAS_WIDTH_4K as f64;
    let th = CANVAS_HEIGHT_4K as f64;

    let scale = (tw / sw).max(th / sh);
    let nw = ((sw * scale).round() as u32).max(CANVAS_WIDTH_4K);
    let nh = ((sh * scale).round() as u32).max(CANVAS_HEIGHT_4K);

    let resized = resize(&src, nw, nh, FilterType::Triangle);
    let crop_x = (nw - CANVAS_WIDTH_4K) / 2;
    let crop_y = (nh - CANVAS_HEIGHT_4K) / 2;

    Ok(crop_imm(&resized, crop_x, crop_y, CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K).to_image())
}

/// Compute harmonious, muted background color derived from source images
pub fn compute_harmonious_palette(images: &[RgbImage]) -> ([u8; 3], [u8; 3]) {
    let mut total_r = 0u64;
    let mut total_g = 0u64;
    let mut total_b = 0u64;
    let mut count = 0u64;

    for img in images {
        let step_x = (img.width() / 16).max(1);
        let step_y = (img.height() / 16).max(1);
        for y in (0..img.height()).step_by(step_y as usize) {
            for x in (0..img.width()).step_by(step_x as usize) {
                let p = img.get_pixel(x, y);
                total_r += p[0] as u64;
                total_g += p[1] as u64;
                total_b += p[2] as u64;
                count += 1;
            }
        }
    }

    if count == 0 {
        return ([20, 24, 34], [10, 12, 18]);
    }

    let avg_r = (total_r / count) as f32;
    let avg_g = (total_g / count) as f32;
    let avg_b = (total_b / count) as f32;

    let luminance = 0.299 * avg_r + 0.587 * avg_g + 0.114 * avg_b;
    let target_lum = 34.0;
    let lum_factor = if luminance > 0.0 {
        (target_lum / luminance).min(1.2)
    } else {
        1.0
    };

    let base_r = (avg_r * 0.35 + luminance * 0.65) * lum_factor;
    let base_g = (avg_g * 0.35 + luminance * 0.65) * lum_factor;
    let base_b = (avg_b * 0.35 + luminance * 0.65) * lum_factor;

    let c1 = [
        (base_r * 1.15).clamp(16.0, 70.0) as u8,
        (base_g * 1.15).clamp(18.0, 75.0) as u8,
        (base_b * 1.15).clamp(26.0, 90.0) as u8,
    ];

    let c2 = [
        (base_r * 0.50).clamp(8.0, 40.0) as u8,
        (base_g * 0.50).clamp(10.0, 45.0) as u8,
        (base_b * 0.50).clamp(14.0, 55.0) as u8,
    ];

    (c1, c2)
}

/// Create smooth ambient canvas with subtle color fluctuation
pub fn create_styled_background(
    w: u32,
    h: u32,
    c1: [u8; 3],
    c2: [u8; 3],
    rng: &mut impl Rng,
) -> RgbImage {
    let mut bg = RgbImage::new(w, h);
    let wf = w as f32;
    let hf = h as f32;
    let cx = wf * rng.gen_range(0.45..0.55);
    let cy = hf * rng.gen_range(0.45..0.55);
    let max_dist = (cx * cx + cy * cy).sqrt();

    for (x, y, pixel) in bg.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let dist = (dx * dx + dy * dy).sqrt() / max_dist;
        let t = dist.clamp(0.0, 1.0);

        let wobble = (((x as f32) * 0.003).sin() * ((y as f32) * 0.003).cos()) * 0.05;
        let blend = (t + wobble).clamp(0.0, 1.0);

        let r = (c1[0] as f32 * (1.0 - blend) + c2[0] as f32 * blend) as u8;
        let g = (c1[1] as f32 * (1.0 - blend) + c2[1] as f32 * blend) as u8;
        let b = (c1[2] as f32 * (1.0 - blend) + c2[2] as f32 * blend) as u8;

        *pixel = Rgb([r, g, b]);
    }
    bg
}

#[derive(Debug, Clone, Copy)]
pub enum FrameStyle {
    Polaroid,
    WoodFrame,
    ThinWhite,
    None,
}

/// Frame decoration around an image without cropping its content
pub fn decorate_photo_card(src: &RgbImage, style: FrameStyle) -> RgbaImage {
    let sw = src.width();
    let sh = src.height();

    let (pad_top, pad_bot, pad_sides, border_color) = match style {
        FrameStyle::Polaroid => (20u32, 64u32, 20u32, [248, 248, 250, 255]),
        FrameStyle::WoodFrame => (16u32, 16u32, 16u32, [45, 35, 28, 255]),
        FrameStyle::ThinWhite => (10u32, 10u32, 10u32, [242, 242, 245, 255]),
        FrameStyle::None => (0u32, 0u32, 0u32, [0, 0, 0, 0]),
    };

    let card_w = sw + pad_sides * 2;
    let card_h = sh + pad_top + pad_bot;

    let mut card = RgbaImage::from_pixel(card_w, card_h, Rgba(border_color));

    if let FrameStyle::WoodFrame = style {
        for y in 0..card_h {
            for x in 0..card_w {
                let edge = x == 4 || x == card_w - 5 || y == 4 || y == card_h - 5;
                if edge {
                    card.put_pixel(x, y, Rgba([185, 155, 95, 255]));
                }
            }
        }
    }

    for y in 0..sh {
        for x in 0..sw {
            let p = src.get_pixel(x, y);
            card.put_pixel(x + pad_sides, y + pad_top, Rgba([p[0], p[1], p[2], 255]));
        }
    }

    card
}

/// Bilinear rotation with alpha channel. Angles are kept gentle (-14 deg to +14 deg).
pub fn rotate_rgba(src: &RgbaImage, angle_rad: f32) -> RgbaImage {
    let sw = src.width() as f32;
    let sh = src.height() as f32;
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    let nw = (sw * cos_a.abs() + sh * sin_a.abs()).ceil() as u32;
    let nh = (sw * sin_a.abs() + sh * cos_a.abs()).ceil() as u32;

    let mut dest = RgbaImage::new(nw, nh);
    let src_cx = sw / 2.0;
    let src_cy = sh / 2.0;
    let dest_cx = nw as f32 / 2.0;
    let dest_cy = nh as f32 / 2.0;

    for dy in 0..nh {
        let y_diff = dy as f32 - dest_cy;
        for dx in 0..nw {
            let x_diff = dx as f32 - dest_cx;
            let sx = x_diff * cos_a + y_diff * sin_a + src_cx;
            let sy = -x_diff * sin_a + y_diff * cos_a + src_cy;

            if sx >= 0.0 && sx < sw - 1.0 && sy >= 0.0 && sy < sh - 1.0 {
                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let x1 = x0 + 1;
                let y1 = y0 + 1;
                let fx = sx - x0 as f32;
                let fy = sy - y0 as f32;

                let p00 = src.get_pixel(x0, y0);
                let p10 = src.get_pixel(x1, y0);
                let p01 = src.get_pixel(x0, y1);
                let p11 = src.get_pixel(x1, y1);

                let mut rgba = [0u8; 4];
                for i in 0..4 {
                    let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
                    let bot = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
                    rgba[i] = (top * (1.0 - fy) + bot * fy).clamp(0.0, 255.0) as u8;
                }
                dest.put_pixel(dx, dy, Rgba(rgba));
            }
        }
    }

    dest
}

/// Blends RGBA image with alpha and soft drop shadow onto RGB canvas
pub fn blend_tilted_card_with_shadow(
    canvas: &mut RgbImage,
    card: &RgbaImage,
    pos_x: i32,
    pos_y: i32,
) {
    let cw = canvas.width() as i32;
    let ch = canvas.height() as i32;
    let kw = card.width() as i32;
    let kh = card.height() as i32;

    let shadow_ox = 18i32;
    let shadow_oy = 26i32;
    for y in 0..kh {
        for x in 0..kw {
            let alpha = card.get_pixel(x as u32, y as u32)[3];
            if alpha > 25 {
                let target_x = pos_x + x + shadow_ox;
                let target_y = pos_y + y + shadow_oy;
                if target_x >= 0 && target_x < cw && target_y >= 0 && target_y < ch {
                    let p = canvas.get_pixel_mut(target_x as u32, target_y as u32);
                    let shadow_factor = (alpha as f32 / 255.0) * 0.45;
                    p[0] = (p[0] as f32 * (1.0 - shadow_factor)) as u8;
                    p[1] = (p[1] as f32 * (1.0 - shadow_factor)) as u8;
                    p[2] = (p[2] as f32 * (1.0 - shadow_factor)) as u8;
                }
            }
        }
    }

    for y in 0..kh {
        for x in 0..kw {
            let src_p = card.get_pixel(x as u32, y as u32);
            let alpha = src_p[3] as f32 / 255.0;
            if alpha > 0.01 {
                let target_x = pos_x + x;
                let target_y = pos_y + y;
                if target_x >= 0 && target_x < cw && target_y >= 0 && target_y < ch {
                    let bg_p = canvas.get_pixel_mut(target_x as u32, target_y as u32);
                    bg_p[0] = (src_p[0] as f32 * alpha + bg_p[0] as f32 * (1.0 - alpha)) as u8;
                    bg_p[1] = (src_p[1] as f32 * alpha + bg_p[1] as f32 * (1.0 - alpha)) as u8;
                    bg_p[2] = (src_p[2] as f32 * alpha + bg_p[2] as f32 * (1.0 - alpha)) as u8;
                }
            }
        }
    }
}

/// Mode B: Natural scattered collage where portrait-heavy photos are comfortably spaced,
/// with peripheral photos intentionally overhanging/bleeding offscreen to suppress empty background.
pub fn compose_collage(images: &[PathBuf]) -> Result<RgbImage, String> {
    if images.is_empty() {
        return Err("No images provided for collage".to_string());
    }

    let mut loaded = Vec::new();
    for p in images {
        if let Ok(src) = load_rgb_image(p) {
            loaded.push(src);
        }
    }
    if loaded.is_empty() {
        return Err("Failed to load any valid images for collage".to_string());
    }

    let mut rng = rand::thread_rng();
    let (c1, c2) = compute_harmonious_palette(&loaded);
    let mut canvas = create_styled_background(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K, c1, c2, &mut rng);

    let frame_styles = [
        FrameStyle::Polaroid,
        FrameStyle::ThinWhite,
        FrameStyle::WoodFrame,
        FrameStyle::None,
    ];

    // Staggered layout anchors designed for portrait-heavy collections.
    // Anchors near the outer boundaries (x < 350 or x > 3500, y < 250 or y > 1900)
    // intentionally bleed outside the 4K canvas to eliminate empty background margins.
    let layout_anchors = [
        // Perimeter overhang layer (bleeds off canvas edges)
        (200i32, 350i32),
        (1050i32, 220i32),
        (1920i32, 180i32),
        (2790i32, 220i32),
        (3640i32, 350i32),
        (200i32, 1800i32),
        (1050i32, 1940i32),
        (1920i32, 1980i32),
        (2790i32, 1940i32),
        (3640i32, 1800i32),
        // Mid-field hero layer (gently overlapping across screen center)
        (650i32, 1080i32),
        (1450i32, 1050i32),
        (2390i32, 1050i32),
        (3190i32, 1080i32),
        (1920i32, 1080i32),
    ];

    let mut photo_indices: Vec<usize> = (0..loaded.len()).collect();
    photo_indices.shuffle(&mut rng);

    for (i, &(anchor_x, anchor_y)) in layout_anchors.iter().enumerate() {
        let src = &loaded[photo_indices[i % photo_indices.len()]];

        // Sizing: Portraits look best scaled to long-edge ~1200-1380px
        let max_dim = rng.gen_range(1150..=1380);
        let scaled = fit_downscale_only(src, max_dim);

        let style = *frame_styles.choose(&mut rng).unwrap();
        let card = decorate_photo_card(&scaled, style);

        // Gentle tilt alternating between left and right (-12 deg to +12 deg)
        let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
        let angle_deg = sign * rng.gen_range(3.5..11.5);
        let rotated = rotate_rgba(&card, angle_deg * (std::f32::consts::PI / 180.0));

        let jx = rng.gen_range(-50..50);
        let jy = rng.gen_range(-40..40);
        let px = anchor_x + jx - (rotated.width() as i32 / 2);
        let py = anchor_y + jy - (rotated.height() as i32 / 2);

        blend_tilted_card_with_shadow(&mut canvas, &rotated, px, py);
    }

    Ok(canvas)
}

/// Mode C: Center-Outward Dynamic Tiling with larger base tile dimensions (~1050-1450px)
/// and fewer total tiles (~8-12 tiles), giving each image generous presence with zero internal cropping.
pub fn compose_tiling(images: &[PathBuf]) -> Result<RgbImage, String> {
    if images.is_empty() {
        return Err("No images provided for tiling".to_string());
    }

    let mut loaded = Vec::new();
    for p in images {
        if let Ok(src) = load_rgb_image(p) {
            loaded.push(src);
        }
    }
    if loaded.is_empty() {
        return Err("Failed to load any valid images for tiling".to_string());
    }

    let mut rng = rand::thread_rng();
    let (c1, c2) = compute_harmonious_palette(&loaded);
    let mut canvas = create_styled_background(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K, c1, c2, &mut rng);

    let gap = 16i32;
    let canvas_cx = (CANVAS_WIDTH_4K / 2) as i32;
    let canvas_cy = (CANVAS_HEIGHT_4K / 2) as i32;

    struct PlacedImage {
        rect: RectI32,
        img: RgbImage,
    }

    let mut placed: Vec<PlacedImage> = Vec::new();
    let mut placed_rects: Vec<RectI32> = Vec::new();

    // 1. Place initial center hero image with large presence (~1250px)
    let first_src = &loaded[0];
    let first_scaled = fit_downscale_only(first_src, 1250);
    let fw = first_scaled.width() as i32;
    let fh = first_scaled.height() as i32;
    let first_rect = RectI32 {
        x: canvas_cx - fw / 2,
        y: canvas_cy - fh / 2,
        w: fw,
        h: fh,
    };
    placed_rects.push(first_rect);
    placed.push(PlacedImage {
        rect: first_rect,
        img: first_scaled,
    });

    let mut img_idx = 1;
    let mut attempts = 0;

    let overlaps_any = |cand: &RectI32, list: &[RectI32]| -> bool {
        let expanded = RectI32 {
            x: cand.x - gap,
            y: cand.y - gap,
            w: cand.w + gap * 2,
            h: cand.h + gap * 2,
        };
        for r in list {
            if expanded.intersects(r) {
                return true;
            }
        }
        false
    };

    let is_canvas_fully_covered = |list: &[RectI32]| -> bool {
        let check_cols = 12i32;
        let check_rows = 7i32;
        let step_x = (CANVAS_WIDTH_4K as i32) / check_cols;
        let step_y = (CANVAS_HEIGHT_4K as i32) / check_rows;

        for c in 0..check_cols {
            let px = c * step_x + step_x / 2;
            for r in 0..check_rows {
                let py = r * step_y + step_y / 2;
                let covered = list.iter().any(|rect| rect.contains(px, py));
                if !covered {
                    return false;
                }
            }
        }
        true
    };

    // Continue packing larger tiles outwards until 4K screen is covered
    while attempts < 35 {
        attempts += 1;

        if placed.len() >= 9 && is_canvas_fully_covered(&placed_rects) {
            break;
        }

        let src = &loaded[img_idx % loaded.len()];
        img_idx += 1;

        // Larger tile scale (~1050 - 1420px) reduces total tile count and increases impact
        let max_dim = rng.gen_range(1050..=1420);
        let scaled = fit_downscale_only(src, max_dim);
        let w = scaled.width() as i32;
        let h = scaled.height() as i32;

        let mut candidates = Vec::new();
        for r in &placed_rects {
            // Right of r
            candidates.push(RectI32 {
                x: r.right() + gap,
                y: r.y,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() + gap,
                y: r.bottom() - h,
                w,
                h,
            });
            // Left of r
            candidates.push(RectI32 {
                x: r.x - gap - w,
                y: r.y,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.x - gap - w,
                y: r.bottom() - h,
                w,
                h,
            });
            // Above r
            candidates.push(RectI32 {
                x: r.x,
                y: r.y - gap - h,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() - w,
                y: r.y - gap - h,
                w,
                h,
            });
            // Below r
            candidates.push(RectI32 {
                x: r.x,
                y: r.bottom() + gap,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() - w,
                y: r.bottom() + gap,
                w,
                h,
            });
        }

        candidates.sort_by_key(|c| {
            let cx = c.x + c.w / 2;
            let cy = c.y + c.h / 2;
            let dx = cx - canvas_cx;
            let dy = cy - canvas_cy;
            (dx * dx + dy * dy) as u64
        });

        if let Some(best) = candidates
            .into_iter()
            .find(|c| !overlaps_any(c, &placed_rects))
        {
            placed_rects.push(best);
            placed.push(PlacedImage {
                rect: best,
                img: scaled,
            });
        }
    }

    // Render placed images onto canvas, clipping only at canvas outer boundaries
    let cw = CANVAS_WIDTH_4K as i32;
    let ch = CANVAS_HEIGHT_4K as i32;

    for item in placed {
        let rx = item.rect.x;
        let ry = item.rect.y;
        let rw = item.rect.w;
        let rh = item.rect.h;

        let start_x = rx.max(0);
        let end_x = (rx + rw).min(cw);
        let start_y = ry.max(0);
        let end_y = (ry + rh).min(ch);

        if start_x < end_x && start_y < end_y {
            for y in start_y..end_y {
                let local_y = (y - ry) as u32;
                for x in start_x..end_x {
                    let local_x = (x - rx) as u32;
                    let p = item.img.get_pixel(local_x, local_y);
                    canvas.put_pixel(x as u32, y as u32, *p);
                }
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
        let use_tiling = rng.gen_bool(0.5);
        if use_tiling {
            let count = 14.min(images.len());
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_tiling(&selected)?;
            (CompositionMode::Tiling, img, selected)
        } else {
            let count = 15.min(images.len());
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
            let count = images.len().min(15);
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_collage(&selected)?;
            (img, selected)
        }
        CompositionMode::Tiling => {
            let count = images.len().min(14);
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
        let (c1, c2) = ([40, 45, 60], [15, 18, 25]);
        let mut rng = rand::thread_rng();
        let bg = create_styled_background(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K, c1, c2, &mut rng);
        assert_eq!(bg.width(), 3840);
        assert_eq!(bg.height(), 2160);
    }

    #[test]
    fn test_fit_downscale_only() {
        let src = RgbImage::from_pixel(1000, 2000, Rgb([255, 0, 0]));
        let downscaled = fit_downscale_only(&src, 800);
        assert_eq!(downscaled.width(), 400);
        assert_eq!(downscaled.height(), 800);

        let small = RgbImage::from_pixel(300, 400, Rgb([0, 255, 0]));
        let kept = fit_downscale_only(&small, 800);
        assert_eq!(kept.width(), 300);
        assert_eq!(kept.height(), 400);
    }

    #[test]
    fn test_rotate_rgba_dimensions() {
        let card = RgbaImage::from_pixel(200, 300, Rgba([255, 255, 255, 255]));
        let rotated = rotate_rgba(&card, 0.15);
        assert!(rotated.width() >= 200);
        assert!(rotated.height() >= 300);
    }
}
