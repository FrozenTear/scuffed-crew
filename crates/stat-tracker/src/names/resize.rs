//! 8-bit grayscale resize with Pillow's `Image.resize(.., BILINEAR)` arithmetic
//! (separable triangle filter widened when shrinking, 22-bit fixed point,
//! horizontal pass first). Templates in a names pack are cut with Pillow, so
//! a glyph read here has to land on the same pixel values.

const PRECISION_BITS: u32 = 32 - 8 - 2;

struct Coeffs {
    ksize: usize,
    bounds: Vec<(usize, usize)>,
    k: Vec<i32>,
}

fn coeffs(in_size: usize, out_size: usize) -> Coeffs {
    let scale = in_size as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = filterscale; // bilinear support 1.0
    let ksize = support.ceil() as usize * 2 + 1;
    let mut k = vec![0i32; out_size * ksize];
    let mut bounds = Vec::with_capacity(out_size);
    let ss = 1.0 / filterscale;
    let mut pre = vec![0f64; ksize];
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize;
        let n = xmax.saturating_sub(xmin);
        let mut ww = 0.0;
        for (x, p) in pre.iter_mut().enumerate().take(n) {
            let mut t = (x as f64 + xmin as f64 - center + 0.5) * ss;
            if t < 0.0 {
                t = -t;
            }
            let w = if t < 1.0 { 1.0 - t } else { 0.0 };
            *p = w;
            ww += w;
        }
        for x in 0..n {
            let v = if ww != 0.0 { pre[x] / ww } else { pre[x] };
            let scaled = v * f64::from(1u32 << PRECISION_BITS);
            k[xx * ksize + x] = if v < 0.0 {
                (scaled - 0.5) as i32
            } else {
                (scaled + 0.5) as i32
            };
        }
        bounds.push((xmin, n));
    }
    Coeffs { ksize, bounds, k }
}

fn clip8(v: i64) -> u8 {
    if v >= (1i64 << PRECISION_BITS) << 8 {
        255
    } else if v <= 0 {
        0
    } else {
        (v >> PRECISION_BITS) as u8
    }
}

/// Resize a row-major 8-bit image `(w, h)` to `(ow, oh)`.
pub fn resize_l(src: &[u8], w: usize, h: usize, ow: usize, oh: usize) -> Vec<u8> {
    if w == ow && h == oh {
        return src.to_vec();
    }
    let mut cur = src.to_vec();
    let mut cw = w;
    if w != ow {
        let c = coeffs(w, ow);
        let mut out = vec![0u8; ow * h];
        for y in 0..h {
            let row = &cur[y * cw..(y + 1) * cw];
            for (xx, &(xmin, n)) in c.bounds.iter().enumerate() {
                let k = &c.k[xx * c.ksize..];
                let mut ss: i64 = 1 << (PRECISION_BITS - 1);
                for x in 0..n {
                    ss += i64::from(row[xmin + x]) * i64::from(k[x]);
                }
                out[y * ow + xx] = clip8(ss);
            }
        }
        cur = out;
        cw = ow;
    }
    if h != oh {
        let c = coeffs(h, oh);
        let mut out = vec![0u8; cw * oh];
        for (yy, &(ymin, n)) in c.bounds.iter().enumerate() {
            let k = &c.k[yy * c.ksize..];
            for x in 0..cw {
                let mut ss: i64 = 1 << (PRECISION_BITS - 1);
                for y in 0..n {
                    ss += i64::from(cur[(ymin + y) * cw + x]) * i64::from(k[y]);
                }
                out[yy * cw + x] = clip8(ss);
            }
        }
        cur = out;
    }
    cur
}
