//! Test fixtures of the cv readers: synthetic boards, glyph packs and word
//! templates (no game art). One module per reader, moved out of
//! `heroes.rs`, `banner.rs` and `result.rs` so tests of other modules
//! depend on the fixtures, not on the reader's file.

pub(crate) mod heroes {
    //! Synthetic boards and templates for tests. No game art.
    use crate::shadow::heroes::*;
    use image::{Rgb, Rgba};
    use image::{RgbImage, RgbaImage};
    use std::path::Path;

    pub(crate) const PURPLE: [u8; 3] = [110, 20, 120];
    pub(crate) const YELLOW: [u8; 3] = [150, 125, 10];
    pub(crate) const TEAL: [u8; 3] = [20, 110, 120];

    /// Deterministic synthetic "portrait" for class `id`: a few coloured
    /// shapes on the background. No game art.
    pub(crate) fn portrait(id: u32, size: u32, bg: Option<[u8; 3]>) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
        if let Some(b) = bg {
            for p in img.pixels_mut() {
                *p = Rgba([b[0], b[1], b[2], 255]);
            }
        }
        let mut seed = id.wrapping_mul(2_654_435_761).wrapping_add(12345);
        let mut rnd = |m: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % m
        };
        let s = size as f32;
        for _ in 0..6 {
            let (cx, cy) = (rnd(1000) as f32 / 1000.0 * s, rnd(1000) as f32 / 1000.0 * s);
            let r = (0.12 + rnd(1000) as f32 / 1000.0 * 0.25) * s;
            let col = [rnd(256) as u8, rnd(256) as u8, rnd(256) as u8];
            let square = rnd(2) == 0;
            for y in 0..size {
                for x in 0..size {
                    let (dx, dy) = (x as f32 - cx, y as f32 - cy);
                    let inside = if square {
                        dx.abs() < r && dy.abs() < r
                    } else {
                        dx * dx + dy * dy < r * r
                    };
                    if inside {
                        img.put_pixel(x, y, Rgba([col[0], col[1], col[2], 255]));
                    }
                }
            }
        }
        img
    }

    /// Synthetic board: header bar, two team blocks of `team` rows with
    /// role column, separators and a portrait per row.
    pub(crate) fn board(ids: &[u32], team: usize, pitch: u32, colours: [[u8; 3]; 2]) -> RgbImage {
        let hh = (pitch as f32 * 0.39) as u32;
        let w = pitch * 14;
        let gap = pitch;
        let h = hh + 2 * team as u32 * pitch + gap + pitch;
        let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 24]));
        let x0 = pitch / 2;
        for y in 0..hh {
            for x in x0..w - pitch / 2 {
                img.put_pixel(x, y, Rgb([205, 208, 212]));
            }
        }
        for (t, colour) in colours.iter().enumerate() {
            let top = hh + t as u32 * (team as u32 * pitch + gap);
            for k in 0..team as u32 {
                let ry0 = top + k * pitch;
                let rh = pitch - 2;
                for y in ry0..ry0 + rh {
                    for x in x0..w - pitch / 2 {
                        img.put_pixel(x, y, Rgb(*colour));
                    }
                }
                let (px0, py0, px1, _) =
                    portrait_box((x0 as usize, 0, 0, 0), (ry0 as usize, (ry0 + rh) as usize));
                let p = portrait(ids[t * team + k as usize], (px1 - px0) as u32, None);
                for (x, y, px) in p.enumerate_pixels() {
                    if px.0[3] > 0 {
                        img.put_pixel(
                            px0 as u32 + x,
                            py0 as u32 + y,
                            Rgb([px.0[0], px.0[1], px.0[2]]),
                        );
                    }
                }
            }
        }
        img
    }

    /// Templates as the cutting script writes them: portrait on PURPLE with
    /// alpha marking the foreground, 95 px.
    pub(crate) fn template_dir(ids: &[u32]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write_templates(dir.path(), ids, 95, 0);
        dir
    }

    /// Write `hero-{id}.png` for each id into `dir` (created if missing):
    /// `size` px, drawn as synthetic portrait `id + art`, so two packs can
    /// hold different art under the same hero keys.
    pub(crate) fn write_templates(dir: &Path, ids: &[u32], size: u32, art: u32) {
        std::fs::create_dir_all(dir).unwrap();
        for &id in ids {
            let fg = portrait(id + art, size, None);
            let mut t = RgbaImage::new(size, size);
            for (x, y, p) in fg.enumerate_pixels() {
                let v = if p.0[3] > 0 {
                    Rgba([p.0[0], p.0[1], p.0[2], 255])
                } else {
                    Rgba([PURPLE[0], PURPLE[1], PURPLE[2], 0])
                };
                t.put_pixel(x, y, v);
            }
            t.save(dir.join(format!("hero-{id}.png"))).unwrap();
        }
    }

    /// A `special/skull_01.png` in `dir`.
    pub(crate) fn write_skull(dir: &Path) {
        std::fs::create_dir_all(dir.join("special")).unwrap();
        portrait(501, 74, Some(PURPLE))
            .save(dir.join("special/skull_01.png"))
            .unwrap();
    }
}

