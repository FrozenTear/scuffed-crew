//! Software Overview renderer (ab_glyph + image). Used for headless PNGs
//! so empty / with-data states can be reviewed without wgpu.

use std::path::Path;

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont, point};
use image::{Rgba, RgbaImage};

use crate::aggregate::aggregate;
use crate::app::TrackerApp;
use crate::model::{Game, Role, SeasonSel};
use crate::overview::{self, TONIGHT_EMPTY};
use crate::theme::{self, rgb};

const W: u32 = 1280;
const H: u32 = 860;

struct Fonts<'a> {
    medium: FontRef<'a>,
    semibold: FontRef<'a>,
    bold: FontRef<'a>,
    extrabold: FontRef<'a>,
}

pub fn write_overview_png(app: &TrackerApp, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let img = render(app)?;
    img.save(path)?;
    Ok(())
}

fn render(app: &TrackerApp) -> anyhow::Result<RgbaImage> {
    let fonts = Fonts {
        medium: FontRef::try_from_slice(theme::FONT_BYTES_MEDIUM)?,
        semibold: FontRef::try_from_slice(theme::FONT_BYTES_SEMIBOLD)?,
        bold: FontRef::try_from_slice(theme::FONT_BYTES_BOLD)?,
        extrabold: FontRef::try_from_slice(theme::FONT_BYTES_EXTRABOLD)?,
    };
    let mut img = RgbaImage::from_pixel(W, H, rgba(theme::BG));
    let window = crate::seasons::window_for(&app.season, &app.seasons.seasons);
    let tonight = overview::tonight_games(&app.games, app.clock);
    let stats = aggregate(&app.games, window, None);
    let all_time = aggregate(&app.games, None, None);

    let mut y = theme::PAGE_PAD_Y as i32;
    let x = theme::PAGE_PAD_X as i32;
    let content_w = W as i32 - (theme::PAGE_PAD_X as i32) * 2;

    draw_text(
        &mut img,
        &fonts.extrabold,
        "Scuffed Tracker",
        x,
        y,
        theme::SIZE_FEATURED,
        theme::TEXT,
    );
    let mut chip_x = x + 320;
    if !app.seasons.seasons.is_empty() {
        chip_x = draw_chip(
            &mut img,
            &fonts.semibold,
            "All time",
            chip_x,
            y + 4,
            matches!(app.season, SeasonSel::AllTime),
        );
        for s in &app.seasons.seasons {
            let sel = matches!(app.season, SeasonSel::Season(ref id) if id == &s.id);
            let label = if s.is_current {
                format!("{} · current", s.name)
            } else {
                s.name.clone()
            };
            chip_x = draw_chip(&mut img, &fonts.semibold, &label, chip_x + 8, y + 4, sel);
        }
    }
    let _ = chip_x;
    draw_chip(
        &mut img,
        &fonts.medium,
        &app.live_status,
        W as i32 - theme::PAGE_PAD_X as i32 - 200,
        y + 4,
        false,
    );
    y += 56;

    y = draw_label(&mut img, &fonts.semibold, "TONIGHT", x, y);
    if tonight.is_empty() {
        fill_round(&mut img, x, y, content_w, 56, 16, theme::SURFACE);
        stroke_round(&mut img, x, y, content_w, 56, 16, theme::BORDER);
        draw_text(
            &mut img,
            &fonts.medium,
            TONIGHT_EMPTY,
            x + 16,
            y + 18,
            theme::SIZE_BODY,
            theme::TEXT_3,
        );
        y += 80;
    } else {
        y = draw_game_card(&mut img, &fonts, tonight[0], x, y, content_w, true);
        if tonight.len() > 1 {
            let gap = theme::GRID_GAP as i32;
            let n = (tonight.len() - 1) as i32;
            let cw = (content_w - gap * (n - 1).max(0)) / n.max(1);
            let mut cx = x;
            let mut max_y = y;
            for g in tonight.iter().skip(1) {
                let bottom = draw_game_card(&mut img, &fonts, g, cx, y, cw, false);
                max_y = max_y.max(bottom);
                cx += cw + gap;
            }
            y = max_y + 8;
        }
    }

    y = draw_label(&mut img, &fonts.semibold, "HEROES", x, y);
    if stats.heroes.is_empty() {
        draw_text(
            &mut img,
            &fonts.medium,
            "No heroes in this window",
            x,
            y,
            theme::SIZE_BODY,
            theme::TEXT_3,
        );
        y += 36;
    } else {
        let shown = stats.heroes.len().min(4) as i32;
        let gap = theme::GRID_GAP as i32;
        let cw = (content_w - gap * (shown - 1).max(0)) / shown.max(1);
        let mut cx = x;
        let mut max_y = y;
        for h in stats.heroes.iter().take(4) {
            let bottom = draw_hero_card(&mut img, &fonts, h, cx, y, cw);
            max_y = max_y.max(bottom);
            cx += cw + gap;
        }
        y = max_y + 16;
    }

    let gap = theme::GRID_GAP as i32;
    let pw = (content_w - gap * 2) / 3;
    draw_bottom_panel(
        &mut img,
        &fonts,
        x,
        y,
        pw,
        "SEASON",
        &format!("{:.0}%", stats.record.win_rate_pct()),
        &format!(
            "{}–{}–{} · {} games",
            stats.record.wins, stats.record.losses, stats.record.draws, stats.record.games
        ),
        if !matches!(app.season, SeasonSel::AllTime) {
            format!(
                "{:+.0} vs all time",
                (stats.record.win_rate() - all_time.record.win_rate()) * 100.0
            )
        } else {
            "All time".into()
        },
    );
    let maps_line = if stats.maps.is_empty() {
        "No maps in this window".into()
    } else {
        stats
            .maps
            .iter()
            .take(4)
            .map(|m| format!("{}  {:.0}%", m.map_name, m.record.win_rate_pct()))
            .collect::<Vec<_>>()
            .join("   ·   ")
    };
    draw_bottom_panel(
        &mut img,
        &fonts,
        x + pw + gap,
        y,
        pw,
        "MAPS",
        "",
        &maps_line,
        String::new(),
    );
    draw_bottom_panel(
        &mut img,
        &fonts,
        x + (pw + gap) * 2,
        y,
        pw,
        "TRACKER HEALTH",
        "",
        &app.health_status,
        "Companion overlay — P3".into(),
    );

    Ok(img)
}

