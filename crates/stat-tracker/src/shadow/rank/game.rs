//! What a game record gets from one rank update frame: the reader's
//! [`RankRead`] mapped to the core [`RankUpdate`], so storage and sync never
//! see a shadow type. The daemon's wiring (`daemon::rank`) calls
//! [`read_for_game`] on the rank screen's frames after a game.
//!
//! Stricter than [`Reader::read`]:
//!
//! * only a 2560x1440 frame is read (the size the pack was measured at); any
//!   other size is `None`, with no error and no log line;
//! * a frame that is not a rank update screen (another screen, or the
//!   placement screen, which has no "RANK PROGRESS:" label) is `None`;
//! * the role is kept only when the pack holds all three role icons
//!   ([`roles_complete`]): with fewer, an icon of a missing class would be
//!   judged against the others alone and read as one of them;
//! * more than [`crate::model::RANK_MAX_CHANGE_TAGS`] change tags is no tag
//!   list at all, as the site would null it;
//! * a value is kept only with a confidence in (0, 1].

use image::RgbImage;
use scuffed_types::api::RankRole;

use crate::model::{RANK_MAX_CHANGE_TAGS, RankNumber, RankTags, RankText, RankUpdate};

use super::{FRAME_H, FRAME_W, Family, Field, RankRead, Reader, Role};

/// The site's spelling of a role ([`RankRole`]): the reader's [`Role`] maps
/// onto it one to one, so the stored string is the one the site parses.
pub fn rank_role(r: Role) -> RankRole {
    match r {
        Role::Tank => RankRole::Tank,
        Role::Damage => RankRole::Damage,
        Role::Support => RankRole::Support,
        Role::Open => RankRole::Open,
    }
}

/// The pack holds the Tank, Damage and Support icons.
pub fn roles_complete(reader: &Reader) -> bool {
    Role::ICONS
        .iter()
        .all(|r| reader.templates().has_class(Family::Role, r.as_str()))
}

/// Read `img` for the game record: `None` unless it is a 2560x1440 rank
/// update screen with at least one field read.
pub fn read_for_game(reader: &Reader, img: &RgbImage) -> Option<RankUpdate> {
    if img.dimensions() != (FRAME_W, FRAME_H) {
        return None;
    }
    let read = reader.read(img).ok()?;
    let rank = to_rank_update(&read, roles_complete(reader));
    (!rank.is_empty()).then_some(rank)
}

/// `read` as the core type. `roles` is [`roles_complete`] for the pack
/// that read it; without it the role is `None`.
pub fn to_rank_update(read: &RankRead, roles: bool) -> RankUpdate {
    let role = if roles {
        kept(&read.role).and_then(|(r, confidence)| {
            let value = serde_json::to_value(rank_role(*r)).ok()?;
            Some(RankText {
                value: value.as_str()?.to_string(),
                confidence,
            })
        })
    } else {
        None
    };
    RankUpdate {
        role,
        role_rank: text(&read.role_rank),
        division: number(&read.division),
        progress_pct: number(&read.progress_pct),
        progress_change: number(&read.progress_change),
        change_tags: kept(&read.change_tags)
            .filter(|(tags, _)| tags.len() <= RANK_MAX_CHANGE_TAGS)
            .map(|(tags, confidence)| RankTags {
                value: tags.clone(),
                confidence,
            }),
        higher_than_pct: number(&read.higher_than_pct),
        hero_sr: number(&read.hero_sr),
        hero_sr_change: number(&read.hero_sr_change),
        next_mode_hint: text(&read.next_mode_hint),
    }
}

/// The value and its confidence, when there is a value and the confidence
/// is in (0, 1].
fn kept<T>(f: &Field<T>) -> Option<(&T, f32)> {
    let c = f.confidence;
    let v = f.value.as_ref()?;
    (c.is_finite() && c > 0.0 && c <= 1.0).then_some((v, c))
}

fn text(f: &Field<String>) -> Option<RankText> {
    kept(f).map(|(v, confidence)| RankText {
        value: v.clone(),
        confidence,
    })
}

fn number<T: Copy + Into<i64>>(f: &Field<T>) -> Option<RankNumber> {
    kept(f).map(|(v, confidence)| RankNumber {
        value: (*v).into(),
        confidence,
    })
}