/// Synthetic glyph packs for tests. No game art.
pub(crate) mod banner {
    use crate::shadow::banner::*;
    use image::RgbImage;
    use image::{GrayImage, Rgb};
    use std::collections::HashMap;
    use std::path::Path;

    /// Synthetic 5x7-ish block glyphs, one distinct bit pattern per letter,
    /// scaled to `H` rows. Not the game font: only the pipeline is under test.
    pub(crate) fn glyph(c: char) -> Gray {
        // cap rows 7..25: the bar window adds room above and below, like
        // the real banner (the `|` reaches past the baseline)
        glyph_rows(c, 7)
    }

    /// [`glyph`] with the capital between rows `top` and 25. The accolade
    /// fixtures use 8, where maps-v1 keeps it ([`CAP_ROWS`]).
    pub(crate) fn glyph_rows(c: char, top: usize) -> Gray {
        let w = 12;
        let mut g = Gray::new(w, H);
        let code = c as u32;
        for y in top..25 {
            for x in 1..w - 1 {
                let cell = ((y - top) / 3) * 3 + (x - 1) / 4;
                let on = (code.wrapping_mul(2654435761) >> (cell % 29)) & 1 == 1 || x == 1;
                if on {
                    g.px[y * w + x] = 1.0;
                }
            }
        }
        g
    }

    pub(crate) const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ':6";
    pub(crate) const GAP: f64 = 2.0;
    pub(crate) const SPACE: f64 = 9.0;

    pub(crate) fn atlas() -> HashMap<char, Gray> {
        LETTERS.chars().map(|c| (c, glyph(c))).collect()
    }

    /// Write a synthetic pack (`atlas.json` plus one PNG per glyph) the way
    /// maps-v1 lays it out, with every glyph in `letters`.
    pub(crate) fn write_pack(dir: &Path, letters: &str) {
        write_pack_rows(dir, letters, 7);
    }

    /// [`write_pack`] with the [`accolade_atlas`] glyphs
    /// ([`accolade_glyph`], so `Ç` has its cedilla).
    pub(crate) fn write_accolade_pack(dir: &Path, letters: &str) {
        write_pack_of(dir, letters, accolade_glyph);
    }

    fn write_pack_rows(dir: &Path, letters: &str, top: usize) {
        write_pack_of(dir, letters, |c| glyph_rows(c, top));
    }

    fn write_pack_of(dir: &Path, letters: &str, glyph: impl Fn(char) -> Gray) {
        std::fs::create_dir_all(dir).unwrap();
        let mut entries = Vec::new();
        for c in letters.chars() {
            let g = glyph(c);
            let file = format!("g{:04x}.png", c as u32);
            let img = GrayImage::from_fn(g.w as u32, H as u32, |x, y| {
                image::Luma([(g.at(x as usize, y as usize) * 255.0).round() as u8])
            });
            img.save(dir.join(&file)).unwrap();
            entries.push(format!(
                "{}: {}",
                serde_json::json!(c.to_string()),
                serde_json::json!(file)
            ));
        }
        let json = format!(
            "{{\"height\": {H}, \"gap\": {GAP}, \"space\": {SPACE}, \"glyphs\": {{{}}}}}",
            entries.join(", ")
        );
        std::fs::write(dir.join("atlas.json"), json).unwrap();
    }

    /// Rows of the `Ç` glyph's cedilla, top and bottom edge: where maps-v1
    /// has it, under the baseline ([`CAP_ROWS`] ends at 25).
    pub(crate) const CEDILLA_ROWS: (usize, usize) = (25, 31);

    /// One accolade glyph: the capital where maps-v1 keeps it
    /// ([`CAP_ROWS`]). `Ç` is the `C` with a cedilla joined under it.
    pub(crate) fn accolade_glyph(c: char) -> Gray {
        if c == 'Ç' {
            return with_cedilla(CEDILLA_ROWS);
        }
        glyph_rows(c, 8)
    }

    /// The `C` glyph with a mark under it in `rows` (top and bottom edge).
    pub(crate) fn with_cedilla(rows: (usize, usize)) -> Gray {
        let mut g = glyph_rows('C', 8);
        for y in rows.0..rows.1 {
            for x in 4..8 {
                g.px[y * g.w + x] = 1.0;
            }
        }
        g
    }