fn draw_game_card(
    img: &mut RgbaImage,
    fonts: &Fonts<'_>,
    game: &Game,
    x: i32,
    y: i32,
    w: i32,
    featured: bool,
) -> i32 {
    let h = if featured { 168 } else { 132 };
    fill_role_card(img, x, y, w, h, game.role);
    stroke_round(img, x, y, w, h, 16, theme::BORDER);
    fill_rect(
        img,
        x,
        y + 8,
        theme::STRIPE as i32,
        h - 16,
        theme::outcome_color(game.outcome),
    );
    let tx = x + 16;
    let mut ty = y + 12;
    draw_text(
        img,
        &fonts.semibold,
        &game.role.label().to_ascii_uppercase(),
        tx,
        ty,
        theme::SIZE_LABEL,
        theme::TEXT_3,
    );
    ty += 18;
    let title_size = if featured {
        theme::SIZE_FEATURED
    } else {
        theme::SIZE_TITLE
    };
    draw_text(
        img,
        &fonts.extrabold,
        &game.map_name,
        tx,
        ty,
        title_size,
        theme::TEXT,
    );
    ty += if featured { 34 } else { 24 };
    draw_text(
        img,
        &fonts.medium,
        &format!("{}  ·  {}", game.hero, game.played_at.format("%H:%M")),
        tx,
        ty,
        theme::SIZE_BODY,
        theme::TEXT_2,
    );
    ty += 20;
    draw_text(
        img,
        &fonts.bold,
        &game.outcome.label().to_ascii_uppercase(),
        tx,
        ty,
        theme::SIZE_LABEL,
        theme::outcome_color(game.outcome),
    );
    if featured && game.has_stat_line() {
        ty += 22;
        let labels = ["E", "D", "A", "DMG", "HEAL", "MIT"];
        let vals = [
            fmt_stat(game.elims),
            fmt_stat(game.deaths),
            fmt_stat(game.assists),
            fmt_stat(game.damage),
            fmt_stat(game.healing),
            fmt_stat(game.mitigation),
        ];
        let mut sx = tx;
        for (l, v) in labels.iter().zip(vals.iter()) {
            fill_round(img, sx, ty, 72, 40, 12, theme::BG);
            stroke_round(img, sx, ty, 72, 40, 12, theme::BORDER);
            draw_text(
                img,
                &fonts.semibold,
                l,
                sx + 8,
                ty + 4,
                theme::SIZE_LABEL,
                theme::TEXT_3,
            );
            draw_text(
                img,
                &fonts.bold,
                v,
                sx + 8,
                ty + 18,
                theme::SIZE_BODY,
                theme::TEXT,
            );
            sx += 80;
        }
    }
    y + h + theme::GRID_GAP as i32
}

