//! Rev 2 dark tokens from `docs/notes/stat-tracker-gui-redesign-2026-09-02.md`.
//!
//! Urbanist is the labelled *tracker product face* (OFL; bundled TTFs).
//! `radius-card` 16 is a documented product exception. Inner pad 12, page pad 24/32.

use iced::border::Radius;
use iced::font::{Family, Stretch, Style, Weight};
use iced::gradient::Linear;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Degrees, Font, Shadow, Theme, theme};

use crate::model::{Outcome, Role};

pub const FONT_BYTES_MEDIUM: &[u8] = include_bytes!("../fonts/Urbanist-Medium.ttf");
pub const FONT_BYTES_SEMIBOLD: &[u8] = include_bytes!("../fonts/Urbanist-SemiBold.ttf");
pub const FONT_BYTES_BOLD: &[u8] = include_bytes!("../fonts/Urbanist-Bold.ttf");
pub const FONT_BYTES_EXTRABOLD: &[u8] = include_bytes!("../fonts/Urbanist-ExtraBold.ttf");

const fn urbanist(weight: Weight) -> Font {
    Font {
        family: Family::Name("Urbanist"),
        weight,
        stretch: Stretch::Normal,
        style: Style::Normal,
    }
}

pub const FONT_MEDIUM: Font = urbanist(Weight::Medium);
pub const FONT_SEMIBOLD: Font = urbanist(Weight::Semibold);
pub const FONT_BOLD: Font = urbanist(Weight::Bold);
pub const FONT_EXTRABOLD: Font = urbanist(Weight::ExtraBold);

pub const BG: Color = Color::from_rgb(
    0x12 as f32 / 255.0,
    0x12 as f32 / 255.0,
    0x18 as f32 / 255.0,
);
pub const SURFACE: Color = Color::from_rgb(
    0x1c as f32 / 255.0,
    0x1c as f32 / 255.0,
    0x25 as f32 / 255.0,
);
pub const BORDER: Color = Color::from_rgb(
    0x2a as f32 / 255.0,
    0x2a as f32 / 255.0,
    0x36 as f32 / 255.0,
);
pub const TEXT: Color = Color::from_rgb(
    0xf2 as f32 / 255.0,
    0xf2 as f32 / 255.0,
    0xf7 as f32 / 255.0,
);
pub const TEXT_2: Color = Color::from_rgb(
    0xc9 as f32 / 255.0,
    0xc9 as f32 / 255.0,
    0xd6 as f32 / 255.0,
);
pub const TEXT_3: Color = Color::from_rgb(
    0x8d as f32 / 255.0,
    0x8d as f32 / 255.0,
    0xa0 as f32 / 255.0,
);
pub const ACCENT: Color = Color::from_rgb(
    0x8f as f32 / 255.0,
    0x73 as f32 / 255.0,
    0xff as f32 / 255.0,
);
pub const OK: Color = Color::from_rgb(
    0x46 as f32 / 255.0,
    0xd8 as f32 / 255.0,
    0xa4 as f32 / 255.0,
);
pub const DANGER: Color = Color::from_rgb(
    0xff as f32 / 255.0,
    0x5c as f32 / 255.0,
    0x7a as f32 / 255.0,
);
pub const WARN: Color = Color::from_rgb(
    0xf5 as f32 / 255.0,
    0xb8 as f32 / 255.0,
    0x4a as f32 / 255.0,
);
pub const ROLE_TANK: Color = Color::from_rgb(
    0x5b as f32 / 255.0,
    0x8d as f32 / 255.0,
    0xef as f32 / 255.0,
);
pub const ROLE_DAMAGE: Color = Color::from_rgb(
    0xff as f32 / 255.0,
    0x7a as f32 / 255.0,
    0x59 as f32 / 255.0,
);
pub const ROLE_SUPPORT: Color = OK;

/// Card radius — documented product exception (Nirify 16–20 range).
pub const RADIUS_CARD: f32 = 16.0;
/// Inner blocks / stat boxes.
pub const RADIUS_INNER: f32 = 12.0;
pub const RADIUS_CHIP: f32 = 999.0;
/// Card / inner-block padding (task: inner pad 12).
pub const PAD_INNER: f32 = 12.0;
pub const PAGE_PAD_Y: f32 = 24.0;
pub const PAGE_PAD_X: f32 = 32.0;
pub const GRID_GAP: f32 = 12.0;
/// Outcome stripe — 4 px, not a Fill-height child (that collapses in Row).
pub const STRIPE: f32 = 4.0;
pub const HEIGHT_FEATURED: f32 = 216.0;
pub const HEIGHT_COMPACT: f32 = 180.0;
pub const HEIGHT_HERO: f32 = 160.0;
/// Fixed sidebar; the content pane flexes (rev 3 — no left-pinned 1400 cap).
pub const SIDEBAR_WIDTH: f32 = 168.0;