    /// [`LETTERS`] plus `Ç`, for accolade packs and frames.
    pub(crate) fn accolade_letters() -> String {
        format!("{LETTERS}Ç")
    }

    /// Glyphs with the capital where maps-v1 keeps it, for accolade frames.
    pub(crate) fn accolade_atlas() -> HashMap<char, Gray> {
        accolade_letters()
            .chars()
            .map(|c| (c, accolade_glyph(c)))
            .collect()
    }

    /// What an accolade frame holds besides the map line.
    #[derive(Clone, Copy)]
    pub(crate) struct Accolade {
        pub size: (u32, u32),
        /// Left edge of the map line, as a fraction of the frame width
        /// (17.4% after VICTORY, 15.5% after DEFEAT).
        pub x: f64,
        /// Top edge of the capitals, as a fraction of the frame height.
        pub cap_top: f64,
        /// Height of the capitals, as a fraction of the frame height.
        pub cap: f64,
        /// White accent marks above the first and third glyph, joined to
        /// them (`Í`, `Ö`).
        pub accents: bool,
        /// The grey `MATCH TIME` line under the map.
        pub match_time: bool,
        /// A performance overlay's white text across the top of the frame,
        /// down to this fraction of the frame height.
        pub overlay_to: Option<f64>,
        /// The line's width over its natural width (1.0 is as drawn).
        pub stretch: f64,
    }

    impl Accolade {
        pub(crate) fn at(size: (u32, u32)) -> Self {
            Self {
                size,
                x: 0.174,
                cap_top: 0.0417,
                cap: 0.015,
                accents: false,
                match_time: true,
                overlay_to: None,
                stretch: 1.0,
            }
        }

        /// The line `px` rows below its measured place (above when negative).
        pub(crate) fn moved_px(self, px: f64) -> Self {
            Self {
                cap_top: self.cap_top + px / self.size.1 as f64,
                ..self
            }
        }
    }

    /// A dark frame with an accolade header: a team-coloured result word
    /// block at the left, `text` in white from the [`accolade_atlas`] glyphs
    /// with capitals `cap` of the frame height tall, and the match time line.
    pub(crate) fn frame_with_accolade(text: &str, a: Accolade) -> RgbImage {
        let marks: &[usize] = if a.accents { &[0, 2] } else { &[] };
        paint_accolade(&accolade_atlas(), GAP, SPACE, text, a, marks, (5.5, 0.0))
    }

    /// [`frame_with_accolade`] from any glyphs. `marks` are the glyphs (by
    /// index, spaces not counted) that get a white accent block above them,
    /// from `mark_rows.0` to `mark_rows.1` glyph rows above the capitals'
    /// top (0 joins the accent to its glyph).
    pub(crate) fn paint_accolade(
        glyphs: &HashMap<char, Gray>,
        gap: f64,
        space: f64,
        text: &str,
        a: Accolade,
        marks: &[usize],
        mark_rows: (f64, f64),
    ) -> RgbImage {
        let (w, h) = a.size;
        let mut img = RgbImage::from_pixel(w, h, Rgb([12, 14, 24]));
        let (wf, hf) = (w as f64, h as f64);
        // result word: tall, bright, team blue. Its darkest channel is dark.
        for y in (0.036 * hf) as u32..(0.085 * hf) as u32 {
            for x in (0.01 * wf) as u32..(0.145 * wf) as u32 {
                img.put_pixel(x, y, Rgb([20, 190, 255]));
            }
        }
        let t = compose(glyphs, gap, space, text).unwrap();
        let tpl = t.render(&vec![0; t.parts.len()]);
        // 17 glyph rows hold the capital
        let s = a.cap * hf / 17.0;
        let sx_scale = s * a.stretch;
        let (tw, th) = (
            (tpl.w as f64 * sx_scale).round() as u32,
            (H as f64 * s).round() as u32,
        );
        let scaled = tpl.resize(tw as usize, th as usize);
        let sx = (a.x * wf) as u32;
        let top = (a.cap_top * hf - 8.0 * s).round() as u32;
        for y in 0..th {
            for x in 0..tw {
                let v = scaled.at(x as usize, y as usize).clamp(0.0, 1.0);
                let c = (12.0 + 240.0 * v) as u8;
                img.put_pixel(sx + x, top + y, Rgb([c, c, c.max(24)]));
            }
        }
        let cap_top = (a.cap_top * hf).round() as u32;
        for &i in marks {
            let (px, g) = &t.parts[i];
            let gx = sx + ((*px as f64 + 3.0) * sx_scale) as u32;
            let gw = (((g.w as f64 - 6.0) * sx_scale) as u32).max(1);
            for y in cap_top - (mark_rows.0 * s) as u32..cap_top - (mark_rows.1 * s) as u32 {
                for x in gx..gx + gw {
                    img.put_pixel(x, y, Rgb([250, 250, 250]));
                }
            }
        }
        if let Some(to) = a.overlay_to {
            for y in (0.022 * hf) as u32..(to * hf) as u32 {
                for x in 0..w {
                    if x % 11 < 6 {
                        img.put_pixel(x, y, Rgb([255, 255, 255]));
                    }
                }
            }
        }
        if a.match_time {
            for y in (0.0667 * hf) as u32..(0.0815 * hf) as u32 {
                for x in sx..sx + (0.11 * wf) as u32 {
                    if (x - sx) % 14 < 9 {
                        img.put_pixel(x, y, Rgb([160, 164, 172]));
                    }
                }
            }
        }
        img
    }

