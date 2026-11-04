//! The icon templates the cv-v1 bans and picks reader matches against.
//!
//! The art is the game's, so it is never committed. The reader loads it from
//! `<data_dir>/templates/bans-picks-v1/`: one PNG per hero, named by the
//! hero's pack key (`wrecking-ball.png`, `soldier-76.png`, the stems of
//! `scuffed_types::HERO_NAMES`), each [`TEMPLATE_W`] by [`TEMPLATE_H`] pixels,
//! a cut of the hero's tile at the hero select grid's native 2560x1440 scale.
//! An optional `no-preference.png` of the same size is the icon the game
//! shows for a player with no preferred hero. Without all 54 hero files
//! there is no template set and the reader stays off.

use std::path::{Path, PathBuf};

use image::RgbImage;

use super::imaging::{Gray, gradient, gray_from_rgb, resize_area};
use super::matcher::Prepared;

/// Folder under the tracker's data dir.
pub const TEMPLATE_DIR: &str = "templates/bans-picks-v1";
/// File stem of the "no preferred hero" icon.
pub const NO_PREFERENCE_KEY: &str = "no-preference";
/// Template size in pixels.
pub const TEMPLATE_W: u32 = 100;
pub const TEMPLATE_H: u32 = 96;

/// Where an icon is read, and so at what scale it is matched. The factor
/// is the template-to-screen ratio, times one half for the uses matched on
/// the half-size working plane (see [`Use::scale`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// A ban result slot (about 1.87 times the grid tile).
    Slot,
    /// The lobby ban icon (1.6 times).
    Lobby,
    /// A row in the hero select left column (0.8 times).
    Row,
    /// The local player's ban vote slot on the ban vote screen (0.465
    /// times), matched on the full-size plane.
    VoteSlot,
    /// A hero box of the ban vote screen's TEAM BAN SCORE strip (0.70
    /// times), matched on the full-size plane.
    Strip,
}

impl Use {
    pub const ALL: [Use; 5] = [Use::Slot, Use::Lobby, Use::Row, Use::VoteSlot, Use::Strip];

    /// Template scale on the plane the use is matched on. `Slot`, `Lobby`
    /// and `Row` are matched on the half-size plane, so their factor is the
    /// screen ratio times 0.5. `VoteSlot` and `Strip` are small icons
    /// (about 46 and 70 pixels wide) and are matched on the full-size
    /// plane, so their factor is the screen ratio itself.
    pub fn scale(self) -> f64 {
        match self {
            Use::Slot => 1.87 * 0.5,
            Use::Lobby => 1.6 * 0.5,
            Use::Row => 0.8 * 0.5,
            Use::VoteSlot => 0.465,
            Use::Strip => 0.70,
        }
    }

    fn index(self) -> usize {
        match self {
            Use::Slot => 0,
            Use::Lobby => 1,
            Use::Row => 2,
            Use::VoteSlot => 3,
            Use::Strip => 4,
        }
    }
}

#[derive(Debug)]
pub enum TemplateError {
    /// Hero files that are not in the folder or could not be read.
    Missing(Vec<String>),
    /// A file whose size is not [`TEMPLATE_W`] by [`TEMPLATE_H`].
    BadSize(PathBuf, u32, u32),
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemplateError::Missing(keys) => write!(
                f,
                "bans-picks-v1 templates missing or unreadable: {}",
                keys.join(", ")
            ),
            TemplateError::BadSize(path, w, h) => write!(
                f,
                "{} is {w}x{h}, want {TEMPLATE_W}x{TEMPLATE_H}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for TemplateError {}

/// The 54 hero keys, in `HERO_NAMES` order.
pub fn hero_keys() -> Vec<String> {
    scuffed_types::HERO_NAMES
        .iter()
        .map(|h| scuffed_types::pack_file_stem(h.file).to_string())
        .collect()
}

/// Templates prepared at every scale the reader uses.
pub struct TemplateSet {
    /// Hero keys first, then [`NO_PREFERENCE_KEY`] when present.
    keys: Vec<String>,
    heroes: usize,
    /// `prepared[use][class]`.
    prepared: [Vec<Prepared>; 5],
}

impl TemplateSet {
    /// Load from `<data_dir>/templates/bans-picks-v1/`.
    pub fn load(data_dir: &Path) -> Result<Self, TemplateError> {
        Self::load_dir(&data_dir.join(TEMPLATE_DIR))
    }

    /// Load from a folder of `<key>.png` files.
    pub fn load_dir(dir: &Path) -> Result<Self, TemplateError> {
        let mut missing = Vec::new();
        let mut heroes = Vec::new();
        for key in hero_keys() {
            match read_png(&dir.join(format!("{key}.png"))) {
                Some(img) => heroes.push((key, img)),
                None => missing.push(key),
            }
        }
        if !missing.is_empty() {
            return Err(TemplateError::Missing(missing));
        }
        let np = read_png(&dir.join(format!("{NO_PREFERENCE_KEY}.png")));
        for (key, img) in heroes
            .iter()
            .map(|(k, i)| (k.as_str(), i))
            .chain(np.iter().map(|i| (NO_PREFERENCE_KEY, i)))
        {
            if img.dimensions() != (TEMPLATE_W, TEMPLATE_H) {
                return Err(TemplateError::BadSize(
                    dir.join(format!("{key}.png")),
                    img.width(),
                    img.height(),
                ));
            }
        }
        Ok(Self::from_images(heroes, np))
    }

    /// Build from images already in memory. Sizes are not checked.
    pub fn from_images(heroes: Vec<(String, RgbImage)>, no_preference: Option<RgbImage>) -> Self {
        let n_heroes = heroes.len();
        let mut keys: Vec<String> = heroes.iter().map(|(k, _)| k.clone()).collect();
        let mut grays: Vec<Gray> = heroes.iter().map(|(_, i)| gray_from_rgb(i)).collect();
        if let Some(img) = &no_preference {
            keys.push(NO_PREFERENCE_KEY.to_string());
            grays.push(gray_from_rgb(img));
        }
        let prepare = |u: Use| -> Vec<Prepared> {
            grays
                .iter()
                .map(|g| Prepared::new(&gradient(&resize_area(g, u.scale()))))
                .collect()
        };
        Self {
            keys,
            heroes: n_heroes,
            prepared: Use::ALL.map(prepare),
        }
    }

    /// Hero count (the no-preference class is not one).
    pub fn hero_count(&self) -> usize {
        self.heroes
    }

    pub fn has_no_preference(&self) -> bool {
        self.keys.len() > self.heroes
    }

    /// Key of class `i`: a hero key, or [`NO_PREFERENCE_KEY`].
    pub fn key(&self, i: usize) -> &str {
        &self.keys[i]
    }

    /// Prepared templates for a use. With `with_no_preference` the
    /// no-preference class (when loaded) is last.
    pub(super) fn classes(&self, u: Use, with_no_preference: bool) -> Vec<&Prepared> {
        let all = &self.prepared[u.index()];
        let n = if with_no_preference {
            all.len()
        } else {
            self.heroes
        };
        all[..n].iter().collect()
    }
}

fn read_png(path: &Path) -> Option<RgbImage> {
    image::open(path).ok().map(|i| i.to_rgb8())
}
