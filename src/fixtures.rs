use image::{ImageBuffer, Rgb};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FixtureSpec {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub primary_color: [u8; 3],
    pub secondary_color: [u8; 3],
    pub pattern: PatternType,
}

#[derive(Debug, Clone, Copy)]
pub enum PatternType {
    LinearGradientH,
    LinearGradientV,
    RadialGradient,
    Checkerboard,
    Stripes,
    FramedBox,
}

pub fn get_sample_specs() -> Vec<FixtureSpec> {
    vec![
        // 4K Large Landscapes (>= 3000px, 16:9 aspect)
        FixtureSpec {
            name: "01_4k_mountain_sunset.jpg",
            width: 3840,
            height: 2160,
            primary_color: [240, 90, 40],
            secondary_color: [30, 20, 60],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "02_4k_ocean_deep.jpg",
            width: 3840,
            height: 2160,
            primary_color: [10, 80, 160],
            secondary_color: [5, 20, 50],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "03_4k_neon_cyberpunk.jpg",
            width: 3840,
            height: 2160,
            primary_color: [220, 20, 140],
            secondary_color: [10, 220, 200],
            pattern: PatternType::LinearGradientH,
        },
        // QHD / FHD Landscapes (< 3000px, 16:9)
        FixtureSpec {
            name: "04_fhd_forest_green.jpg",
            width: 1920,
            height: 1080,
            primary_color: [34, 139, 34],
            secondary_color: [10, 40, 15],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "05_fhd_warm_amber.jpg",
            width: 1920,
            height: 1080,
            primary_color: [255, 165, 0],
            secondary_color: [128, 0, 0],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "06_qhd_mist_lavender.jpg",
            width: 2560,
            height: 1440,
            primary_color: [180, 160, 230],
            secondary_color: [45, 30, 80],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "07_qhd_golden_hour.jpg",
            width: 2560,
            height: 1440,
            primary_color: [255, 215, 0],
            secondary_color: [200, 80, 20],
            pattern: PatternType::Stripes,
        },
        // Ultrawide (21:9)
        FixtureSpec {
            name: "08_uw_cosmic_blue.jpg",
            width: 3440,
            height: 1440,
            primary_color: [40, 60, 180],
            secondary_color: [10, 10, 30],
            pattern: PatternType::LinearGradientH,
        },
        FixtureSpec {
            name: "09_uw_emerald_city.jpg",
            width: 2560,
            height: 1080,
            primary_color: [20, 180, 100],
            secondary_color: [5, 50, 30],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "10_uw_crimson_nebula.jpg",
            width: 3440,
            height: 1440,
            primary_color: [180, 20, 50],
            secondary_color: [30, 5, 20],
            pattern: PatternType::RadialGradient,
        },
        // Tall Portraits (9:16 or similar vertical)
        FixtureSpec {
            name: "11_portrait_sakura_pink.jpg",
            width: 1080,
            height: 1920,
            primary_color: [255, 182, 193],
            secondary_color: [180, 40, 100],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "12_portrait_azure_sky.jpg",
            width: 1080,
            height: 1920,
            primary_color: [100, 200, 255],
            secondary_color: [20, 60, 140],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "13_portrait_mint_breeze.jpg",
            width: 1440,
            height: 2560,
            primary_color: [152, 251, 152],
            secondary_color: [30, 110, 70],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "14_portrait_autumn_maple.jpg",
            width: 1200,
            height: 1920,
            primary_color: [210, 70, 20],
            secondary_color: [90, 20, 10],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "15_portrait_amethyst.jpg",
            width: 1080,
            height: 1920,
            primary_color: [150, 80, 200],
            secondary_color: [50, 10, 80],
            pattern: PatternType::Stripes,
        },
        FixtureSpec {
            name: "16_portrait_arctic_frost.jpg",
            width: 900,
            height: 1600,
            primary_color: [200, 240, 255],
            secondary_color: [70, 120, 180],
            pattern: PatternType::Checkerboard,
        },
        FixtureSpec {
            name: "17_portrait_citrus_burst.jpg",
            width: 800,
            height: 1400,
            primary_color: [255, 200, 0],
            secondary_color: [220, 80, 10],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "18_portrait_midnight_purple.jpg",
            width: 1080,
            height: 1920,
            primary_color: [90, 40, 160],
            secondary_color: [15, 5, 40],
            pattern: PatternType::FramedBox,
        },
        // Squares (1:1)
        FixtureSpec {
            name: "19_square_ruby_gem.jpg",
            width: 1200,
            height: 1200,
            primary_color: [220, 30, 60],
            secondary_color: [60, 10, 20],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "20_square_sapphire.jpg",
            width: 1000,
            height: 1000,
            primary_color: [30, 90, 210],
            secondary_color: [10, 30, 90],
            pattern: PatternType::Checkerboard,
        },
        FixtureSpec {
            name: "21_square_topaz.jpg",
            width: 1000,
            height: 1000,
            primary_color: [255, 180, 50],
            secondary_color: [140, 80, 10],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "22_square_jade.jpg",
            width: 800,
            height: 800,
            primary_color: [50, 190, 120],
            secondary_color: [10, 70, 40],
            pattern: PatternType::Stripes,
        },
        FixtureSpec {
            name: "23_square_obsidian.jpg",
            width: 800,
            height: 800,
            primary_color: [70, 75, 90],
            secondary_color: [20, 22, 28],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "24_square_pearl.jpg",
            width: 600,
            height: 600,
            primary_color: [240, 240, 245],
            secondary_color: [160, 165, 180],
            pattern: PatternType::LinearGradientH,
        },
        // Small / Medium mixed
        FixtureSpec {
            name: "25_small_card_red.jpg",
            width: 640,
            height: 480,
            primary_color: [230, 50, 50],
            secondary_color: [100, 10, 20],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "26_small_card_blue.jpg",
            width: 640,
            height: 480,
            primary_color: [50, 100, 230],
            secondary_color: [10, 30, 100],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "27_small_card_gold.jpg",
            width: 640,
            height: 480,
            primary_color: [240, 190, 40],
            secondary_color: [120, 80, 10],
            pattern: PatternType::LinearGradientH,
        },
        FixtureSpec {
            name: "28_small_portrait_leaf.jpg",
            width: 500,
            height: 750,
            primary_color: [80, 200, 100],
            secondary_color: [20, 70, 30],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "29_small_portrait_dusk.jpg",
            width: 480,
            height: 720,
            primary_color: [200, 80, 140],
            secondary_color: [40, 20, 60],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "30_small_tile_pattern.jpg",
            width: 512,
            height: 512,
            primary_color: [255, 140, 0],
            secondary_color: [60, 30, 100],
            pattern: PatternType::Checkerboard,
        },
    ]
}

