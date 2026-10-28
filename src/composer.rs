use image::RgbImage;
use rand::seq::SliceRandom;
use std::path::{Path, PathBuf};

pub const CANVAS_WIDTH_4K: u32 = 3840;
pub const CANVAS_HEIGHT_4K: u32 = 2160;

#[derive(Debug, Clone)]
pub enum WallpaperCandidate {
    Single(PathBuf),
    Collage(Vec<PathBuf>),
}

pub fn evaluate_image_for_wallpaper(path: &Path) -> Result<WallpaperCandidate, String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| format!("Failed to open image {}: {}", path.display(), e))?
        .with_guessed_format()
        .map_err(|e| format!("Format error: {}", e))?;

    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| format!("Dimension error: {}", e))?;

    let aspect = width as f64 / height as f64;
    let target_aspect = 16.0 / 9.0;
    let aspect_diff = (aspect - target_aspect).abs();

    if width >= 3000 && aspect_diff < 0.35 {
        Ok(WallpaperCandidate::Single(path.to_path_buf()))
    } else {
        Ok(WallpaperCandidate::Collage(vec![path.to_path_buf()]))
    }
}

pub fn create_empty_4k_canvas() -> RgbImage {
    RgbImage::new(CANVAS_WIDTH_4K, CANVAS_HEIGHT_4K)
}

pub fn select_random_images(images: &[PathBuf], count: usize) -> Vec<&PathBuf> {
    let mut rng = rand::thread_rng();
    images.choose_multiple(&mut rng, count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canvas_dimensions() {
        let img = create_empty_4k_canvas();
        assert_eq!(img.width(), 3840);
        assert_eq!(img.height(), 2160);
    }
}