fn draw_hero_card(
    img: &mut RgbaImage,
    fonts: &Fonts<'_>,
    hero: &crate::aggregate::HeroAgg,
    x: i32,
    y: i32,
    w: i32,
) -> i32 {
    let h = 148;
    fill_role_card(img, x, y, w, h, hero.role);
    stroke_round(img, x, y, w, h, 16, theme::BORDER);
    fill_rect(img, x, y + 8, theme::STRIPE as i32, h - 16, theme::TEXT_3);
    let tx = x + 16;
    draw_text(
        img,
        &fonts.semibold,
        &hero.role.label().to_ascii_uppercase(),
        tx,
        y + 12,
        theme::SIZE_LABEL,
        theme::TEXT_3,
    );
    draw_text(
        img,
        &fonts.bold,
        &hero.hero,
        tx,
        y + 30,
        theme::SIZE_TITLE,
        theme::TEXT,
    );
    draw_text(
        img,
        &fonts.extrabold,
        &format!("{:.0}%", hero.record.win_rate_pct()),
        tx,
        y + 56,
        theme::SIZE_FEATURED,
        theme::TEXT,
    );
    draw_text(
        img,
        &fonts.medium,
        &format!("{} games", hero.record.games),
        tx,
        y + 92,
        theme::SIZE_META,
        theme::TEXT_2,
    );
    let bar_w = w - 32;
    fill_round(img, tx, y + 118, bar_w, 6, 3, theme::BORDER);
    let fill = ((hero.record.win_rate() * bar_w as f32).round() as i32).max(4);
    fill_round(img, tx, y + 118, fill, 6, 3, theme::OK);
    y + h + theme::GRID_GAP as i32
}

#[allow(clippy::too_many_arguments)]
fn draw_bottom_panel(
    img: &mut RgbaImage,
    fonts: &Fonts<'_>,
    x: i32,
    y: i32,
    w: i32,
    title: &str,
    big: &str,
    mid: &str,
    small: String,
) {
    let h = 168;
    fill_round(img, x, y, w, h, 16, theme::SURFACE);
    stroke_round(img, x, y, w, h, 16, theme::BORDER);
    draw_text(
        img,
        &fonts.semibold,
        title,
        x + 16,
        y + 14,
        theme::SIZE_LABEL,
        theme::TEXT_3,
    );
    let mut ty = y + 36;
    if !big.is_empty() {
        draw_text(
            img,
            &fonts.extrabold,
            big,
            x + 16,
            ty,
            theme::SIZE_FEATURED,
            theme::TEXT,
        );
        ty += 40;
    }
    if !mid.is_empty() {
        draw_wrapped(
            img,
            &fonts.medium,
            mid,
            x + 16,
            ty,
            w - 32,
            theme::SIZE_BODY,
            theme::TEXT_2,
        );
        ty += 28;
    }
    if !small.is_empty() {
        draw_text(
            img,
            &fonts.medium,
            &small,
            x + 16,
            ty,
            theme::SIZE_META,
            theme::TEXT_3,
        );
    }
}

fn draw_chip(
    img: &mut RgbaImage,
    font: &FontRef<'_>,
    label: &str,
    x: i32,
    y: i32,
    selected: bool,
) -> i32 {
    let w = (text_width(font, label, theme::SIZE_META) as i32) + 28;
    let bg = if selected {
        theme::ACCENT
    } else {
        theme::SURFACE
    };
    fill_round(img, x, y, w, 28, 14, bg);
    if !selected {
        stroke_round(img, x, y, w, 28, 14, theme::BORDER);
    }
    draw_text(
        img,
        font,
        label,
        x + 14,
        y + 6,
        theme::SIZE_META,
        if selected { theme::TEXT } else { theme::TEXT_2 },
    );
    x + w
}

fn draw_label(img: &mut RgbaImage, font: &FontRef<'_>, s: &str, x: i32, y: i32) -> i32 {
    draw_text(img, font, s, x, y, theme::SIZE_LABEL, theme::TEXT_3);
    y + 22
}