pub fn generate_fixture_image(spec: &FixtureSpec) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let mut img = ImageBuffer::new(spec.width, spec.height);
    let (w, h) = (spec.width as f32, spec.height as f32);
    let cx = w / 2.0;
    let cy = h / 2.0;
    let max_r = (cx * cx + cy * cy).sqrt();

    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let (xf, yf) = (x as f32, y as f32);
        let t = match spec.pattern {
            PatternType::LinearGradientH => xf / w,
            PatternType::LinearGradientV => yf / h,
            PatternType::RadialGradient => {
                let dx = xf - cx;
                let dy = yf - cy;
                ((dx * dx + dy * dy).sqrt() / max_r).clamp(0.0, 1.0)
            }
            PatternType::Checkerboard => {
                let cell_size = (w.min(h) / 8.0).max(16.0);
                let col = (xf / cell_size) as u32;
                let row = (yf / cell_size) as u32;
                if (col + row).is_multiple_of(2) {
                    0.1
                } else {
                    0.9
                }
            }
            PatternType::Stripes => {
                let stripe_w = (w.min(h) / 12.0).max(12.0);
                let idx = ((xf + yf) / stripe_w) as u32;
                if idx.is_multiple_of(2) { 0.15 } else { 0.85 }
            }
            PatternType::FramedBox => {
                let border = (w.min(h) * 0.08).max(8.0);
                let inner_x = xf >= border && xf <= w - border;
                let inner_y = yf >= border && yf <= h - border;
                if inner_x && inner_y {
                    let dx = xf - cx;
                    let dy = yf - cy;
                    0.2 + 0.6 * ((dx * dx + dy * dy).sqrt() / max_r).clamp(0.0, 1.0)
                } else {
                    0.95
                }
            }
        };

        let r = (spec.primary_color[0] as f32 * (1.0 - t) + spec.secondary_color[0] as f32 * t)
            .round() as u8;
        let g = (spec.primary_color[1] as f32 * (1.0 - t) + spec.secondary_color[1] as f32 * t)
            .round() as u8;
        let b = (spec.primary_color[2] as f32 * (1.0 - t) + spec.secondary_color[2] as f32 * t)
            .round() as u8;

        *pixel = Rgb([r, g, b]);
    }

    img
}

pub fn generate_all_fixtures(output_dir: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(output_dir).map_err(|e| {
        format!(
            "Failed to create fixture dir {}: {}",
            output_dir.display(),
            e
        )
    })?;

    let specs = get_sample_specs();
    let mut generated_paths = Vec::with_capacity(specs.len());

    for spec in &specs {
        let path = output_dir.join(spec.name);
        let img = generate_fixture_image(spec);
        img.save(&path)
            .map_err(|e| format!("Failed to save fixture image {}: {}", path.display(), e))?;
        generated_paths.push(path);
    }

    Ok(generated_paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_specs_count() {
        let specs = get_sample_specs();
        assert_eq!(specs.len(), 30);
    }

    #[test]
    fn test_generate_single_fixture() {
        let specs = get_sample_specs();
        let img = generate_fixture_image(&specs[0]);
        assert_eq!(img.width(), 3840);
        assert_eq!(img.height(), 2160);
    }
}
