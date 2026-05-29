use anyhow::Context;
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use rayon::prelude::*;
use std::path::Path;

/// Calculate the number of tiles at a given zoom level.
pub fn tiles_at_zoom(
    full_width: u32,
    full_height: u32,
    tile_size: u32,
    zoom: u32,
    max_zoom: u32,
) -> (u32, u32) {
    let scale = 1u32 << (max_zoom - zoom.min(max_zoom));
    let scaled_w = full_width / scale;
    let scaled_h = full_height / scale;
    let cols = scaled_w.div_ceil(tile_size);
    let rows = scaled_h.div_ceil(tile_size);
    (cols.max(1), rows.max(1))
}

/// Calculate the maximum zoom level for a given image size and tile size.
/// At max_zoom, the image is served at full resolution.
/// Each lower zoom level halves the resolution.
pub fn calculate_max_zoom(full_width: u32, full_height: u32, tile_size: u32) -> u32 {
    let max_dim = full_width.max(full_height);
    if max_dim <= tile_size {
        return 0;
    }
    // How many times can we halve before fitting in one tile?
    (max_dim as f64 / tile_size as f64).log2().ceil() as u32
}

/// Generate a full tile pyramid from a floor image.
///
/// Creates: `{output_dir}/floors/{floor_id}/{zoom}/{x}/{y}.webp`
///
/// Returns (full_width, full_height) of the source image.
pub fn generate_tile_pyramid(
    image_path: &Path,
    output_dir: &Path,
    floor_id: &str,
    tile_size: u32,
    max_zoom: Option<u32>,
) -> anyhow::Result<(u32, u32)> {
    let img = image::open(image_path)
        .with_context(|| format!("Failed to open floor image: {:?}", image_path))?;

    let (full_width, full_height) = img.dimensions();
    let max_zoom =
        max_zoom.unwrap_or_else(|| calculate_max_zoom(full_width, full_height, tile_size));

    tracing::info!(
        "Generating tiles for floor '{}': {}x{}, max_zoom={}",
        floor_id,
        full_width,
        full_height,
        max_zoom
    );

    for zoom in 0..=max_zoom {
        generate_zoom_level(&img, output_dir, floor_id, tile_size, zoom, max_zoom)?;
    }

    Ok((full_width, full_height))
}

fn generate_zoom_level(
    img: &DynamicImage,
    output_dir: &Path,
    floor_id: &str,
    tile_size: u32,
    zoom: u32,
    max_zoom: u32,
) -> anyhow::Result<()> {
    let (full_w, full_h) = img.dimensions();
    let scale = 1u32 << (max_zoom - zoom);

    // Resize image for this zoom level
    let scaled_w = (full_w / scale).max(1);
    let scaled_h = (full_h / scale).max(1);

    let scaled_img = if zoom == max_zoom {
        img.clone()
    } else {
        img.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Lanczos3)
    };

    let (cols, rows) = tiles_at_zoom(full_w, full_h, tile_size, zoom, max_zoom);

    tracing::info!(
        "  Zoom {}: {}x{} -> {}x{} tiles",
        zoom,
        scaled_w,
        scaled_h,
        cols,
        rows
    );

    // Generate tiles in parallel
    let tile_coords: Vec<(u32, u32)> = (0..rows)
        .flat_map(|y| (0..cols).map(move |x| (x, y)))
        .collect();

    tile_coords
        .par_iter()
        .try_for_each(|&(x, y)| -> anyhow::Result<()> {
            let tile_dir = output_dir.join(format!("floors/{}/{}", floor_id, zoom));
            std::fs::create_dir_all(tile_dir.join(format!("{}", x)))?;

            let src_x = x * tile_size;
            let src_y = y * tile_size;
            let crop_w = tile_size.min(scaled_w.saturating_sub(src_x));
            let crop_h = tile_size.min(scaled_h.saturating_sub(src_y));

            if crop_w == 0 || crop_h == 0 {
                return Ok(());
            }

            // Crop the tile from the scaled image
            let tile_img = scaled_img.crop_imm(src_x, src_y, crop_w, crop_h);

            // If tile is smaller than tile_size, pad with transparent pixels
            let final_tile = if crop_w < tile_size || crop_h < tile_size {
                let mut padded = RgbaImage::new(tile_size, tile_size);
                image::imageops::overlay(&mut padded, &tile_img.to_rgba8(), 0, 0);
                DynamicImage::ImageRgba8(padded)
            } else {
                tile_img
            };

            let tile_path = tile_dir.join(format!("{}/{}.webp", x, y));
            final_tile.save_with_format(&tile_path, ImageFormat::WebP)?;

            Ok(())
        })?;

    Ok(())
}

