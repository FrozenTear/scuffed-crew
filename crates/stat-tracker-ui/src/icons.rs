//! The PNG icons compiled into the GUI (0.6 top bar and Settings page).
//!
//! Each tab icon is one white alpha mask (`assets/tab-icons/<tab>.png`),
//! tinted when it is decoded: TEXT on the selected tab, TEXT_2 on the
//! others. The other icons are drawn in their colour. `assets/draw_icons.py`
//! draws the overview mask and the other icons. The five other tab masks
//! were rasterised once at 2x from the 0.6 design spec's SVG paths; the
//! spec is not in the repository, so they are kept as committed images
//! (`assets/draw_icons.py` says how to replace one). Iced 0.14 can decode
//! PNG bytes itself; this decodes them once up front so a tint can be
//! applied and every frame reuses one handle.

use std::sync::OnceLock;

use iced::Color;
use iced::widget::image;

use crate::model::Screen;
use crate::theme::{TEXT, TEXT_2};

/// An icon this crate draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// A tab icon, bright when the tab is selected.
    Tab(Screen, bool),
    /// The "need a look" chip's warning sign.
    Warn,
    /// The status strip's and the Website account's check.
    Check,
    /// The lock beside a setting the environment fixes.
    Lock,
    /// An open ADVANCED row's chevron.
    FoldOpen,
    /// A closed ADVANCED row's chevron.
    FoldClosed,
    /// The open Games row: Uploaded.
    Upload,
    /// The open Games row: Waiting to upload.
    Clock,
    /// The open Games row: Edit.
    Pencil,
    /// The open Games row: Report misread.
    Flag,
    /// The open Games row: Scoreboard picture.
    Picture,
    /// The warning sign in the loss colour: Upload rejected.
    WarnLoss,
}

const TABS: usize = Screen::all().len();
const FIXED: [Icon; 11] = [
    Icon::Warn,
    Icon::Check,
    Icon::Lock,
    Icon::FoldOpen,
    Icon::FoldClosed,
    Icon::Upload,
    Icon::Clock,
    Icon::Pencil,
    Icon::Flag,
    Icon::Picture,
    Icon::WarnLoss,
];
const COUNT: usize = TABS * 2 + FIXED.len();

impl Icon {
    /// Every icon, each once.
    pub fn all() -> Vec<Icon> {
        let mut all: Vec<Icon> = Screen::all()
            .into_iter()
            .flat_map(|s| [Icon::Tab(s, false), Icon::Tab(s, true)])
            .collect();
        all.extend(FIXED);
        all
    }

    fn index(self) -> usize {
        match self {
            Icon::Tab(screen, selected) => screen as usize * 2 + usize::from(selected),
            Icon::Warn => TABS * 2,
            Icon::Check => TABS * 2 + 1,
            Icon::Lock => TABS * 2 + 2,
            Icon::FoldOpen => TABS * 2 + 3,
            Icon::FoldClosed => TABS * 2 + 4,
            Icon::Upload => TABS * 2 + 5,
            Icon::Clock => TABS * 2 + 6,
            Icon::Pencil => TABS * 2 + 7,
            Icon::Flag => TABS * 2 + 8,
            Icon::Picture => TABS * 2 + 9,
            Icon::WarnLoss => TABS * 2 + 10,
        }
    }