    /// A dark 1920x1080 frame with a banner: `|` then `text` drawn from the
    /// synthetic glyphs at 1080p scale, then a tall `TIME` block.
    pub(crate) fn frame_with_banner(text: &str) -> RgbImage {
        let mut img = RgbImage::from_pixel(1920, 1080, Rgb([20, 24, 40]));
        let glyphs = atlas();
        let t = compose(&glyphs, GAP, SPACE, text).unwrap();
        let tpl = t.render(&vec![0; t.parts.len()]);
        // bar window: 1080p bar is 20 px tall at y 35; strip window adds 15%/12%
        let (bar_t, bar_h) = (35u32, 20u32);
        let wt = bar_t - 3;
        let win_h = bar_h + 3 + 2;
        let s = win_h as f64 / H as f64;
        let x0 = 1600u32;
        for y in bar_t..bar_t + bar_h {
            for x in x0..x0 + 2 {
                img.put_pixel(x, y, Rgb([235, 235, 235]));
            }
        }
        let sx = x0 + 10;
        let w = (tpl.w as f64 * s).round() as u32;
        let scaled = tpl.resize(w as usize, win_h as usize);
        for y in 0..win_h {
            for x in 0..w {
                let v = scaled.at(x as usize, y as usize).clamp(0.0, 1.0);
                let c = (20.0 + 215.0 * v) as u8;
                img.put_pixel(sx + x, wt + y, Rgb([c, c, c.max(40)]));
            }
        }
        let tx = sx + w + 20;
        for y in 32..57 {
            for x in tx..tx + 60 {
                if (x - tx) % 14 < 9 {
                    img.put_pixel(x, y, Rgb([240, 240, 240]));
                }
            }
        }
        img
    }
}

pub(crate) mod result {
    //! Synthetic word templates and frames. Not game art.
    use crate::shadow::ncc::Gray;
    use crate::shadow::result::*;
    use image::Rgb;
    use image::RgbImage;
    use std::path::Path;

    /// Synthetic word: vertical bars in a word-specific rhythm.
    pub(crate) fn word(seed: u32, w: usize, h: usize) -> Gray {
        let mut g = Gray::new(w, h);
        for y in 2..h - 2 {
            for x in 0..w {
                let on = (x / (2 + seed as usize)).is_multiple_of(2)
                    || (y + x * seed as usize).is_multiple_of(11);
                if on {
                    g.px[y * w + x] = 0.9;
                }
            }
        }
        g
    }

    pub(crate) fn lay(id: &str) -> Layout {
        Layout::by_id(id).unwrap()
    }

    /// Paint a work-scale word into a 2560x1440 frame at (x0, y0), doubled.
    pub(crate) fn paint_at(img: &mut RgbImage, x0: u32, y0: u32, t: &Gray, colour: [u8; 3]) {
        for y in 0..t.h * 2 {
            for x in 0..t.w * 2 {
                if t.at(x / 2, y / 2) > 0.4 {
                    img.put_pixel(x0 + x as u32, y0 + y as u32, Rgb(colour));
                }
            }
        }
    }

    /// Paint a work-scale word into a 2560x1440 frame inside a layout's region.
    pub(crate) fn paint(img: &mut RgbImage, layout: &Layout, t: &Gray, colour: [u8; 3]) {
        let [fx0, fy0, _, _] = layout.roi;
        let (x0, y0) = ((fx0 * 2560.0) as u32 + 20, (fy0 * 1440.0) as u32 + 10);
        paint_at(img, x0, y0, t, colour);
    }

    /// Write `<id>_<word>.png` files the way a pack ships them.
    pub(crate) fn write_pack(dir: &Path, files: &[(&str, &Gray)]) {
        std::fs::create_dir_all(dir).unwrap();
        for (name, g) in files {
            let img = image::GrayImage::from_fn(g.w as u32, g.h as u32, |x, y| {
                image::Luma([(g.at(x as usize, y as usize) * 255.0) as u8])
            });
            img.save(dir.join(name)).unwrap();
        }
    }
}
