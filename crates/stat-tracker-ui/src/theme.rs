//! Rev 2 dark tokens from `docs/notes/stat-tracker-gui-redesign-2026-09-02.md`.
//!
//! Urbanist is the labelled *tracker product face* (OFL; bundled TTFs).
//! `radius-card` 16 is a documented product exception. Inner pad 12, page pad 24/32.

use iced::border::Radius;
use iced::font::{Family, Stretch, Style, Weight};
use iced::gradient::Linear;
use iced::widget::{button, container, toggler};
use iced::{Background, Border, Color, Degrees, Font, Shadow, Theme, Vector, theme};

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
/// Maps screen compact card — name, featured WR, games · W–L, win bar.
pub const HEIGHT_MAP: f32 = 148.0;
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

/// Translucent companion pane. Design §3: shadows only on this surface.
/// Compositor blur (`org_kde_kwin_blur`) is a follow-up, not v1.
pub fn companion_panel(_theme: &Theme) -> container::Style {
    let mut fill = SURFACE;
    fill.a = 0.88;
    container::Style {
        background: Some(Background::Color(fill)),
        text_color: Some(TEXT),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: card_radius(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

pub fn selected_surface_panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        text_color: Some(TEXT),
        border: Border {
            color: ACCENT,
            width: 2.0,
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
    stripe_color(outcome_color(outcome))
}

pub fn stripe_color(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(color)),
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

/// Header filter segment inside a season or role tray.
///
/// Selected treatment is the same shape for every group: a filled pill,
/// semibold light text. `accent` is the brand accent for season and the
/// role colour for role. Unselected segments are flat text in the tray,
/// not a second bordered pill.
pub fn filter_segment(
    accent: Color,
    selected: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hot = matches!(status, button::Status::Hovered | button::Status::Pressed);
        if selected {
            button::Style {
                background: Some(Background::Color(accent)),
                text_color: TEXT,
                border: Border {
                    color: accent,
                    width: 0.0,
                    radius: RADIUS_CHIP.into(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        } else {
            button::Style {
                background: hot.then_some(Background::Color(BORDER)),
                text_color: if hot { TEXT } else { TEXT_2 },
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: RADIUS_CHIP.into(),
                },
                shadow: Shadow::default(),
                snap: false,
            }
        }
    }
}

/// Companion show/hide switch. Track + knob, not a filter pill.
pub fn companion_toggle(_theme: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, hot) = match status {
        toggler::Status::Active { is_toggled } => (is_toggled, false),
        toggler::Status::Hovered { is_toggled } => (is_toggled, true),
        toggler::Status::Disabled { is_toggled } => (is_toggled, false),
    };
    let track = if on { ACCENT } else { SURFACE };
    let track_border = if on {
        ACCENT
    } else if hot {
        TEXT_3
    } else {
        BORDER
    };
    let knob = if on || hot { TEXT } else { TEXT_2 };
    toggler::Style {
        background: Background::Color(track),
        background_border_width: 1.0,
        background_border_color: track_border,
        foreground: Background::Color(knob),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(if on { TEXT } else { TEXT_2 }),
        border_radius: None,
        padding_ratio: 0.16,
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::{button, toggler};

    fn segment(accent: Color, selected: bool, status: button::Status) -> button::Style {
        filter_segment(accent, selected)(&iced_theme(), status)
    }

    #[test]
    fn header_filters_share_one_selected_treatment() {
        let season = segment(ACCENT, true, button::Status::Active);
        let role = segment(ROLE_TANK, true, button::Status::Active);
        assert_eq!(season.text_color, TEXT);
        assert_eq!(role.text_color, TEXT);
        assert_eq!(season.border.radius, role.border.radius);
        assert_eq!(season.border.width, role.border.width);
        assert_eq!(season.background, Some(Background::Color(ACCENT)));
        assert_eq!(role.background, Some(Background::Color(ROLE_TANK)));
        assert_eq!(
            segment(ROLE_DAMAGE, true, button::Status::Active).background,
            Some(Background::Color(ROLE_DAMAGE))
        );
        assert_eq!(
            segment(ROLE_SUPPORT, true, button::Status::Active).background,
            Some(Background::Color(ROLE_SUPPORT))
        );

        let off = segment(ACCENT, false, button::Status::Active);
        assert!(off.background.is_none());
        assert_eq!(off.text_color, TEXT_2);
        assert_ne!(off.background, season.background);
    }

    #[test]
    fn companion_toggle_is_a_switch_not_a_filled_chip() {
        let on = companion_toggle(&iced_theme(), toggler::Status::Active { is_toggled: true });
        let off = companion_toggle(&iced_theme(), toggler::Status::Active { is_toggled: false });
        assert_eq!(on.background, Background::Color(ACCENT));
        assert_eq!(on.foreground, Background::Color(TEXT));
        assert_eq!(off.background, Background::Color(SURFACE));
        assert_eq!(off.background_border_color, BORDER);
        assert_ne!(off.background, on.background);
        assert_eq!(on.text_color, Some(TEXT));
        assert_eq!(off.text_color, Some(TEXT_2));
        assert!(on.border_radius.is_none(), "switch track stays round");
    }
}
