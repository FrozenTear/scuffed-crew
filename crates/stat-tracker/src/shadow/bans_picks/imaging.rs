//! Pixel steps for the cv-v1 bans and picks reader: grey conversion, area
//! resize, Gaussian blur, Sobel gradient magnitude and an 8-bit HSV
//! conversion. They follow OpenCV's 8-bit conventions (the grey weights, the
//! area resize, reflect-101 borders, H in 0..180) because the thresholds in
//! the parent module were measured with the OpenCV version of the same steps.
//! Pixels only: no templates, no readers.

use image::RgbImage;

/// 8-bit grey image, row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// Float image (a gradient magnitude), row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

/// BGR to grey with OpenCV's fixed-point weights (0.299 R, 0.587 G, 0.114 B).
pub fn gray_from_rgb(img: &RgbImage) -> Gray {
    let px = img
        .pixels()
        .map(|p| {
            let [r, g, b] = p.0;
            ((b as u32 * 1868 + g as u32 * 9617 + r as u32 * 4899 + 8192) >> 14) as u8
        })
        .collect();
    Gray {
        w: img.width() as usize,
        h: img.height() as usize,
        px,
    }
}

/// Area-average shrink by exactly one half, rounding half up, the way
/// OpenCV's `INTER_AREA` does for a 2x2 cell. Odd trailing rows or columns
/// are dropped.
pub fn halve(src: &Gray) -> Gray {
    let (w, h) = (src.w / 2, src.h / 2);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let r0 = &src.px[2 * y * src.w..2 * y * src.w + src.w];
        let r1 = &src.px[(2 * y + 1) * src.w..(2 * y + 1) * src.w + src.w];
        for x in 0..w {
            let s =
                r0[2 * x] as u32 + r0[2 * x + 1] as u32 + r1[2 * x] as u32 + r1[2 * x + 1] as u32;
            px.push(((s + 2) >> 2) as u8);
        }
    }
    Gray { w, h, px }
}

/// Weights for one output sample of an area resize: first source index and
/// the normalised overlaps.
fn area_weights(dst: usize, src: usize, scale: f64) -> Vec<(usize, Vec<f32>)> {
    (0..dst)
        .map(|d| {
            let a = d as f64 * scale;
            let b = (a + scale).min(src as f64);
            let first = a.floor() as usize;
            let last = (b.ceil() as usize).min(src).max(first + 1);
            let mut w: Vec<f64> = (first..last)
                .map(|i| ((i + 1) as f64).min(b) - (i as f64).max(a))
                .map(|v| v.max(0.0))
                .collect();
            let total: f64 = w.iter().sum();
            if total <= 0.0 {
                w = vec![1.0];
            }
            let total: f64 = w.iter().sum();
            (
                first.min(src - 1),
                w.iter().map(|v| (v / total) as f32).collect(),
            )
        })
        .collect()
}

/// Area-average resize by a factor (`INTER_AREA`): each output size is
/// `round_half_even(size * factor)`, each output pixel the overlap-weighted
/// mean of the source pixels it covers.
pub fn resize_area(src: &Gray, factor: f64) -> Gray {
    let dw = ((src.w as f64 * factor).round_ties_even() as usize).max(1);
    let dh = ((src.h as f64 * factor).round_ties_even() as usize).max(1);
    let wx = area_weights(dw, src.w, 1.0 / factor);
    let wy = area_weights(dh, src.h, 1.0 / factor);
    let mut tmp = vec![0f32; dw * src.h];
    for y in 0..src.h {
        let row = &src.px[y * src.w..(y + 1) * src.w];
        for (x, (first, w)) in wx.iter().enumerate() {
            tmp[y * dw + x] = w
                .iter()
                .enumerate()
                .map(|(k, wt)| row[(first + k).min(src.w - 1)] as f32 * wt)
                .sum();
        }
    }
    let mut px = vec![0u8; dw * dh];
    for (y, (first, w)) in wy.iter().enumerate() {
        for x in 0..dw {
            let v: f32 = w
                .iter()
                .enumerate()
                .map(|(k, wt)| tmp[(first + k).min(src.h - 1) * dw + x] * wt)
                .sum();
            px[y * dw + x] = v.round_ties_even().clamp(0.0, 255.0) as u8;
        }
    }
    Gray { w: dw, h: dh, px }
}