/// Generate a thumbnail from a floor image.
pub fn generate_thumbnail(
    image_path: &Path,
    output_path: &Path,
    max_dimension: u32,
) -> anyhow::Result<()> {
    let img = image::open(image_path)?;
    let thumbnail = img.thumbnail(max_dimension, max_dimension);
    thumbnail.save_with_format(output_path, ImageFormat::WebP)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_at_max_zoom() {
        // 1024x512 image with 256 tiles at max zoom (zoom=2)
        let (cols, rows) = tiles_at_zoom(1024, 512, 256, 2, 2);
        assert_eq!(cols, 4); // 1024/256
        assert_eq!(rows, 2); // 512/256
    }

    #[test]
    fn tiles_at_zoom_zero() {
        // At zoom 0 with max_zoom=2, scale is 4x
        // 1024/4 = 256 -> 1 tile wide
        // 512/4 = 128 -> 1 tile tall
        let (cols, rows) = tiles_at_zoom(1024, 512, 256, 0, 2);
        assert_eq!(cols, 1);
        assert_eq!(rows, 1);
    }

    #[test]
    fn tiles_at_zoom_noneven() {
        // 1000x500 at max zoom should round up
        let (cols, rows) = tiles_at_zoom(1000, 500, 256, 3, 3);
        assert_eq!(cols, 4); // ceil(1000/256)
        assert_eq!(rows, 2); // ceil(500/256)
    }

    #[test]
    fn calculate_max_zoom_small_image() {
        // 256x256 fits in one tile
        assert_eq!(calculate_max_zoom(256, 256, 256), 0);
    }

    #[test]
    fn calculate_max_zoom_large_image() {
        // 2048x2048 needs 3 zoom levels (2048/256 = 8 = 2^3)
        assert_eq!(calculate_max_zoom(2048, 2048, 256), 3);
    }

    #[test]
    fn calculate_max_zoom_nonsquare() {
        // 4096x1024 → max dim is 4096 → log2(4096/256) = 4
        assert_eq!(calculate_max_zoom(4096, 1024, 256), 4);
    }

    #[test]
    fn generate_tiles_from_test_image() {
        // Create a small test image (512x512 red square)
        let img = RgbaImage::from_fn(512, 512, |_, _| image::Rgba([255, 0, 0, 255]));
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test_floor.png");
        img.save(&img_path).unwrap();

        let output_dir = tmp.path().join("output");
        let (w, h) = generate_tile_pyramid(&img_path, &output_dir, "ground", 256, Some(1)).unwrap();

        assert_eq!(w, 512);
        assert_eq!(h, 512);

        // At zoom 1 (max): 2x2 tiles
        assert!(output_dir.join("floors/ground/1/0/0.webp").exists());
        assert!(output_dir.join("floors/ground/1/0/1.webp").exists());
        assert!(output_dir.join("floors/ground/1/1/0.webp").exists());
        assert!(output_dir.join("floors/ground/1/1/1.webp").exists());

        // At zoom 0: 1x1 tile
        assert!(output_dir.join("floors/ground/0/0/0.webp").exists());
    }
}
