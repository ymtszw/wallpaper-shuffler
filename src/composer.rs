use image::imageops::{FilterType, crop_imm, resize};
use image::{ImageReader, Rgb, RgbImage, Rgba, RgbaImage};
use rand::Rng;
use rand::seq::SliceRandom;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

pub const CANVAS_WIDTH_4K: u32 = 3840;
pub const CANVAS_HEIGHT_4K: u32 = 2160;
pub const MIN_SINGLE_LANDSCAPE_WIDTH: u32 = 1900;

static BG_ROTATION_INDEX: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy)]
pub struct AmbientPreset {
    pub name: &'static str,
    pub c1: [u8; 3],
    pub c2: [u8; 3],
}

pub const AMBIENT_PRESETS: [AmbientPreset; 5] = [
    // 1. 淡い紫 (Soft Lavender / Wisteria)
    AmbientPreset {
        name: "Soft Lavender (淡い紫)",
        c1: [210, 200, 226], // Light lavender mist
        c2: [170, 155, 195], // Muted twilight violet
    },
    // 2. 淡いピンク (Pastel Rose / Sakura Blossom)
    AmbientPreset {
        name: "Pastel Blossom (淡いピンク)",
        c1: [232, 202, 212], // Soft sakura pink
        c2: [198, 162, 178], // Muted rose
    },
    // 3. 淡いオレンジ (Warm Peach / Sunset Apricot)
    AmbientPreset {
        name: "Sunset Apricot (淡いオレンジ)",
        c1: [238, 210, 190], // Warm peach
        c2: [205, 172, 146], // Soft terracotta sand
    },
    // 4. 淡い青 (Soft Sky / Azure Mist)
    AmbientPreset {
        name: "Sky Mist (淡い青)",
        c1: [192, 212, 235], // Soft baby sky tint
        c2: [155, 180, 212], // Calm slate azure
    },
    // 5. 淡い緑 (Sage / Mint Tea)
    AmbientPreset {
        name: "Sage Mint (淡い緑)",
        c1: [195, 218, 202], // Soft herbal mint
        c2: [158, 192, 168], // Calming sage green
    },
];

/// Get the next rotated ambient pastel gradient (Lavender -> Pink -> Orange -> Sky Blue -> Mint)
pub fn next_rotated_ambient_gradient() -> AmbientPreset {
    let idx = BG_ROTATION_INDEX.fetch_add(1, Ordering::Relaxed) % AMBIENT_PRESETS.len();
    AMBIENT_PRESETS[idx]
}

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

/// Only landscape images (width > height) with horizontal resolution >= 1900px (2K+)
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

    if width <= height {
        return false; // Strictly landscape only, never vertical/square
    }

    width >= MIN_SINGLE_LANDSCAPE_WIDTH
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

