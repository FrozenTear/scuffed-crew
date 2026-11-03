//! Masked normalised cross-correlation on gradient planes, the matcher the
//! cv-v1 bans and picks reader runs. One template per hero icon, one search
//! window per screen position, the best score per template.
//!
//! The score is OpenCV's `TM_CCOEFF_NORMED` with a mask: the template is
//! zero-meaned over its mask, the window statistics are taken over the same
//! mask at each offset, and the dot product is divided by the two standard
//! deviations. The mask drops the heart badge that sits in the top-right
//! corner of an icon, because it is on some icons and not on others. Mask
//! sums come from integral images, so an offset costs one dot product.
//!
//! The search is coarse to fine: every template is scored on a strided grid
//! of offsets, the best few are refined at full resolution around their
//! coarse peak, and the rest keep their coarse score. [`SearchPlan::EXACT`]
//! scores every offset of every template.

use super::imaging::Plane;

/// Share of the template height, from the top, the corner mask covers.
const CORNER_ROWS: f64 = 0.32;
/// Share of the template width, from the right, the corner mask covers
/// (the mask starts at 70 percent of the width).
const CORNER_COL: f64 = 0.70;

/// A template zero-meaned over its mask.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub w: usize,
    pub h: usize,
    /// Zero-meaned gradient, zero inside the masked corner.
    tp: Vec<f32>,
    /// Sum of squares of `tp`.
    sum_sq: f64,
    /// Pixels inside the mask.
    n: f64,
    /// Rows `0..cut_rows` have their columns from `cut_col` masked out.
    cut_rows: usize,
    cut_col: usize,
}

impl Prepared {
    pub fn new(grad: &Plane) -> Self {
        let (w, h) = (grad.w, grad.h);
        let cut_rows = (h as f64 * CORNER_ROWS) as usize;
        let cut_col = (w as f64 * CORNER_COL) as usize;
        let masked = |x: usize, y: usize| y < cut_rows && x >= cut_col;
        let mut sum = 0f64;
        let mut n = 0f64;
        for y in 0..h {
            for x in 0..w {
                if !masked(x, y) {
                    sum += grad.px[y * w + x] as f64;
                    n += 1.0;
                }
            }
        }
        let mean = if n > 0.0 { sum / n } else { 0.0 };
        let mut tp = vec![0f32; w * h];
        let mut sum_sq = 0f64;
        for y in 0..h {
            for x in 0..w {
                if !masked(x, y) {
                    let v = grad.px[y * w + x] as f64 - mean;
                    tp[y * w + x] = v as f32;
                    sum_sq += v * v;
                }
            }
        }
        Self {
            w,
            h,
            tp,
            sum_sq,
            n,
            cut_rows,
            cut_col,
        }
    }

    /// Columns of template row `y` that carry weight.
    #[inline]
    fn row_len(&self, y: usize) -> usize {
        if y < self.cut_rows {
            self.cut_col
        } else {
            self.w
        }
    }
}

/// A search window cut from a gradient plane, with integral images of its
/// values and squares.
#[derive(Debug, Clone)]
pub struct Window {
    pub w: usize,
    pub h: usize,
    px: Vec<f32>,
    int_sum: Vec<f64>,
    int_sq: Vec<f64>,
}

impl Window {
    /// The rectangle `x0..x1`, `y0..y1` of `plane`, clipped to it.
    pub fn cut(plane: &Plane, x0: usize, y0: usize, x1: usize, y1: usize) -> Self {
        let x1 = x1.min(plane.w);
        let y1 = y1.min(plane.h);
        let (x0, y0) = (x0.min(x1), y0.min(y1));
        let (w, h) = (x1 - x0, y1 - y0);
        let mut px = Vec::with_capacity(w * h);
        for y in y0..y1 {
            px.extend_from_slice(&plane.px[y * plane.w + x0..y * plane.w + x1]);
        }
        let iw = w + 1;
        let mut int_sum = vec![0f64; iw * (h + 1)];
        let mut int_sq = vec![0f64; iw * (h + 1)];
        for y in 0..h {
            let (mut rs, mut rq) = (0f64, 0f64);
            for x in 0..w {
                let v = px[y * w + x] as f64;
                rs += v;
                rq += v * v;
                int_sum[(y + 1) * iw + x + 1] = int_sum[y * iw + x + 1] + rs;
                int_sq[(y + 1) * iw + x + 1] = int_sq[y * iw + x + 1] + rq;
            }
        }
        Self {
            w,
            h,
            px,
            int_sum,
            int_sq,
        }
    }

    fn rect(int: &[f64], iw: usize, x: usize, y: usize, w: usize, h: usize) -> f64 {
        int[(y + h) * iw + x + w] - int[y * iw + x + w] - int[(y + h) * iw + x] + int[y * iw + x]
    }

