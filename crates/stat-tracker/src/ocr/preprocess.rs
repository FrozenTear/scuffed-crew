use image::{DynamicImage, GrayImage};

pub fn prepare(img: &DynamicImage) -> GrayImage {
    let gray = img.to_luma8();
    let (w, h) = gray.dimensions();

    // Upscale 2x with nearest-neighbor (preserves sharp text edges for OCR)
    let mut upscaled = GrayImage::new(w * 2, h * 2);
    for y in 0..h {
        for x in 0..w {
            let px = *gray.get_pixel(x, y);
            upscaled.put_pixel(x * 2, y * 2, px);
            upscaled.put_pixel(x * 2 + 1, y * 2, px);
            upscaled.put_pixel(x * 2, y * 2 + 1, px);
            upscaled.put_pixel(x * 2 + 1, y * 2 + 1, px);
        }
    }

    // Otsu threshold for binarization
    let threshold = otsu_threshold(&upscaled);
    for px in upscaled.pixels_mut() {
        px.0[0] = if px.0[0] > threshold { 255 } else { 0 };
    }

    upscaled
}

fn otsu_threshold(img: &GrayImage) -> u8 {
    let mut histogram = [0u32; 256];
    for px in img.pixels() {
        histogram[px.0[0] as usize] += 1;
    }

    let total = img.pixels().count() as f64;
    let mut sum_total = 0.0;
    for (i, &count) in histogram.iter().enumerate() {
        sum_total += i as f64 * count as f64;
    }

    let mut sum_bg = 0.0;
    let mut weight_bg = 0.0;
    let mut max_variance = 0.0;
    let mut best_threshold = 0u8;

    for (i, &count) in histogram.iter().enumerate() {
        weight_bg += count as f64;
        if weight_bg == 0.0 {
            continue;
        }
        let weight_fg = total - weight_bg;
        if weight_fg == 0.0 {
            break;
        }

        sum_bg += i as f64 * count as f64;
        let mean_bg = sum_bg / weight_bg;
        let mean_fg = (sum_total - sum_bg) / weight_fg;
        let variance = weight_bg * weight_fg * (mean_bg - mean_fg).powi(2);

        if variance > max_variance {
            max_variance = variance;
            best_threshold = i as u8;
        }
    }

    best_threshold
}

pub fn crop_scoreboard(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    // OW2 Tab scoreboard: centered panel ~65% width, ~70% height
    // Tuned from actual OW2 screenshot samples
    let x = w * 175 / 1000;
    let y = h * 15 / 100;
    let crop_w = w * 65 / 100;
    let crop_h = h * 70 / 100;
    img.crop_imm(x, y, crop_w, crop_h)
}
