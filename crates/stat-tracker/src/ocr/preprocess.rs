use image::{DynamicImage, GrayImage, Luma};

pub fn prepare(img: &DynamicImage) -> GrayImage {
    prepare_with_threshold(img, 140)
}

pub fn prepare_with_threshold(img: &DynamicImage, threshold: u8) -> GrayImage {
    let gray = img.to_luma8();
    let (w, h) = gray.dimensions();

    // Only upscale if the image is small (< 1280px wide) — high-res displays don't need it
    let work_img = if w < 1280 {
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
        upscaled
    } else {
        gray
    };

    // 3x3 median filter to remove noise while preserving text edges
    let filtered = median_filter_3x3(&work_img);

    // Invert so bright text becomes black (for Tesseract) and everything else becomes white.
    let mut binary = filtered;
    for px in binary.pixels_mut() {
        px.0[0] = if px.0[0] > threshold { 0 } else { 255 };
    }

    binary
}

fn median_filter_3x3(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = GrayImage::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let mut window = [0u8; 9];
            let mut idx = 0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as u32;
                    let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as u32;
                    window[idx] = img.get_pixel(sx, sy).0[0];
                    idx += 1;
                }
            }
            window.sort_unstable();
            out.put_pixel(x, y, Luma([window[4]]));
        }
    }

    out
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