    /// The masked correlation score of `t` with its top-left corner at `(x, y)`.
    fn score(&self, t: &Prepared, x: usize, y: usize) -> f32 {
        let iw = self.w + 1;
        let (mut s, mut q) = (
            Self::rect(&self.int_sum, iw, x, y, t.w, t.h),
            Self::rect(&self.int_sq, iw, x, y, t.w, t.h),
        );
        let cut_w = t.w - t.cut_col;
        if t.cut_rows > 0 && cut_w > 0 {
            s -= Self::rect(&self.int_sum, iw, x + t.cut_col, y, cut_w, t.cut_rows);
            q -= Self::rect(&self.int_sq, iw, x + t.cut_col, y, cut_w, t.cut_rows);
        }
        let var = q - s * s / t.n;
        let denom = (t.sum_sq * var.max(0.0)).sqrt();
        if denom < 1e-6 {
            return 0.0;
        }
        let mut dot = 0f64;
        for ty in 0..t.h {
            let len = t.row_len(ty);
            let a = &t.tp[ty * t.w..ty * t.w + len];
            let b = &self.px[(y + ty) * self.w + x..(y + ty) * self.w + x + len];
            dot += dot8(a, b) as f64;
        }
        (dot / denom) as f32
    }

    /// Best score over all offsets in `xs` by `ys` steps, with its offset.
    fn best_on_grid(&self, t: &Prepared, step: usize) -> (f32, usize, usize) {
        let mut best = (f32::MIN, 0, 0);
        let (mx, my) = (self.w - t.w, self.h - t.h);
        let mut y = 0;
        while y <= my {
            let mut x = 0;
            while x <= mx {
                let s = self.score(t, x, y);
                if s > best.0 {
                    best = (s, x, y);
                }
                x += step;
            }
            y += step;
        }
        best
    }

    /// Best score within `radius` of `(cx, cy)`, every offset.
    fn refine(&self, t: &Prepared, cx: usize, cy: usize, radius: usize) -> f32 {
        let (mx, my) = (self.w - t.w, self.h - t.h);
        let mut best = f32::MIN;
        for y in cy.saturating_sub(radius)..=(cy + radius).min(my) {
            for x in cx.saturating_sub(radius)..=(cx + radius).min(mx) {
                best = best.max(self.score(t, x, y));
            }
        }
        best
    }
}

/// Eight-lane dot product; the lanes let the compiler vectorise it.
#[inline]
fn dot8(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let ((ca, ra), (cb, rb)) = (a.as_chunks::<8>(), b.as_chunks::<8>());
    for (x, y) in ca.iter().zip(cb) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// How a window is searched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchPlan {
    /// Offset step of the coarse pass. 1 scores every offset.
    pub step: usize,
    /// How many templates, best first, are refined after the coarse pass.
    pub refine: usize,
}

impl SearchPlan {
    /// The reader's plan: step 2, refine the best 8.
    pub const FAST: SearchPlan = SearchPlan { step: 2, refine: 8 };
    /// For the layout probe, which only needs the best score to be about right.
    pub const PROBE: SearchPlan = SearchPlan { step: 3, refine: 3 };
    /// Every offset of every template.
    pub const EXACT: SearchPlan = SearchPlan { step: 1, refine: 0 };
}

