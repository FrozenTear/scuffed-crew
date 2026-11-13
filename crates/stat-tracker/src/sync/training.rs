//! Opt-in training crops (`/api/training-crops`): the kinds this tracker
//! sends, the caps it checks before a crop leaves the machine, the `meta`
//! JSON and the hand-built `multipart/form-data` body.
//!
//! The contract is `docs/api/training-crops.md`; the shapes and caps come
//! from `scuffed_types`. Nothing here does I/O.

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};
use scuffed_types::{
    CropKind, HERO_ICON_MAX_FILES_PER_UPLOAD, HERO_ICON_MAX_REQUEST_BYTES, HERO_ICON_MIN_EDGE,
    TRAINING_CROP_GAME_TOKEN_LEN, TRAINING_CROP_META_MAX_BYTES, TRAINING_CROP_SCREEN_MAX_EDGE,
    TRAINING_CROP_SCREEN_MIN_EDGE,
};

/// A kind this tracker may send. There is no whole-screen kind: the server
/// refuses `full_screen` (422) and this type cannot name it, so no code path
/// can ask for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UploadKind {
    HeroIcon,
    StatDigits,
    MapName,
}

impl UploadKind {
    /// Every kind, in the order a send pass walks them.
    pub const ALL: [UploadKind; 3] = [
        UploadKind::HeroIcon,
        UploadKind::StatDigits,
        UploadKind::MapName,
    ];

    /// The shared kind.
    pub fn crop_kind(self) -> CropKind {
        match self {
            UploadKind::HeroIcon => CropKind::HeroIcon,
            UploadKind::StatDigits => CropKind::StatDigits,
            UploadKind::MapName => CropKind::MapName,
        }
    }

    /// The wire name (`?kind=`), also the outbox folder name.
    pub fn as_str(self) -> &'static str {
        self.crop_kind().as_str()
    }

    /// Parse a wire name. `full_screen` and anything else is `None`.
    pub fn parse(raw: &str) -> Option<UploadKind> {
        UploadKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == raw)
    }

    /// The member's consent for this kind in `consent`.
    pub fn consented(self, consent: &TrainingCropConsent) -> bool {
        consent.get(self.crop_kind())
    }

    /// Plain name for the desktop app.
    pub fn label(self) -> &'static str {
        match self {
            UploadKind::HeroIcon => "Hero icons",
            UploadKind::StatDigits => "Stat digits",
            UploadKind::MapName => "Map names",
        }
    }
}

/// One crop waiting in the outbox: its file stem, its `meta` and the PNG.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedCrop {
    pub id: String,
    pub meta: TrainingCropUploadMeta,
    pub png: Vec<u8>,
}

/// Why a crop is not sent. Such a crop is dropped, never shrunk: the point
/// of a training crop is the pixels the reader saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CropRefusal {
    /// No PNG signature or no readable header.
    NotPng,
    /// Width or height outside the kind's bounds.
    Edges { width: u32, height: u32 },
    /// Over the kind's byte limit.
    TooLarge(usize),
    /// A `meta` field out of bounds (the field name), or the `meta` JSON
    /// over its 4 KiB limit.
    Meta(&'static str),
    /// A `hero_icon` crop without a hero.
    NoHero,
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// Width and height from a PNG's `IHDR`, without decoding it.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || png[..8] != PNG_SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((width, height))
}

/// The `meta` part as sent: the fields that are set, nothing for the ones
/// that are not (no `null`s).
pub fn meta_json(meta: &TrainingCropUploadMeta) -> Vec<u8> {
    let mut value = serde_json::to_value(meta).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.retain(|_, field| !field.is_null());
    }
    serde_json::to_vec(&value).unwrap_or_default()
}

/// Check one crop against the kind's caps and the `meta` bounds before it
/// leaves the machine. The server checks the same and would refuse the
/// whole request for one bad crop.
pub fn check_crop(kind: UploadKind, crop: &QueuedCrop) -> Result<(), CropRefusal> {
    let (width, height) = png_size(&crop.png).ok_or(CropRefusal::NotPng)?;
    let (max_width, max_height) = kind.crop_kind().max_edges();
    let edge_ok = |value: u32, max: u32| (HERO_ICON_MIN_EDGE..=max).contains(&value);
    if !edge_ok(width, max_width) || !edge_ok(height, max_height) {
        return Err(CropRefusal::Edges { width, height });
    }
    if crop.png.len() > kind.crop_kind().max_crop_bytes() {
        return Err(CropRefusal::TooLarge(crop.png.len()));
    }
    crop.meta.validate().map_err(CropRefusal::Meta)?;
    if kind == UploadKind::HeroIcon && crop.meta.hero.is_none() {
        return Err(CropRefusal::NoHero);
    }
    if meta_json(&crop.meta).len() > TRAINING_CROP_META_MAX_BYTES {
        return Err(CropRefusal::Meta("meta"));
    }
    Ok(())
}

/// Room each pair takes in the body besides the `meta` JSON and the PNG:
/// the boundary lines and both part headers, rounded up.
const PAIR_OVERHEAD: usize = 512;
/// The closing boundary, rounded up.
const BODY_TAIL: usize = 128;