/// Reflect-101 border index: `-1 -> 1`, `n -> n - 2`.
#[inline]
fn reflect(i: isize, n: usize) -> usize {
    if n == 1 {
        return 0;
    }
    let n = n as isize;
    let mut i = i;
    if i < 0 {
        i = -i;
    }
    if i >= n {
        i = 2 * (n - 1) - i;
    }
    i.clamp(0, n - 1) as usize
}

/// 7-tap Gaussian, sigma 1, as `getGaussianKernel(7, 1.0)`.
fn gauss7() -> [f32; 7] {
    let mut k = [0f32; 7];
    let mut sum = 0f32;
    for (i, v) in k.iter_mut().enumerate() {
        let x = i as f32 - 3.0;
        *v = (-x * x / 2.0).exp();
        sum += *v;
    }
    for v in &mut k {
        *v /= sum;
    }
    k
}

/// Gaussian blur (7 taps, sigma 1, reflect-101), result rounded to 8 bits.
pub fn blur(src: &Gray) -> Gray {
    let k = gauss7();
    let (w, h) = (src.w, src.h);
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        let row = &src.px[y * w..(y + 1) * w];
        for x in 0..w {
            let mut s = 0f32;
            for (t, kv) in k.iter().enumerate() {
                s += row[reflect(x as isize + t as isize - 3, w)] as f32 * kv;
            }
            tmp[y * w + x] = s;
        }
    }
    let mut px = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0f32;
            for (t, kv) in k.iter().enumerate() {
                s += tmp[reflect(y as isize + t as isize - 3, h) * w + x] * kv;
            }
            px[y * w + x] = s.round_ties_even().clamp(0.0, 255.0) as u8;
        }
    }
    Gray { w, h, px }
}

/// Sobel gradient magnitude (3x3, reflect-101) of an 8-bit image.
pub fn sobel_magnitude(src: &Gray) -> Plane {
    let (w, h) = (src.w, src.h);
    let at = |x: isize, y: isize| src.px[reflect(y, h) * w + reflect(x, w)] as f32;
    let mut px = vec![0f32; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let dx = (at(x + 1, y - 1) - at(x - 1, y - 1))
                + 2.0 * (at(x + 1, y) - at(x - 1, y))
                + (at(x + 1, y + 1) - at(x - 1, y + 1));
            let dy = (at(x - 1, y + 1) - at(x - 1, y - 1))
                + 2.0 * (at(x, y + 1) - at(x, y - 1))
                + (at(x + 1, y + 1) - at(x + 1, y - 1));
            px[y as usize * w + x as usize] = (dx * dx + dy * dy).sqrt();
        }
    }
    Plane { w, h, px }
}

/// Blur, then Sobel magnitude: the plane every match runs on.
pub fn gradient(src: &Gray) -> Plane {
    sobel_magnitude(&blur(src))
}

