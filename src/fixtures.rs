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
        // ========================================================
        // 1. 3K+ Large Landscapes (for direct 4K background, 6 items)
        // ========================================================
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
        FixtureSpec {
            name: "04_4k_alpine_forest.jpg",
            width: 3840,
            height: 2160,
            primary_color: [34, 139, 34],
            secondary_color: [12, 45, 20],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "05_4k_crimson_nebula.jpg",
            width: 3840,
            height: 2160,
            primary_color: [200, 30, 70],
            secondary_color: [35, 8, 30],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "06_uw_cosmic_horizon.jpg",
            width: 3440,
            height: 1440,
            primary_color: [45, 70, 200],
            secondary_color: [15, 15, 35],
            pattern: PatternType::LinearGradientH,
        },
        // ========================================================
        // 2. Sub-4K Portrait Images (Majority! 18 items, long edge ~2K)
        // ========================================================
        FixtureSpec {
            name: "07_portrait_sakura_tower.jpg",
            width: 1080,
            height: 1920,
            primary_color: [255, 180, 195],
            secondary_color: [165, 40, 95],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "08_portrait_azure_skylight.jpg",
            width: 1080,
            height: 1920,
            primary_color: [95, 195, 255],
            secondary_color: [25, 65, 145],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "09_portrait_mint_flora.jpg",
            width: 1440,
            height: 2560,
            primary_color: [145, 245, 175],
            secondary_color: [28, 105, 68],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "10_portrait_amethyst_temple.jpg",
            width: 1080,
            height: 1920,
            primary_color: [160, 85, 215],
            secondary_color: [45, 12, 75],
            pattern: PatternType::Stripes,
        },
        FixtureSpec {
            name: "11_portrait_arctic_glacier.jpg",
            width: 1200,
            height: 1920,
            primary_color: [195, 235, 255],
            secondary_color: [60, 110, 175],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "12_portrait_sunset_boulevard.jpg",
            width: 1200,
            height: 1920,
            primary_color: [250, 130, 45],
            secondary_color: [90, 20, 60],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "13_portrait_citrus_glow.jpg",
            width: 1440,
            height: 2560,
            primary_color: [255, 210, 30],
            secondary_color: [210, 75, 15],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "14_portrait_midnight_cathedral.jpg",
            width: 1536,
            height: 2048,
            primary_color: [85, 45, 160],
            secondary_color: [18, 8, 42],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "15_portrait_emerald_bamboo.jpg",
            width: 1080,
            height: 1920,
            primary_color: [40, 180, 100],
            secondary_color: [10, 60, 30],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "16_portrait_golden_lantern.jpg",
            width: 1080,
            height: 1920,
            primary_color: [255, 195, 50],
            secondary_color: [140, 70, 10],
            pattern: PatternType::Stripes,
        },
        FixtureSpec {
            name: "17_portrait_lavender_dusk.jpg",
            width: 1200,
            height: 1920,
            primary_color: [175, 150, 230],
            secondary_color: [50, 30, 80],
            pattern: PatternType::LinearGradientH,
        },
        FixtureSpec {
            name: "18_portrait_autumn_gate.jpg",
            width: 1200,
            height: 1920,
            primary_color: [220, 85, 30],
            secondary_color: [95, 25, 10],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "19_portrait_crystal_falls.jpg",
            width: 1440,
            height: 2560,
            primary_color: [80, 210, 240],
            secondary_color: [20, 70, 130],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "20_portrait_neon_rain.jpg",
            width: 1440,
            height: 2560,
            primary_color: [230, 30, 130],
            secondary_color: [20, 20, 70],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "21_portrait_monochrome_alley.jpg",
            width: 1080,
            height: 1920,
            primary_color: [220, 220, 225],
            secondary_color: [40, 42, 50],
            pattern: PatternType::Checkerboard,
        },
        FixtureSpec {
            name: "22_portrait_coral_garden.jpg",
            width: 1536,
            height: 2048,
            primary_color: [250, 115, 90],
            secondary_color: [80, 25, 40],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "23_portrait_celestial_moon.jpg",
            width: 1536,
            height: 2048,
            primary_color: [240, 240, 180],
            secondary_color: [15, 25, 60],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "24_portrait_scarlet_bridge.jpg",
            width: 1536,
            height: 2048,
            primary_color: [215, 35, 45],
            secondary_color: [60, 10, 15],
            pattern: PatternType::FramedBox,
        },
        // ========================================================
        // 3. Sub-4K Landscape Images (4 items)
        // ========================================================
        FixtureSpec {
            name: "25_fhd_sapphire_lake.jpg",
            width: 1920,
            height: 1080,
            primary_color: [30, 110, 220],
            secondary_color: [10, 35, 90],
            pattern: PatternType::LinearGradientV,
        },
        FixtureSpec {
            name: "26_fhd_warm_amber.jpg",
            width: 1920,
            height: 1080,
            primary_color: [255, 170, 20],
            secondary_color: [130, 40, 10],
            pattern: PatternType::FramedBox,
        },
        FixtureSpec {
            name: "27_fhd_cherry_blossom.jpg",
            width: 1920,
            height: 1080,
            primary_color: [255, 185, 200],
            secondary_color: [170, 45, 95],
            pattern: PatternType::LinearGradientH,
        },
        FixtureSpec {
            name: "28_4x3_vintage_cafe.jpg",
            width: 2048,
            height: 1536,
            primary_color: [190, 140, 95],
            secondary_color: [75, 45, 25],
            pattern: PatternType::FramedBox,
        },
        // ========================================================
        // 4. Sub-4K Square Images (2 items)
        // ========================================================
        FixtureSpec {
            name: "29_square_ruby_mandala.jpg",
            width: 1800,
            height: 1800,
            primary_color: [225, 35, 65],
            secondary_color: [65, 12, 25],
            pattern: PatternType::RadialGradient,
        },
        FixtureSpec {
            name: "30_square_sapphire_mosaic.jpg",
            width: 1800,
            height: 1800,
            primary_color: [35, 100, 225],
            secondary_color: [12, 35, 95],
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
    fn test_portrait_majority() {
        let specs = get_sample_specs();
        let portraits = specs.iter().filter(|s| s.height > s.width).count();
        let sub4k = specs
            .iter()
            .filter(|s| {
                !(s.width >= 3000 && (s.width as f64 / s.height as f64 - 1.77).abs() < 0.35)
            })
            .count();
        assert!(
            portraits > sub4k / 2,
            "Portraits ({}) should be the majority among sub-4k images ({})",
            portraits,
            sub4k
        );
    }
}