/// How many of `crops`, from the front, go in one request: at most 20
/// pairs and at most 2 MiB of body. Each crop must already have passed
/// [`check_crop`]. Zero only for an empty list.
pub fn batch_len(crops: &[QueuedCrop]) -> usize {
    let mut total = BODY_TAIL;
    let mut count = 0;
    for crop in crops.iter().take(HERO_ICON_MAX_FILES_PER_UPLOAD) {
        let size = PAIR_OVERHEAD + meta_json(&crop.meta).len() + crop.png.len();
        if total + size > HERO_ICON_MAX_REQUEST_BYTES {
            break;
        }
        total += size;
        count += 1;
    }
    count
}

/// `multipart/form-data` with `boundary`: a `meta` part, then its `crop`
/// part, for each crop in order. The shape the server reads (meta, crop,
/// meta, crop, ...).
pub fn multipart_body(boundary: &str, crops: &[QueuedCrop]) -> Vec<u8> {
    let mut body = Vec::new();
    for crop in crops {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"meta\"\r\nContent-Type: application/json\r\n\r\n",
        );
        body.extend_from_slice(&meta_json(&crop.meta));
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"crop\"; filename=\"crop.png\"\r\nContent-Type: image/png\r\n\r\n",
        );
        body.extend_from_slice(&crop.png);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// Exactly 32 lowercase hex characters, the shape the server takes.
pub fn game_token_ok(token: &str) -> bool {
    token.len() == TRAINING_CROP_GAME_TOKEN_LEN
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `bytes` as lowercase hex.
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 16 random bytes from the kernel, as 32 lowercase hex characters. `None`
/// when the random source cannot be read: then no crop is kept, rather
/// than falling back to a token that carries a time or an id.
pub fn random_token() -> Option<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .ok()?;
    // All zeros means the read did not fill the buffer in any useful way.
    if bytes.iter().all(|b| *b == 0) {
        return None;
    }
    Some(hex(&bytes))
}

/// The `hero` field for a read hero: the hero key (`soldier-76`, see
/// `scuffed_types::hero_key`), or `None` for a placeholder or a name the
/// hero list does not know, so the server never gets a guess.
pub fn hero_field(read: &str) -> Option<String> {
    scuffed_types::hero_key(read).map(str::to_string)
}

/// `meta` for a `map_name` crop: the game token, the screen it came from
/// and the capture size when the server would take it. The map banner box
/// sits at a fixed place on every screen size this tracker reads, so there
/// is no `found_spot` (the tracker sends that only for screen sizes it has
/// no layout for), and no hero.
pub fn map_name_meta(game_token: &str, width: u32, height: u32) -> TrainingCropUploadMeta {
    TrainingCropUploadMeta {
        game_token: Some(game_token.to_string()),
        ..tab_meta(width, height)
    }
}

/// `meta` every Tab crop starts from: the screen and the capture size when
/// the server would take it. No game token yet, and no `found_spot`: every
/// box this tracker cuts from a Tab is placed by its own layout or search
/// on every screen size, the same rule as [`map_name_meta`].
fn tab_meta(width: u32, height: u32) -> TrainingCropUploadMeta {
    let edge_ok =
        |v: u32| (TRAINING_CROP_SCREEN_MIN_EDGE..=TRAINING_CROP_SCREEN_MAX_EDGE).contains(&v);
    let size = (edge_ok(width) && edge_ok(height)).then_some((width, height));
    TrainingCropUploadMeta {
        screen: Some("tab".to_string()),
        screen_width: size.map(|(w, _)| w),
        screen_height: size.map(|(_, h)| h),
        ..TrainingCropUploadMeta::default()
    }
}

/// `meta` for a `hero_icon` crop of the career panel's portrait on a Tab of
/// `width` x `height`, without the game token
/// ([`super::keep_tab_crops`] adds it). `score` is the panel read's best
/// score and `lead` its lead over the runner-up, so `runner_up` is their
/// difference; `icon_set` is the hero pack that read it. `None` when `hero`
/// is not a known hero: the server requires one for this kind.
pub fn hero_icon_meta(
    width: u32,
    height: u32,
    hero: &str,
    score: f32,
    lead: f32,
    icon_set: &str,
) -> Option<TrainingCropUploadMeta> {
    let hero = hero_field(hero)?;
    let finite = |v: f32| v.is_finite().then_some(f64::from(v));
    let (score, margin) = (finite(score), finite(lead));
    Some(TrainingCropUploadMeta {
        hero: Some(hero),
        slot_type: Some("career_panel".to_string()),
        score,
        runner_up: score.zip(margin).map(|(s, m)| s - m),
        margin,
        icon_set_version: Some(icon_set.to_string()),
        ..tab_meta(width, height)
    })
}

/// The stat columns of a Tab row, left to right, as a `stat_digits` crop's
/// `slot_type`.
pub const STAT_DIGIT_COLUMNS: [&str; 6] = [
    "elims",
    "assists",
    "deaths",
    "damage",
    "healing",
    "mitigation",
];

/// `meta` for a `stat_digits` crop: stat column `column` (0 to 5, see
/// [`STAT_DIGIT_COLUMNS`]) of the member's own row on a Tab of `width` x
/// `height`, without the game token ([`super::keep_tab_crops`] adds it).
/// `hero` is the own hero when it is a known hero. `None` for a column past
/// the six.
pub fn stat_digits_meta(
    width: u32,
    height: u32,
    column: usize,
    hero: &str,
) -> Option<TrainingCropUploadMeta> {
    let slot = STAT_DIGIT_COLUMNS.get(column)?;
    Some(TrainingCropUploadMeta {
        hero: hero_field(hero),
        slot_type: Some((*slot).to_string()),
        ..tab_meta(width, height)
    })
}