/// OpenCV 8-bit HSV of one RGB pixel: H 0..180, S and V 0..255.
pub fn hsv(rgb: [u8; 3]) -> [u8; 3] {
    let [r, g, b] = rgb.map(|v| v as i32);
    let v = r.max(g).max(b);
    let diff = v - r.min(g).min(b);
    let s = if v == 0 {
        0
    } else {
        ((255.0 * diff as f64 / v as f64).round_ties_even()) as i32
    };
    let mut hdeg = if diff == 0 {
        0.0
    } else if v == r {
        60.0 * (g - b) as f64 / diff as f64
    } else if v == g {
        120.0 + 60.0 * (b - r) as f64 / diff as f64
    } else {
        240.0 + 60.0 * (r - g) as f64 / diff as f64
    };
    if hdeg < 0.0 {
        hdeg += 360.0;
    }
    let h = ((hdeg / 2.0).round_ties_even() as i32) % 180;
    [h as u8, s as u8, v as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, v: u8) -> Gray {
        Gray {
            w,
            h,
            px: vec![v; w * h],
        }
    }

    #[test]
    fn halve_rounds_half_up_and_keeps_flat_planes() {
        assert_eq!(halve(&flat(4, 4, 77)).px, vec![77; 4]);
        let g = Gray {
            w: 2,
            h: 2,
            px: vec![0, 0, 0, 2],
        };
        assert_eq!(halve(&g).px, vec![1]);
        let g = Gray {
            w: 2,
            h: 2,
            px: vec![0, 0, 0, 1],
        };
        assert_eq!(halve(&g).px, vec![0]);
        let g = Gray {
            w: 2,
            h: 2,
            px: vec![1, 1, 1, 0],
        };
        assert_eq!(halve(&g).px, vec![1]);
    }

    #[test]
    fn resize_area_sizes_and_flat_planes() {
        let out = resize_area(&flat(100, 96, 90), 0.935);
        assert_eq!((out.w, out.h), (94, 90));
        assert!(out.px.iter().all(|&v| v == 90));
        let out = resize_area(&flat(100, 96, 200), 0.4);
        assert_eq!((out.w, out.h), (40, 38));
        assert!(out.px.iter().all(|&v| v == 200));
    }

    #[test]
    fn resize_area_halving_matches_the_exact_halve() {
        let mut g = flat(8, 8, 0);
        for (i, v) in g.px.iter_mut().enumerate() {
            *v = (i * 7 % 251) as u8;
        }
        let a = resize_area(&g, 0.5);
        let b = halve(&g);
        for (x, y) in a.px.iter().zip(&b.px) {
            assert!((*x as i32 - *y as i32).abs() <= 1);
        }
    }

    #[test]
    fn blur_keeps_flat_and_smooths_a_spike() {
        assert!(blur(&flat(9, 9, 120)).px.iter().all(|&v| v == 120));
        let mut g = flat(9, 9, 0);
        g.px[4 * 9 + 4] = 255;
        let b = blur(&g);
        assert!(b.px[4 * 9 + 4] < 255 && b.px[4 * 9 + 4] > b.px[4 * 9 + 3]);
    }

    #[test]
    fn sobel_is_zero_on_flat_and_finds_an_edge() {
        assert!(
            sobel_magnitude(&flat(6, 6, 50))
                .px
                .iter()
                .all(|&v| v == 0.0)
        );
        let mut g = flat(6, 6, 0);
        for y in 0..6 {
            for x in 3..6 {
                g.px[y * 6 + x] = 100;
            }
        }
        let s = sobel_magnitude(&g);
        assert_eq!(s.px[2 * 6 + 2], 400.0);
        assert_eq!(s.px[2 * 6], 0.0);
    }

    #[test]
    fn hsv_matches_known_opencv_values() {
        assert_eq!(hsv([255, 0, 0]), [0, 255, 255]);
        assert_eq!(hsv([0, 255, 0]), [60, 255, 255]);
        assert_eq!(hsv([0, 0, 255]), [120, 255, 255]);
        assert_eq!(hsv([128, 128, 128]), [0, 0, 128]);
        assert_eq!(hsv([0, 0, 0]), [0, 0, 0]);
    }

    #[test]
    fn gray_uses_the_opencv_weights() {
        let mut img = RgbImage::new(1, 3);
        img.put_pixel(0, 0, image::Rgb([255, 255, 255]));
        img.put_pixel(0, 1, image::Rgb([255, 0, 0]));
        img.put_pixel(0, 2, image::Rgb([0, 0, 255]));
        assert_eq!(gray_from_rgb(&img).px, vec![255, 76, 29]);
    }
}