fn fmt_stat(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

fn rgba(c: iced::Color) -> Rgba<u8> {
    let [r, g, b] = rgb(c);
    Rgba([r, g, b, 255])
}

fn mix(a: iced::Color, b: iced::Color, t: f32) -> iced::Color {
    iced::Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}

fn fill_role_card(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, role: Role) {
    let role_c = theme::role_color(role);
    for iy in 0..h {
        for ix in 0..w {
            if !inside_round(ix, iy, w, h, 16) {
                continue;
            }
            let t = (ix as f32 / w.max(1) as f32 * 0.85 + iy as f32 / h.max(1) as f32 * 0.15)
                .clamp(0.0, 1.0);
            let tint = mix(role_c, theme::SURFACE, 0.67);
            put(img, x + ix, y + iy, mix(tint, theme::SURFACE, t));
        }
    }
}

fn fill_rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: iced::Color) {
    for iy in 0..h {
        for ix in 0..w {
            put(img, x + ix, y + iy, c);
        }
    }
}

fn fill_round(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, r: i32, c: iced::Color) {
    for iy in 0..h {
        for ix in 0..w {
            if inside_round(ix, iy, w, h, r) {
                put(img, x + ix, y + iy, c);
            }
        }
    }
}

fn stroke_round(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, r: i32, c: iced::Color) {
    for iy in 0..h {
        for ix in 0..w {
            if inside_round(ix, iy, w, h, r)
                && (!inside_round(ix - 1, iy, w, h, r)
                    || !inside_round(ix + 1, iy, w, h, r)
                    || !inside_round(ix, iy - 1, w, h, r)
                    || !inside_round(ix, iy + 1, w, h, r))
            {
                put(img, x + ix, y + iy, c);
            }
        }
    }
}

fn inside_round(ix: i32, iy: i32, w: i32, h: i32, r: i32) -> bool {
    if ix < 0 || iy < 0 || ix >= w || iy >= h {
        return false;
    }
    let r = r.min(w / 2).min(h / 2).max(0);
    let cx = if ix < r {
        ix - r
    } else if ix >= w - r {
        ix - (w - r - 1)
    } else {
        0
    };
    let cy = if iy < r {
        iy - r
    } else if iy >= h - r {
        iy - (h - r - 1)
    } else {
        0
    };
    if cx == 0 || cy == 0 {
        true
    } else {
        cx * cx + cy * cy <= r * r
    }
}

fn put(img: &mut RgbaImage, x: i32, y: i32, c: iced::Color) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x < img.width() && y < img.height() {
        img.put_pixel(x, y, rgba(c));
    }
}

fn draw_text(
    img: &mut RgbaImage,
    font: &FontRef<'_>,
    text: &str,
    x: i32,
    y: i32,
    size: f32,
    color: iced::Color,
) {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let mut caret = x as f32;
    let baseline = y as f32 + scaled.ascent();
    let [r, g, b] = rgb(color);
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, point(caret, baseline));
        if let Some(og) = font.outline_glyph(glyph) {
            let bounds = og.px_bounds();
            og.draw(|gx, gy, v| {
                if v < 0.05 {
                    return;
                }
                let px = (bounds.min.x + gx as f32).round() as i32;
                let py = (bounds.min.y + gy as f32).round() as i32;
                blend(img, px, py, r, g, b, v);
            });
        }
        caret += scaled.h_advance(gid);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_wrapped(
    img: &mut RgbaImage,
    font: &FontRef<'_>,
    text: &str,
    x: i32,
    y: i32,
    max_w: i32,
    size: f32,
    color: iced::Color,
) {
    let mut line = String::new();
    let mut ty = y;
    for word in text.split_whitespace() {
        let trial = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if text_width(font, &trial, size) as i32 > max_w && !line.is_empty() {
            draw_text(img, font, &line, x, ty, size, color);
            ty += (size + 4.0) as i32;
            line = word.to_string();
        } else {
            line = trial;
        }
    }
    if !line.is_empty() {
        draw_text(img, font, &line, x, ty, size, color);
    }
}

fn text_width(font: &FontRef<'_>, text: &str, size: f32) -> f32 {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    text.chars()
        .map(|ch| scaled.h_advance(scaled.scaled_glyph(ch).id))
        .sum()
}

fn blend(img: &mut RgbaImage, x: i32, y: i32, r: u8, g: u8, b: u8, a: f32) {
    if x < 0 || y < 0 {
        return;
    }
    let (x, y) = (x as u32, y as u32);
    if x >= img.width() || y >= img.height() {
        return;
    }
    let p = img.get_pixel(x, y).0;
    let a = a.clamp(0.0, 1.0);
    let inv = 1.0 - a;
    img.put_pixel(
        x,
        y,
        Rgba([
            (r as f32 * a + p[0] as f32 * inv).round() as u8,
            (g as f32 * a + p[1] as f32 * inv).round() as u8,
            (b as f32 * a + p[2] as f32 * inv).round() as u8,
            255,
        ]),
    );
}