/// The best score of every template in `win`, in template order. A template
/// larger than the window scores `None` (the window ran off the frame).
pub fn best_scores(win: &Window, templates: &[&Prepared], plan: SearchPlan) -> Vec<Option<f32>> {
    let mut out: Vec<Option<f32>> = Vec::with_capacity(templates.len());
    let mut peaks = Vec::with_capacity(templates.len());
    for t in templates {
        if win.w < t.w || win.h < t.h || t.w == 0 || t.h == 0 {
            out.push(None);
            peaks.push((0, 0));
        } else {
            let (s, x, y) = win.best_on_grid(t, plan.step.max(1));
            out.push(Some(s));
            peaks.push((x, y));
        }
    }
    if plan.step > 1 && plan.refine > 0 {
        let mut order: Vec<usize> = (0..templates.len()).filter(|&i| out[i].is_some()).collect();
        order.sort_by(|&a, &b| {
            out[b]
                .partial_cmp(&out[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for &i in order.iter().take(plan.refine) {
            let (x, y) = peaks[i];
            let s = win.refine(templates[i], x, y, plan.step - 1);
            out[i] = out[i].map(|c| c.max(s));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plane of deterministic texture.
    fn noise(w: usize, h: usize, seed: u32) -> Plane {
        let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(12345);
        let px = (0..w * h)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((s >> 16) & 0xff) as f32
            })
            .collect();
        Plane { w, h, px }
    }

    /// Two passes of a 3x3 box blur: texture like a picture, not like static.
    fn smooth(p: &Plane) -> Plane {
        let mut cur = p.clone();
        for _ in 0..3 {
            let mut next = cur.clone();
            for y in 1..p.h - 1 {
                for x in 1..p.w - 1 {
                    let mut t = 0.0;
                    for dy in 0..3 {
                        for dx in 0..3 {
                            t += cur.px[(y + dy - 1) * p.w + x + dx - 1];
                        }
                    }
                    next.px[y * p.w + x] = t / 9.0;
                }
            }
            cur = next;
        }
        cur
    }

    fn crop(p: &Plane, x0: usize, y0: usize, w: usize, h: usize) -> Plane {
        let mut px = Vec::new();
        for y in y0..y0 + h {
            px.extend_from_slice(&p.px[y * p.w + x0..y * p.w + x0 + w]);
        }
        Plane { w, h, px }
    }

    #[test]
    fn a_template_cut_from_the_window_scores_one_at_its_offset() {
        let p = noise(60, 50, 1);
        let t = Prepared::new(&crop(&p, 17, 9, 20, 18));
        let win = Window::cut(&p, 0, 0, 60, 50);
        assert!((win.score(&t, 17, 9) - 1.0).abs() < 1e-3);
        assert!(win.score(&t, 18, 9) < 0.5);
    }

    #[test]
    fn score_ignores_brightness_and_contrast_but_not_the_pattern() {
        let p = noise(60, 50, 2);
        let t = Prepared::new(&crop(&p, 10, 10, 20, 18));
        let scaled = Plane {
            w: p.w,
            h: p.h,
            px: p.px.iter().map(|v| v * 0.5 + 40.0).collect(),
        };
        let win = Window::cut(&scaled, 0, 0, 60, 50);
        assert!((win.score(&t, 10, 10) - 1.0).abs() < 1e-3);
        let other = Window::cut(&noise(60, 50, 3), 0, 0, 60, 50);
        assert!(other.best_on_grid(&t, 1).0 < 0.5);
    }

    #[test]
    fn the_corner_mask_hides_a_badge() {
        let p = noise(60, 50, 4);
        let t = Prepared::new(&crop(&p, 10, 10, 30, 25));
        let mut marked = p.clone();
        // paint over the top-right corner of the template area
        for y in 10..10 + 7 {
            for x in 10 + 22..10 + 30 {
                marked.px[y * 60 + x] = 255.0;
            }
        }
        let win = Window::cut(&marked, 0, 0, 60, 50);
        assert!((win.score(&t, 10, 10) - 1.0).abs() < 1e-3);
        // a mark outside the corner does lower it
        let mut hit = p.clone();
        for y in 20..30 {
            for x in 10..20 {
                hit.px[y * 60 + x] = 0.0;
            }
        }
        let win = Window::cut(&hit, 0, 0, 60, 50);
        assert!(win.score(&t, 10, 10) < 0.95);
    }

    #[test]
    fn a_flat_window_scores_zero_not_nan() {
        let t = Prepared::new(&noise(12, 12, 5));
        let flat = Plane {
            w: 30,
            h: 30,
            px: vec![10.0; 900],
        };
        let win = Window::cut(&flat, 0, 0, 30, 30);
        assert_eq!(win.best_on_grid(&t, 1).0, 0.0);
    }

    #[test]
    fn the_fast_plan_finds_the_exact_best_for_the_matching_template() {
        let p = smooth(&noise(90, 80, 6));
        let ts: Vec<Prepared> = (0..6)
            .map(|i| Prepared::new(&smooth(&noise(24, 22, 100 + i))))
            .chain(std::iter::once(Prepared::new(&crop(&p, 31, 27, 24, 22))))
            .collect();
        let refs: Vec<&Prepared> = ts.iter().collect();
        let win = Window::cut(&p, 0, 0, 90, 80);
        for plan in [SearchPlan::FAST, SearchPlan::PROBE] {
            let fast = best_scores(&win, &refs, plan);
            let exact = best_scores(&win, &refs, SearchPlan::EXACT);
            let (fb, eb) = (fast[6].unwrap(), exact[6].unwrap());
            assert!((fb - eb).abs() < 1e-3, "{plan:?}: {fb} vs {eb}");
            let top = fast
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap()
                .0;
            assert_eq!(top, 6);
        }
    }

    #[test]
    fn a_window_smaller_than_the_template_scores_none() {
        let t = Prepared::new(&noise(20, 20, 7));
        let win = Window::cut(&noise(10, 30, 8), 0, 0, 10, 30);
        assert_eq!(best_scores(&win, &[&t], SearchPlan::FAST), vec![None]);
    }

    #[test]
    fn cut_clips_to_the_plane() {
        let win = Window::cut(&noise(10, 10, 9), 5, 5, 50, 50);
        assert_eq!((win.w, win.h), (5, 5));
        let win = Window::cut(&noise(10, 10, 9), 20, 20, 30, 30);
        assert_eq!((win.w, win.h), (0, 0));
    }
}
