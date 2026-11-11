//! `hero_icon` and `stat_digits` crops the daemon cut from an accepted Tab,
//! kept for training once the game is known.
//!
//! The daemon cuts and encodes the pixels; this file adds the game's random
//! token and keeps each crop the kind's caps allow ([`super::keep_training_crop`]'s
//! rules: cached consent, caps checked, never shrunk). A crop the caps
//! refuse is skipped with a debug line. A consented crop lost to an error
//! (no game token, a failed write, a crop that could not be encoded) leaves
//! the training warning for the desktop app ([`warn_crop_lost`]). Nothing
//! here can fail a Tab or a game: every path returns.

use std::path::Path;

use scuffed_types::api::training_crops::TrainingCropUploadMeta;

use super::training::UploadKind;
use super::training_store::{
    Kept, cached_training_consent, game_token_for, keep_crop, write_training_warning,
};

/// The site's last answer has `kind`'s consent on. The daemon asks before
/// it cuts anything.
pub fn training_kind_on(data_dir: &Path, kind: UploadKind) -> bool {
    kind.consented(&cached_training_consent(data_dir))
}

/// Keep `crops` of `kind` (each a `meta` without the game token, and its
/// PNG) for the game `session_id`. `Some(kept)`: how many were kept, the
/// rest refused by a cap or with consent off. `None`: an error lost a
/// consented crop, and the training warning is set. Nothing is read or
/// written while `kind`'s cached consent is off.
pub fn keep_tab_crops(
    data_dir: &Path,
    session_id: &str,
    kind: UploadKind,
    crops: Vec<(TrainingCropUploadMeta, Vec<u8>)>,
) -> Option<usize> {
    if crops.is_empty() || !training_kind_on(data_dir, kind) {
        return Some(0);
    }
    let Some(token) = game_token_for(data_dir, session_id) else {
        warn_crop_lost(data_dir, kind);
        return None;
    };
    let mut kept = 0;
    let mut lost = false;
    for (mut meta, png) in crops {
        meta.game_token = Some(token.clone());
        match keep_crop(data_dir, kind, meta, png) {
            Kept::Yes => kept += 1,
            Kept::Failed => lost = true,
            Kept::NoConsent | Kept::Refused => {}
        }
    }
    if lost {
        warn_crop_lost(data_dir, kind);
        return None;
    }
    Some(kept)
}

/// Leave the training warning: a consented crop of `kind` from a Tab could
/// not be kept on this PC. It stays until a send pass sends something.
pub fn warn_crop_lost(data_dir: &Path, kind: UploadKind) {
    tracing::warn!(
        kind = kind.as_str(),
        "a consented training crop could not be kept"
    );
    let text = format!(
        "Training crops: {} from a Tab could not be kept on this PC. Your games still record.",
        kind.label().to_lowercase()
    );
    write_training_warning(data_dir, Some(&text), true, false);
}