    /// The embedded PNG.
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Icon::Tab(screen, _) => match screen {
                Screen::Overview => include_bytes!("../assets/tab-icons/overview.png"),
                Screen::Games => include_bytes!("../assets/tab-icons/games.png"),
                Screen::Heroes => include_bytes!("../assets/tab-icons/heroes.png"),
                Screen::Maps => include_bytes!("../assets/tab-icons/maps.png"),
                Screen::Seasons => include_bytes!("../assets/tab-icons/seasons.png"),
                Screen::Settings => include_bytes!("../assets/tab-icons/settings.png"),
            },
            Icon::Warn => include_bytes!("../assets/warn.png"),
            Icon::Check => include_bytes!("../assets/check.png"),
            Icon::Lock => include_bytes!("../assets/lock.png"),
            Icon::FoldOpen => include_bytes!("../assets/fold-open.png"),
            Icon::FoldClosed => include_bytes!("../assets/fold-closed.png"),
            Icon::Upload => include_bytes!("../assets/upload.png"),
            Icon::Clock => include_bytes!("../assets/clock.png"),
            Icon::Pencil => include_bytes!("../assets/pencil.png"),
            Icon::Flag => include_bytes!("../assets/flag.png"),
            Icon::Picture => include_bytes!("../assets/picture.png"),
            Icon::WarnLoss => include_bytes!("../assets/warn-loss.png"),
        }
    }

    /// The colour a mask is drawn in, or `None` for an icon drawn in its
    /// own colour.
    pub fn tint(self) -> Option<Color> {
        match self {
            Icon::Tab(_, true) => Some(TEXT),
            Icon::Tab(_, false) => Some(TEXT_2),
            _ => None,
        }
    }
}

/// The decoded icon, tinted when it is a mask.
pub fn decode(icon: Icon) -> Result<::image::RgbaImage, ::image::ImageError> {
    let mut rgba = ::image::load_from_memory(icon.bytes())?.to_rgba8();
    if let Some(color) = icon.tint() {
        let [r, g, b, _] = color.into_rgba8();
        for px in rgba.pixels_mut() {
            px.0[0] = r;
            px.0[1] = g;
            px.0[2] = b;
        }
    }
    Ok(rgba)
}

/// The icon's handle, decoded on first use and kept, so every frame reuses
/// it. A PNG that does not decode is logged once and drawn blank.
pub fn handle(icon: Icon) -> image::Handle {
    static HANDLES: [OnceLock<image::Handle>; COUNT] = [const { OnceLock::new() }; COUNT];
    HANDLES[icon.index()]
        .get_or_init(|| match decode(icon) {
            Ok(rgba) => image::Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw()),
            Err(e) => {
                tracing::warn!(?icon, error = %e, "icon PNG did not decode");
                image::Handle::from_rgba(1, 1, vec![0, 0, 0, 0])
            }
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_its_own_slot() {
        let all = Icon::all();
        assert_eq!(all.len(), COUNT);
        let mut seen: Vec<usize> = all.iter().map(|i| i.index()).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..COUNT).collect::<Vec<_>>());
    }

    /// The embedded bytes decode at their 2x size, and a tab's selected and
    /// other icons come out in their own colours from the one mask.
    #[test]
    fn the_embedded_icons_decode() {
        for icon in Icon::all() {
            let img = decode(icon).unwrap_or_else(|e| panic!("{icon:?}: {e}"));
            let want = match icon {
                Icon::Tab(..) => 32,
                Icon::Warn
                | Icon::Lock
                | Icon::Upload
                | Icon::Clock
                | Icon::Pencil
                | Icon::Flag
                | Icon::Picture
                | Icon::WarnLoss => 24,
                Icon::Check | Icon::FoldOpen | Icon::FoldClosed => 28,
            };
            assert_eq!((img.width(), img.height()), (want, want), "{icon:?}");
            assert!(img.pixels().any(|p| p.0[3] > 200), "{icon:?} is not blank");
        }
        for screen in Screen::all() {
            let on = decode(Icon::Tab(screen, true)).unwrap();
            let dim = decode(Icon::Tab(screen, false)).unwrap();
            let rgb = |c: Color| {
                let [r, g, b, _] = c.into_rgba8();
                [r, g, b]
            };
            for (img, color) in [(&on, TEXT), (&dim, TEXT_2)] {
                for p in img.pixels().filter(|p| p.0[3] > 0) {
                    assert_eq!(p.0[..3], rgb(color), "{screen:?}");
                }
            }
            let alpha = |img: &::image::RgbaImage| img.pixels().map(|p| p.0[3]).collect::<Vec<_>>();
            assert_eq!(alpha(&on), alpha(&dim), "{screen:?}: one mask");
            assert_ne!(on, dim, "{screen:?}: two tints");
        }
    }
}