pub const SIZE_LABEL: f32 = 11.0;
pub const SIZE_META: f32 = 13.0;
pub const SIZE_BODY: f32 = 14.0;
pub const SIZE_TITLE: f32 = 20.0;
pub const SIZE_FEATURED: f32 = 28.0;

pub fn iced_theme() -> Theme {
    Theme::custom(
        "scuffed-tracker",
        theme::Palette {
            background: BG,
            text: TEXT,
            primary: ACCENT,
            success: OK,
            warning: WARN,
            danger: DANGER,
        },
    )
}

pub fn role_color(role: Role) -> Color {
    match role {
        Role::Tank => ROLE_TANK,
        Role::Damage => ROLE_DAMAGE,
        Role::Support => ROLE_SUPPORT,
        Role::Unknown => TEXT_3,
    }
}

pub fn outcome_color(outcome: Outcome) -> Color {
    match outcome {
        Outcome::Win => OK,
        Outcome::Loss => DANGER,
        Outcome::Draw | Outcome::Unknown => TEXT_3,
    }
}

/// Role tint at alpha 0x55 over the card, fading to surface at 160°.
pub fn role_card_background(role: Role) -> Background {
    let mut tint = role_color(role);
    tint.a = 0x55 as f32 / 255.0;
    let linear = Linear::new(Degrees(160.0))
        .add_stop(0.0, tint)
        .add_stop(1.0, SURFACE);
    Background::Gradient(linear.into())
}

pub fn card_radius() -> Radius {
    RADIUS_CARD.into()
}

pub fn inner_radius() -> Radius {
    RADIUS_INNER.into()
}

pub fn page_background(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG)),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn surface_panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        text_color: Some(TEXT),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: card_radius(),
        },
        ..container::Style::default()
    }
}

pub fn role_card(role: Role) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(role_card_background(role)),
        text_color: Some(TEXT),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: card_radius(),
        },
        ..container::Style::default()
    }
}

pub fn stripe(outcome: Outcome) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(outcome_color(outcome))),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn stat_box(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG)),
        text_color: Some(TEXT),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: inner_radius(),
        },
        ..container::Style::default()
    }
}

pub fn chip(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, _status| {
        if selected {
            button::Style {
                background: Some(Background::Color(ACCENT)),
                text_color: TEXT,
                border: Border {
                    color: ACCENT,
                    width: 1.0,
                    radius: RADIUS_CHIP.into(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        } else {
            button::Style {
                background: Some(Background::Color(SURFACE)),
                text_color: TEXT_2,
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: RADIUS_CHIP.into(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        }
    }
}

pub fn role_chip(role: Role, selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    let accent = role_color(role);
    move |_theme, _status| {
        if selected {
            button::Style {
                background: Some(Background::Color(accent)),
                text_color: TEXT,
                border: Border {
                    color: accent,
                    width: 1.0,
                    radius: RADIUS_CHIP.into(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        } else {
            button::Style {
                background: Some(Background::Color(SURFACE)),
                text_color: TEXT_2,
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: RADIUS_CHIP.into(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        }
    }
}

pub fn nav_btn(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, _status| {
        if selected {
            button::Style {
                background: Some(Background::Color(ACCENT)),
                text_color: TEXT,
                border: Border {
                    color: ACCENT,
                    width: 1.0,
                    radius: inner_radius(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        } else {
            button::Style {
                background: Some(Background::Color(SURFACE)),
                text_color: TEXT_2,
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: inner_radius(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        }
    }
}

pub fn ghost_btn() -> impl Fn(&Theme, button::Status) -> button::Style {
    |_theme, _status| button::Style {
        background: Some(Background::Color(SURFACE)),
        text_color: TEXT,
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn danger_btn(armed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, _status| button::Style {
        background: Some(Background::Color(if armed { DANGER } else { SURFACE })),
        text_color: if armed { TEXT } else { DANGER },
        border: Border {
            color: DANGER,
            width: 1.0,
            radius: RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn text_input_style(
    _theme: &Theme,
    _status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    iced::widget::text_input::Style {
        background: Background::Color(BG),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: inner_radius(),
        },
        icon: TEXT_3,
        placeholder: TEXT_3,
        value: TEXT,
        selection: ACCENT,
    }
}