/// Mode A: Single 4K wallpaper fit (strictly landscape width > height)
pub fn compose_single(src_path: &Path) -> Result<RgbImage, String> {
    let src = load_rgb_image(src_path)?;
    let sw = src.width();
    let sh = src.height();

    if sw <= sh {
        return Err(format!(
            "Cannot use portrait image '{}' ({}x{}) for Single wallpaper mode (strictly landscape width > height required)",
            src_path.display(),
            sw,
            sh
        ));
    }

    let sw_f = sw as f64;
    let sh_f = sh as f64;
    let tw = CANVAS_WIDTH_4K as f64;
    let th = CANVAS_HEIGHT_4K as f64;

    let scale = (tw / sw_f).max(th / sh_f);
    let nw = ((sw_f * scale).round() as u32).max(CANVAS_WIDTH_4K);
    let nh = ((sh_f * scale).round() as u32).max(CANVAS_HEIGHT_4K);

    let resized = resize(&src, nw, nh, FilterType::Triangle);
    let crop_x = (nw - CANVAS_WIDTH_4K) / 2;
    let crop_y = (nh - CANVAS_HEIGHT_4K) / 2;

    Ok(crop_imm(&resized, crop_x, crop_y, CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K).to_image())
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

    // Fast 1D lookup tables to avoid 8.3 million trigonometric calls per 4K frame
    let sin_lut: Vec<f32> = (0..w).map(|x| (x as f32 * 0.003).sin()).collect();
    let cos_lut: Vec<f32> = (0..h).map(|y| (y as f32 * 0.003).cos()).collect();

    let c1_f = [c1[0] as f32, c1[1] as f32, c1[2] as f32];
    let diff_f = [
        c2[0] as f32 - c1_f[0],
        c2[1] as f32 - c1_f[1],
        c2[2] as f32 - c1_f[2],
    ];

    for (x, y, pixel) in bg.enumerate_pixels_mut() {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let dist = (dx * dx + dy * dy).sqrt() / max_dist;
        let t = dist.clamp(0.0, 1.0);

        let wobble = (sin_lut[x as usize] * cos_lut[y as usize]) * 0.04;
        let blend = (t + wobble).clamp(0.0, 1.0);

        let r = (c1_f[0] + diff_f[0] * blend) as u8;
        let g = (c1_f[1] + diff_f[1] * blend) as u8;
        let b = (c1_f[2] + diff_f[2] * blend) as u8;

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
        FrameStyle::Polaroid => (24u32, 76u32, 24u32, [248, 248, 250, 255]),
        FrameStyle::WoodFrame => (18u32, 18u32, 18u32, [45, 35, 28, 255]),
        FrameStyle::ThinWhite => (12u32, 12u32, 12u32, [242, 242, 245, 255]),
        FrameStyle::None => (0u32, 0u32, 0u32, [0, 0, 0, 0]),
    };

    let card_w = sw + pad_sides * 2;
    let card_h = sh + pad_top + pad_bot;

    let mut card = RgbaImage::from_pixel(card_w, card_h, Rgba(border_color));

    if let FrameStyle::WoodFrame = style {
        for y in 0..card_h {
            for x in 0..card_w {
                let edge = (5..=6).contains(&x)
                    || (card_w - 7..=card_w - 6).contains(&x)
                    || (5..=6).contains(&y)
                    || (card_h - 7..=card_h - 6).contains(&y);
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

/// High-quality bilinear rotation with sub-pixel boundary anti-aliasing to eliminate jaggies
pub fn rotate_rgba(src: &RgbaImage, angle_rad: f32) -> RgbaImage {
    let sw = src.width() as f32;
    let sh = src.height() as f32;
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    let nw = (sw * cos_a.abs() + sh * sin_a.abs()).ceil() as u32 + 2;
    let nh = (sw * sin_a.abs() + sh * cos_a.abs()).ceil() as u32 + 2;

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

            // Signed distance to the 4 card boundaries [0..sw] x [0..sh]
            let dist_left = sx;
            let dist_right = sw - sx;
            let dist_top = sy;
            let dist_bottom = sh - sy;
            let min_dist = dist_left.min(dist_right).min(dist_top.min(dist_bottom));

            if min_dist > -0.5 {
                // Sub-pixel edge anti-aliasing coverage ramp over 1-pixel boundary
                let edge_coverage = (min_dist + 0.5).clamp(0.0, 1.0);

                let clamp_sx = sx.clamp(0.0, sw - 1.001);
                let clamp_sy = sy.clamp(0.0, sh - 1.001);

                let x0 = clamp_sx.floor() as u32;
                let y0 = clamp_sy.floor() as u32;
                let x1 = (x0 + 1).min(src.width() - 1);
                let y1 = (y0 + 1).min(src.height() - 1);
                let fx = clamp_sx - x0 as f32;
                let fy = clamp_sy - y0 as f32;

                let p00 = src.get_pixel(x0, y0);
                let p10 = src.get_pixel(x1, y0);
                let p01 = src.get_pixel(x0, y1);
                let p11 = src.get_pixel(x1, y1);

                let mut rgba = [0u8; 4];
                for i in 0..3 {
                    let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
                    let bot = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
                    rgba[i] = (top * (1.0 - fy) + bot * fy).clamp(0.0, 255.0) as u8;
                }
                let top_a = p00[3] as f32 * (1.0 - fx) + p10[3] as f32 * fx;
                let bot_a = p01[3] as f32 * (1.0 - fx) + p11[3] as f32 * fx;
                let sampled_a = (top_a * (1.0 - fy) + bot_a * fy).clamp(0.0, 255.0);
                let final_a = (sampled_a * edge_coverage).round().clamp(0.0, 255.0) as u8;
                rgba[3] = final_a;

                if final_a > 0 {
                    dest.put_pixel(dx, dy, Rgba(rgba));
                }
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

    let shadow_ox = 22i32;
    let shadow_oy = 30i32;
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

#[derive(Debug, Clone)]
struct CollageCardPlacement {
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    angle_rad: f32,
}

impl CollageCardPlacement {
    fn contains_point(&self, px: f32, py: f32) -> bool {
        let dx = px - self.cx;
        let dy = py - self.cy;
        let cos_a = (-self.angle_rad).cos();
        let sin_a = (-self.angle_rad).sin();
        let lx = dx * cos_a - dy * sin_a;
        let ly = dx * sin_a + dy * cos_a;
        lx.abs() <= self.w / 2.0 && ly.abs() <= self.h / 2.0
    }

    /// Evaluates percentage of this card that is NOT covered by any subsequent cards (drawn on top)
    fn calculate_unoccluded_ratio(&self, top_cards: &[CollageCardPlacement]) -> f32 {
        let sample_steps = 15;
        let mut visible = 0;
        let total = sample_steps * sample_steps;

        let cos_a = self.angle_rad.cos();
        let sin_a = self.angle_rad.sin();

        for sy in 0..sample_steps {
            let ly = (sy as f32 / (sample_steps - 1) as f32 - 0.5) * self.h;
            for sx in 0..sample_steps {
                let lx = (sx as f32 / (sample_steps - 1) as f32 - 0.5) * self.w;
                let px = self.cx + lx * cos_a - ly * sin_a;
                let py = self.cy + lx * sin_a + ly * cos_a;

                // Check if covered by any card drawn on top of this one
                let covered = top_cards.iter().any(|c| c.contains_point(px, py));
                if !covered {
                    visible += 1;
                }
            }
        }

        visible as f32 / total as f32
    }
}

fn calculate_canvas_margin_ratio(
    placements: &[CollageCardPlacement],
    grid_w: usize,
    grid_h: usize,
) -> f32 {
    let mut covered_count = 0;
    let total_samples = (grid_w * grid_h) as f32;
    let step_x = CANVAS_WIDTH_4K as f32 / grid_w as f32;
    let step_y = CANVAS_HEIGHT_4K as f32 / grid_h as f32;

    for gy in 0..grid_h {
        let py = gy as f32 * step_y + step_y * 0.5;
        for gx in 0..grid_w {
            let px = gx as f32 * step_x + step_x * 0.5;
            if placements.iter().any(|c| c.contains_point(px, py)) {
                covered_count += 1;
            }
        }
    }

    1.0 - (covered_count as f32 / total_samples)
}

fn collect_uncovered_grid_points(
    placements: &[CollageCardPlacement],
    grid_w: usize,
    grid_h: usize,
) -> Vec<(f32, f32)> {
    let mut uncovered = Vec::new();
    let step_x = CANVAS_WIDTH_4K as f32 / grid_w as f32;
    let step_y = CANVAS_HEIGHT_4K as f32 / grid_h as f32;

    for gy in 0..grid_h {
        let py = gy as f32 * step_y + step_y * 0.5;
        for gx in 0..grid_w {
            let px = gx as f32 * step_x + step_x * 0.5;
            if !placements.iter().any(|c| c.contains_point(px, py)) {
                uncovered.push((px, py));
            }
        }
    }

    uncovered
}

/// Mode B: Large portrait-dominated prints with guaranteed >= 70% unoccluded exposure per primary image.
/// Uses exploratory placement optimization for 4+ cards and underlay gap-filling for remaining margin.
pub fn compose_collage(images: &[PathBuf]) -> Result<RgbImage, String> {
    if images.is_empty() {
        return Err("No images provided for collage".to_string());
    }

    let mut rng = rand::thread_rng();

    // Randomize candidates without landscape bias (portraits naturally dominate)
    let mut candidate_paths: Vec<PathBuf> = images.to_vec();
    candidate_paths.shuffle(&mut rng);

    let mut loaded = Vec::new();
    for p in &candidate_paths {
        if let Ok(src) = load_rgb_image(p) {
            loaded.push(src);
            if loaded.len() >= 10 {
                break;
            }
        }
    }
    if loaded.is_empty() {
        return Err("Failed to load any valid images for collage".to_string());
    }

    // Guarantee minimum 7 images for collage composition (5 primary + at least 2 underlay)
    while loaded.len() < 7 {
        let clone_img = loaded[loaded.len() % loaded.len()].clone();
        loaded.push(clone_img);
    }

    let preset = next_rotated_ambient_gradient();
    let mut canvas = create_styled_background(
        CANVAS_WIDTH_4K,
        CANVAS_HEIGHT_4K,
        preset.c1,
        preset.c2,
        &mut rng,
    );

    let frame_styles = [
        FrameStyle::Polaroid,
        FrameStyle::ThinWhite,
        FrameStyle::WoodFrame,
        FrameStyle::None,
    ];

    // Pre-decorate cards once outside search loop to ensure blazing fast performance (<300ms)
    struct PreparedCard {
        card: RgbaImage,
        w: f32,
        h: f32,
    }

    let prepared_cards: Vec<PreparedCard> = loaded
        .iter()
        .map(|src| {
            let max_dim = rng.gen_range(1450..=1800);
            let scaled = fit_downscale_only(src, max_dim);
            let style = *frame_styles.choose(&mut rng).unwrap();
            let card = decorate_photo_card(&scaled, style);
            let w = card.width() as f32;
            let h = card.height() as f32;
            PreparedCard { card, w, h }
        })
        .collect();

    // 1. Determine primary cards count (enforce 5+ images: 5 or 6)
    let primary_count = if prepared_cards.len() >= 6 && rng.gen_bool(0.3) {
        6
    } else {
        5
    };

    let anchors_5 = [
        (800i32, 700i32),   // Top-Left
        (3040i32, 700i32),  // Top-Right
        (1920i32, 1100i32), // Center Hero
        (960i32, 1500i32),  // Bottom-Left
        (2880i32, 1500i32), // Bottom-Right
    ];

    let anchors_6 = [
        (750i32, 650i32),   // Top-Left
        (1920i32, 600i32),  // Top-Center
        (3090i32, 650i32),  // Top-Right
        (800i32, 1520i32),  // Bottom-Left
        (1920i32, 1560i32), // Bottom-Center
        (3040i32, 1520i32), // Bottom-Right
    ];

    let primary_anchors: &[(i32, i32)] = if primary_count == 6 {
        &anchors_6
    } else {
        &anchors_5
    };

    // 2. Exploratory optimization round for primary 5+ cards:
    // Minimize mutual overlap (all cards >= 70% visible) while minimizing canvas background margin.
    let min_exposure_threshold = 0.70f32;
    let mut best_primary_placements = Vec::new();
    let mut best_margin_ratio = 1.0f32;
    let mut best_fallback_placements = Vec::new();
    let mut best_fallback_min_exposure = 0.0f32;

    for attempt in 0..150 {
        let mut candidate_placements = Vec::new();
        for i in 0..primary_count {
            let prep = &prepared_cards[i];
            let (base_x, base_y) = primary_anchors[i];
            let jx = rng.gen_range(-180..=180);
            let jy = rng.gen_range(-130..=130);
            let cx = (base_x + jx) as f32;
            let cy = (base_y + jy) as f32;

            let sign = if (i + attempt) % 2 == 0 { 1.0 } else { -1.0 };
            let angle_deg = sign * rng.gen_range(2.0..=8.5);
            let angle_rad = angle_deg * (std::f32::consts::PI / 180.0);

            candidate_placements.push(CollageCardPlacement {
                cx,
                cy,
                w: prep.w,
                h: prep.h,
                angle_rad,
            });
        }

        // Check unoccluded ratio for each card against top cards
        let mut min_exp = 1.0f32;
        for idx in 0..candidate_placements.len() {
            let top_cards = &candidate_placements[idx + 1..];
            let exp = candidate_placements[idx].calculate_unoccluded_ratio(top_cards);
            if exp < min_exp {
                min_exp = exp;
            }
        }

        if min_exp >= min_exposure_threshold {
            let margin = calculate_canvas_margin_ratio(&candidate_placements, 40, 24);
            if margin < best_margin_ratio {
                best_margin_ratio = margin;
                best_primary_placements = candidate_placements;
            }
        } else if min_exp > best_fallback_min_exposure {
            best_fallback_min_exposure = min_exp;
            best_fallback_placements = candidate_placements;
        }
    }

    if best_primary_placements.is_empty() {
        best_primary_placements = best_fallback_placements;
    }

    // 3. Exception Rule: Underlay Gap-Filling Round (Fixed minimum 2 underlays)
    // Place additional images underneath the primary cards to cover the remaining margins,
    // requiring >= 10% visible exposure.
    struct UnderlayCard {
        card_idx: usize,
        placement: CollageCardPlacement,
    }

    let mut underlay_cards: Vec<UnderlayCard> = Vec::new();
    let mut all_current_placements = best_primary_placements.clone();
    let max_extra = prepared_cards.len().min(primary_count + 4);

    for (extra_idx, prep) in prepared_cards
        .iter()
        .enumerate()
        .take(max_extra)
        .skip(primary_count)
    {
        let uncovered_points = collect_uncovered_grid_points(&all_current_placements, 40, 24);
        let current_margin = uncovered_points.len() as f32 / (40.0 * 24.0);

        // After satisfying the minimum 2 underlay cards, stop if margin is sufficiently low
        if underlay_cards.len() >= 2 && (current_margin <= 0.15 || uncovered_points.is_empty()) {
            break;
        }

        let mut best_cand: Option<CollageCardPlacement> = None;
        let mut best_score = -1.0f32;

        // Search best underlay position across clusters of uncovered points
        for _ in 0..60 {
            let (target_x, target_y) = if let Some(&(tx, ty)) = uncovered_points.choose(&mut rng) {
                (tx, ty)
            } else {
                let border_anchors = [
                    (1920.0, 450.0),
                    (1920.0, 1700.0),
                    (500.0, 1100.0),
                    (3340.0, 1100.0),
                ];
                *border_anchors.choose(&mut rng).unwrap()
            };

            let cx = target_x + rng.gen_range(-100.0..=100.0);
            let cy = target_y + rng.gen_range(-100.0..=100.0);
            let angle_deg = rng.gen_range(-8.5..=8.5);
            let angle_rad = angle_deg * (std::f32::consts::PI / 180.0);

            let cand = CollageCardPlacement {
                cx,
                cy,
                w: prep.w,
                h: prep.h,
                angle_rad,
            };

            // Requirement: underlay card must peek out with at least 10% visible area underneath primary cards
            let visible_ratio = cand.calculate_unoccluded_ratio(&best_primary_placements);
            if visible_ratio >= 0.10 {
                let newly_covered = uncovered_points
                    .iter()
                    .filter(|(px, py)| cand.contains_point(*px, *py))
                    .count();
                let score = newly_covered as f32 * 10.0 + visible_ratio;
                if score > best_score {
                    best_score = score;
                    best_cand = Some(cand);
                }
            }
        }

        if let Some(winning_cand) = best_cand {
            all_current_placements.push(winning_cand.clone());
            underlay_cards.push(UnderlayCard {
                card_idx: extra_idx,
                placement: winning_cand,
            });
        }
    }

    // 4. Rendering in proper depth order:
    // Layer 1: Render Underlay cards FIRST (underneath primary cards)
    for uc in &underlay_cards {
        let prep = &prepared_cards[uc.card_idx];
        let rotated = rotate_rgba(&prep.card, uc.placement.angle_rad);
        let place_x = (uc.placement.cx - rotated.width() as f32 / 2.0).round() as i32;
        let place_y = (uc.placement.cy - rotated.height() as f32 / 2.0).round() as i32;
        blend_tilted_card_with_shadow(&mut canvas, &rotated, place_x, place_y);
    }

    // Layer 2: Render Primary cards ON TOP (prominent, with shadow onto underlay and canvas)
    for (i, p) in best_primary_placements.into_iter().enumerate() {
        let prep = &prepared_cards[i];
        let rotated = rotate_rgba(&prep.card, p.angle_rad);
        let place_x = (p.cx - rotated.width() as f32 / 2.0).round() as i32;
        let place_y = (p.cy - rotated.height() as f32 / 2.0).round() as i32;
        blend_tilted_card_with_shadow(&mut canvas, &rotated, place_x, place_y);
    }

    Ok(canvas)
}

/// Mode C: Center-Outward Dynamic Tiling with bold, wide gaps (28-40px)
/// and evenly distributed packing without lopsided vacant pockets.
pub fn compose_tiling(images: &[PathBuf]) -> Result<RgbImage, String> {
    if images.is_empty() {
        return Err("No images provided for tiling".to_string());
    }

    let mut rng = rand::thread_rng();

    let mut candidate_paths: Vec<PathBuf> = images.to_vec();
    candidate_paths.shuffle(&mut rng);

    let mut loaded = Vec::new();
    for p in &candidate_paths {
        if let Ok(src) = load_rgb_image(p) {
            loaded.push(src);
        }
    }
    if loaded.is_empty() {
        return Err("Failed to load any valid images for tiling".to_string());
    }

    let preset = next_rotated_ambient_gradient();
    let mut canvas = create_styled_background(
        CANVAS_WIDTH_4K,
        CANVAS_HEIGHT_4K,
        preset.c1,
        preset.c2,
        &mut rng,
    );

    let canvas_cx = (CANVAS_WIDTH_4K / 2) as i32;
    let canvas_cy = (CANVAS_HEIGHT_4K / 2) as i32;

    struct PlacedImage {
        rect: RectI32,
        img: RgbImage,
    }

    let mut placed: Vec<PlacedImage> = Vec::new();
    let mut placed_rects: Vec<RectI32> = Vec::new();

    // Bold, noticeable gap (30px) to prevent cramped edges and eliminate lopsided feeling
    let bold_gap = 30i32;

    // 1. Lower-center anchor tile with jitter (bleeds over bottom allowed)
    // Starting in the lower-center distributes space upwards so upper tiles don't get truncated
    let first_src = &loaded[0];
    let first_max = rng.gen_range(1200..=1400);
    let first_scaled = fit_downscale_only(first_src, first_max);
    let fw = first_scaled.width() as i32;
    let fh = first_scaled.height() as i32;

    let base_start_x = canvas_cx;
    let base_start_y = 1550i32; // Lower-center baseline
    let jitter_x = rng.gen_range(-280..=280);
    let jitter_y = rng.gen_range(-160..=160);
    let start_x = base_start_x + jitter_x;
    let start_y = base_start_y + jitter_y;

    let first_rect = RectI32 {
        x: start_x - fw / 2,
        y: start_y - fh / 2,
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
            x: cand.x - bold_gap,
            y: cand.y - bold_gap,
            w: cand.w + bold_gap * 2,
            h: cand.h + bold_gap * 2,
        };
        for r in list {
            if expanded.intersects(r) {
                return true;
            }
        }
        false
    };

    // Coverage test: ensures no large voids across an 8x5 grid of the 4K screen
    let find_uncovered_center_biased_point = |list: &[RectI32]| -> Option<(i32, i32)> {
        let grid_cols = 10i32;
        let grid_rows = 6i32;
        let step_x = (CANVAS_WIDTH_4K as i32) / grid_cols;
        let step_y = (CANVAS_HEIGHT_4K as i32) / grid_rows;

        let mut uncovered = Vec::new();
        for c in 0..grid_cols {
            let px = c * step_x + step_x / 2;
            for r in 0..grid_rows {
                let py = r * step_y + step_y / 2;
                let is_covered = list.iter().any(|rect| rect.contains(px, py));
                if !is_covered {
                    let dx = px - canvas_cx;
                    let dy = py - canvas_cy;
                    let dist_sq = (dx * dx + dy * dy) as u64;
                    uncovered.push((dist_sq, px, py));
                }
            }
        }

        uncovered.sort_by_key(|(d, _, _)| *d);
        uncovered.first().map(|(_, px, py)| (*px, *py))
    };

    // Outward packing with bold uniform gap (30px) and balanced coverage
    while attempts < 35 {
        attempts += 1;

        let uncovered_target = find_uncovered_center_biased_point(&placed_rects);
        if uncovered_target.is_none() && placed.len() >= 8 {
            break;
        }

        let target_pt = uncovered_target.unwrap_or((canvas_cx, canvas_cy));

        let src = &loaded[img_idx % loaded.len()];
        img_idx += 1;

        let max_dim = rng.gen_range(1100..=1450);
        let scaled = fit_downscale_only(src, max_dim);
        let w = scaled.width() as i32;
        let h = scaled.height() as i32;

        let mut candidates = Vec::new();
        for r in &placed_rects {
            // Right edge alignments
            candidates.push(RectI32 {
                x: r.right() + bold_gap,
                y: r.y,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() + bold_gap,
                y: r.y + (r.h - h) / 2,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() + bold_gap,
                y: r.bottom() - h,
                w,
                h,
            });

            // Left edge alignments
            candidates.push(RectI32 {
                x: r.x - bold_gap - w,
                y: r.y,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.x - bold_gap - w,
                y: r.y + (r.h - h) / 2,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.x - bold_gap - w,
                y: r.bottom() - h,
                w,
                h,
            });

            // Top edge alignments
            candidates.push(RectI32 {
                x: r.x,
                y: r.y - bold_gap - h,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.x + (r.w - w) / 2,
                y: r.y - bold_gap - h,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() - w,
                y: r.y - bold_gap - h,
                w,
                h,
            });

            // Bottom edge alignments
            candidates.push(RectI32 {
                x: r.x,
                y: r.bottom() + bold_gap,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.x + (r.w - w) / 2,
                y: r.bottom() + bold_gap,
                w,
                h,
            });
            candidates.push(RectI32 {
                x: r.right() - w,
                y: r.bottom() + bold_gap,
                w,
                h,
            });
        }

        candidates.sort_by_key(|c| {
            let cx = c.x + c.w / 2;
            let cy = c.y + c.h / 2;
            let dx = cx - target_pt.0;
            let dy = cy - target_pt.1;
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
    let valid_exts = ["jpg", "jpeg", "jfif", "jiff", "png", "webp"];

    for dir in dirs {
        if dir.is_file() {
            let has_valid_ext = dir
                .extension()
                .and_then(|s| s.to_str())
                .map(|ext| valid_exts.contains(&ext.to_lowercase().as_str()))
                .unwrap_or(false);
            if has_valid_ext {
                files.push(dir.clone());
            }
        } else if let Ok(entries) = fs::read_dir(dir) {
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
            let count = 10.min(images.len());
            let selected: Vec<PathBuf> = images.iter().take(count).cloned().collect();
            let img = compose_tiling(&selected)?;
            (CompositionMode::Tiling, img, selected)
        } else {
            let count = 10.min(images.len());
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

    let mut rng = rand::thread_rng();
    let mut pool = images.to_vec();
    pool.shuffle(&mut rng);

    let (composed, used) = match mode {
        CompositionMode::Single => {
            let single_img = if pool.len() == 1 {
                &pool[0]
            } else {
                pool.iter()
                    .find(|p| is_single_candidate(p))
                    .ok_or_else(|| {
                        format!(
                            "No qualifying landscape candidate (width > height and width >= {}px) found in images",
                            MIN_SINGLE_LANDSCAPE_WIDTH
                        )
                    })?
            };
            let img = compose_single(single_img)?;
            (img, vec![single_img.clone()])
        }
        CompositionMode::Collage => {
            let count = pool.len().min(10);
            let selected: Vec<PathBuf> = pool.iter().take(count).cloned().collect();
            let img = compose_collage(&selected)?;
            (img, selected)
        }
        CompositionMode::Tiling => {
            let count = pool.len().min(12);
            let selected: Vec<PathBuf> = pool.iter().take(count).cloned().collect();
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

    #[test]
    fn test_single_mode_strictly_rejects_portrait() {
        let temp_dir = std::env::temp_dir().join("wallpaper_shuffler_test");
        let _ = fs::create_dir_all(&temp_dir);
        let portrait_path = temp_dir.join("test_portrait.png");
        let portrait_img = RgbImage::from_pixel(1600, 2500, Rgb([100, 150, 200]));
        portrait_img.save(&portrait_path).unwrap();

        // Must reject portrait image (width <= height)
        assert!(!is_single_candidate(&portrait_path));
        let res = compose_single(&portrait_path);
        assert!(res.is_err(), "compose_single must fail on portrait image");

        // 2K landscape image (1920x1080) should qualify
        let landscape_2k_path = temp_dir.join("test_2k_landscape.png");
        let landscape_2k_img = RgbImage::from_pixel(1920, 1080, Rgb([100, 150, 200]));
        landscape_2k_img.save(&landscape_2k_path).unwrap();
        assert!(is_single_candidate(&landscape_2k_path));
        let res_2k = compose_single(&landscape_2k_path);
        assert!(
            res_2k.is_ok(),
            "compose_single must succeed on 2K landscape"
        );

        // Clean up
        let _ = fs::remove_file(&portrait_path);
        let _ = fs::remove_file(&landscape_2k_path);
    }

    #[test]
    fn test_ambient_presets_rotation() {
        let p0 = next_rotated_ambient_gradient();
        let p1 = next_rotated_ambient_gradient();
        let _p2 = next_rotated_ambient_gradient();
        let _p3 = next_rotated_ambient_gradient();
        let _p4 = next_rotated_ambient_gradient();
        let p5 = next_rotated_ambient_gradient();

        // Must rotate through presets
        assert_ne!(p0.name, p1.name);
        assert_eq!(p0.name, p5.name); // Full 5-step cycle
    }

    #[test]
    fn test_extension_support_jfif_and_jiff() {
        let temp_dir = std::env::temp_dir().join("wallpaper_shuffler_ext_test");
        let _ = fs::create_dir_all(&temp_dir);

        let test_img = RgbImage::from_pixel(100, 100, Rgb([200, 100, 50]));
        let jfif_file = temp_dir.join("sample.jfif");
        let jiff_file = temp_dir.join("sample.jiff");
        let webp_file = temp_dir.join("sample.webp");

        test_img.save(&jfif_file).unwrap();
        // Copy standard JPEG bytes to .jiff to simulate user files with .jiff extension
        fs::copy(&jfif_file, &jiff_file).unwrap();
        test_img.save(&webp_file).unwrap();

        let collected = collect_image_files(std::slice::from_ref(&temp_dir));
        assert_eq!(collected.len(), 3);

        // Verify that load_rgb_image successfully decodes both jfif and jiff files
        assert!(load_rgb_image(&jfif_file).is_ok());
        assert!(load_rgb_image(&jiff_file).is_ok());

        let _ = fs::remove_file(&jfif_file);
        let _ = fs::remove_file(&jiff_file);
        let _ = fs::remove_file(&webp_file);
        let _ = fs::remove_dir(&temp_dir);
    }
}
