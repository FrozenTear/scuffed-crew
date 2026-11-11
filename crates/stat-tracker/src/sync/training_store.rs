//! The training crop files under `<data_dir>/training_crops/`, all mode 0600
//! in a 0700 folder:
//!
//! - `outbox/<kind>/<id>.json` and `<id>.png`: crops waiting to be sent,
//!   oldest id first. A crop is only written while the last consent the
//!   site gave for its kind is on, and a send pass drops a kind's crops as
//!   soon as the site says that consent is off. At most
//!   [`OUTBOX_KEEP_PER_KIND`] per kind; older ones go first.
//! - `consent.json`: the last consent the site gave ([`TrainingCropConsent`]),
//!   so the daemon knows which crops to keep before the next read. Absent
//!   or unreadable is all off.
//! - `status.json`: the warning of the last send pass, for the desktop app.
//! - `game_tokens.json`: the random `game_token` of recent games, by local
//!   session id. The session id never leaves the machine.
//! - `owner.json`: the website URL and a fingerprint of the token the
//!   outbox and cached consent belong to. When the saved website or token
//!   changes, everything above is dropped before any send, so crops kept
//!   under one site's consent never go to another.
//! - `deleted_at`: when the desktop app last asked the website to delete
//!   every own training crop (Unix milliseconds). A pass drops crops kept
//!   before it, and removes again by id what it sent while that delete ran.

use std::path::{Path, PathBuf};

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};
use serde::{Deserialize, Serialize};

use super::training::{QueuedCrop, UploadKind, check_crop, hex, map_name_meta, random_token};

const DIR: &str = "training_crops";
const OUTBOX: &str = "outbox";
const CONSENT_FILE: &str = "consent.json";
const STATUS_FILE: &str = "status.json";
const TOKENS_FILE: &str = "game_tokens.json";
const OWNER_FILE: &str = "owner.json";
const DELETED_AT_FILE: &str = "deleted_at";

/// Most crops kept per kind while they wait. About 20 games of map names.
pub const OUTBOX_KEEP_PER_KIND: usize = 40;

/// Recent games whose token is remembered, so both map crops of a game
/// share one token.
const TOKENS_KEEP: usize = 32;

fn root(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

fn kind_dir(data_dir: &Path, kind: UploadKind) -> PathBuf {
    root(data_dir).join(OUTBOX).join(kind.as_str())
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut at = Some(dir);
    while let Some(path) = at {
        crate::fs_mode::tighten_private_dir(path);
        if path.file_name().is_some_and(|name| name == DIR) {
            break;
        }
        at = path.parent();
    }
    Ok(())
}

/// Write `bytes` to `path` through a temp file, mode 0600.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        private_dir(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, path)?;
    crate::fs_mode::tighten_private_file(path);
    Ok(())
}

// ------------------------------------------------------------------ consent

/// The last consent the site gave. All off when none was read yet.
pub fn cached_training_consent(data_dir: &Path) -> TrainingCropConsent {
    std::fs::read(root(data_dir).join(CONSENT_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(super) fn write_cached_consent(
    data_dir: &Path,
    consent: TrainingCropConsent,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(&consent).map_err(std::io::Error::other)?;
    write_private(&root(data_dir).join(CONSENT_FILE), &bytes)
}

// ------------------------------------------------------------------- status

/// What the last send pass left for the desktop app.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingStatus {
    /// Why crops were not sent, in plain words. `None` after a pass that
    /// worked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// RFC 3339 time of the pass that set the warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// Crops were dropped. Such a warning stays until a pass sends
    /// something, so a later pass with nothing to send does not hide it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sticky: bool,
}

/// The status file, or `None` when there is none or it does not parse.
pub fn read_training_status(data_dir: &Path) -> Option<TrainingStatus> {
    let bytes = std::fs::read(root(data_dir).join(STATUS_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The warning to show, if the last pass left one.
pub fn training_warning(data_dir: &Path) -> Option<String> {
    read_training_status(data_dir).and_then(|status| status.warning)
}

/// Leave `warning` for the desktop app, or clear the last one (`None`).
/// A clear keeps a sticky warning unless `sent_some`.
pub(super) fn write_training_warning(
    data_dir: &Path,
    warning: Option<&str>,
    sticky: bool,
    sent_some: bool,
) {
    let path = root(data_dir).join(STATUS_FILE);
    let Some(warning) = warning else {
        let keep = !sent_some && read_training_status(data_dir).is_some_and(|s| s.sticky);
        if !keep {
            let _ = std::fs::remove_file(path);
        }
        return;
    };
    let status = TrainingStatus {
        warning: Some(warning.to_string()),
        at: Some(chrono::Utc::now().to_rfc3339()),
        sticky,
    };
    let written = serde_json::to_vec(&status)
        .map_err(std::io::Error::other)
        .and_then(|bytes| write_private(&path, &bytes));
    if let Err(error) = written {
        tracing::warn!(%error, "could not write the training crop status");
    }
}

// ------------------------------------------------------------------- outbox

/// Crop ids sort by the time they were kept: milliseconds, then random.
fn new_crop_id() -> Option<String> {
    let millis = chrono::Utc::now().timestamp_millis().max(0);
    let random = random_token()?;
    Some(format!("{millis:013}-{}", &random[..8]))
}

fn crop_ids(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".png").map(str::to_string)
        })
        .filter(|id| dir.join(format!("{id}.json")).is_file())
        .collect();
    ids.sort();
    ids
}

fn remove_crop(dir: &Path, id: &str) {
    let _ = std::fs::remove_file(dir.join(format!("{id}.png")));
    let _ = std::fs::remove_file(dir.join(format!("{id}.json")));
}

/// Keep one crop while `kind`'s cached consent is on. `false` when it was
/// not kept: no consent, a crop over the kind's caps, or a write error.
/// The meta is written first and the PNG last, so a half-written crop is
/// never read as whole.
pub fn keep_training_crop(
    data_dir: &Path,
    kind: UploadKind,
    meta: TrainingCropUploadMeta,
    png: Vec<u8>,
) -> bool {
    keep_crop(data_dir, kind, meta, png) == Kept::Yes
}

/// What [`keep_crop`] did with one crop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kept {
    Yes,
    /// The cached consent for the kind is off. Nothing was written.
    NoConsent,
    /// Over a cap or a `meta` bound ([`check_crop`]): skipped, not shrunk.
    Refused,
    /// No crop id or a write error: a consented crop was lost.
    Failed,
}

/// [`keep_training_crop`], telling a refusal from an error.
pub(super) fn keep_crop(
    data_dir: &Path,
    kind: UploadKind,
    meta: TrainingCropUploadMeta,
    png: Vec<u8>,
) -> Kept {
    if !kind.consented(&cached_training_consent(data_dir)) {
        return Kept::NoConsent;
    }
    let Some(id) = new_crop_id() else {
        return Kept::Failed;
    };
    let crop = QueuedCrop { id, meta, png };
    if let Err(refusal) = check_crop(kind, &crop) {
        tracing::debug!(kind = kind.as_str(), ?refusal, "training crop not kept");
        return Kept::Refused;
    }
    let dir = kind_dir(data_dir, kind);
    let meta = serde_json::to_vec(&crop.meta).unwrap_or_default();
    let written = write_private(&dir.join(format!("{}.json", crop.id)), &meta)
        .and_then(|()| write_private(&dir.join(format!("{}.png", crop.id)), &crop.png));
    if let Err(error) = written {
        tracing::warn!(%error, "could not keep a training crop");
        remove_crop(&dir, &crop.id);
        return Kept::Failed;
    }
    let ids = crop_ids(&dir);
    for old in ids
        .iter()
        .take(ids.len().saturating_sub(OUTBOX_KEEP_PER_KIND))
    {
        remove_crop(&dir, old);
    }
    Kept::Yes
}

/// A `map_name` crop for training, from the map-label PNG the tracker just
/// saved for `session_id` from an accepted Tab. `width` and `height` are the
/// capture size. Nothing is read or kept while the site's last answer has
/// `map_name` consent off.
pub fn keep_map_name_crop(
    data_dir: &Path,
    session_id: &str,
    png_path: &Path,
    width: u32,
    height: u32,
) -> bool {
    if !UploadKind::MapName.consented(&cached_training_consent(data_dir)) {
        return false;
    }
    let Some(token) = game_token_for(data_dir, session_id) else {
        return false;
    };
    let Ok(png) = std::fs::read(png_path) else {
        return false;
    };
    let meta = map_name_meta(&token, width, height);
    keep_training_crop(data_dir, UploadKind::MapName, meta, png)
}

/// The waiting crops of `kind`, oldest first. A crop whose files do not
/// parse is removed.
pub fn queued_crops(data_dir: &Path, kind: UploadKind) -> Vec<QueuedCrop> {
    let dir = kind_dir(data_dir, kind);
    let mut out = Vec::new();
    for id in crop_ids(&dir) {
        let meta = std::fs::read(dir.join(format!("{id}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let png = std::fs::read(dir.join(format!("{id}.png"))).ok();
        match (meta, png) {
            (Some(meta), Some(png)) => out.push(QueuedCrop { id, meta, png }),
            _ => remove_crop(&dir, &id),
        }
    }
    out
}

/// True when any kind has a crop waiting.
pub fn has_queued_crops(data_dir: &Path) -> bool {
    UploadKind::ALL
        .into_iter()
        .any(|kind| !crop_ids(&kind_dir(data_dir, kind)).is_empty())
}

pub(super) fn remove_crops(data_dir: &Path, kind: UploadKind, ids: &[String]) {
    let dir = kind_dir(data_dir, kind);
    for id in ids {
        remove_crop(&dir, id);
    }
}

/// Drop every waiting crop of `kind`.
pub fn drop_kind(data_dir: &Path, kind: UploadKind) {
    let _ = std::fs::remove_dir_all(kind_dir(data_dir, kind));
}

/// Drop every waiting crop, the cached consent and the game tokens: sync
/// is off, or the member deleted their training crops. The status file
/// stays, so a warning is not lost.
pub fn clear_training_outbox(data_dir: &Path) {
    let _ = std::fs::remove_dir_all(root(data_dir).join(OUTBOX));
    let _ = std::fs::remove_file(root(data_dir).join(TOKENS_FILE));
}

/// [`clear_training_outbox`], plus the cached consent, the owner and the
/// warning: sync is off or the website changed, so no crop is kept until
/// the site is read again, and there is no send to warn about.
pub fn forget_training_crops(data_dir: &Path) {
    clear_training_outbox(data_dir);
    let _ = std::fs::remove_file(root(data_dir).join(CONSENT_FILE));
    let _ = std::fs::remove_file(root(data_dir).join(STATUS_FILE));
    let _ = std::fs::remove_file(root(data_dir).join(OWNER_FILE));
}

// -------------------------------------------------------------------- owner

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Owner {
    server_url: String,
    token_fingerprint: String,
}

fn owner_of(server_url: &str, token: &str) -> Owner {
    Owner {
        server_url: server_url.trim().trim_end_matches('/').to_string(),
        token_fingerprint: super::auth_pause::token_fingerprint(token),
    }
}

/// Bind the training crop files to this website and token. When they
/// belonged to another, every waiting crop, the cached consent and the game
/// tokens are dropped first; `true` then. With no owner recorded yet the
/// pair is only recorded: consent is cached only by a send pass, which
/// records its owner before it reads the consent.
pub fn adopt_training_owner(data_dir: &Path, server_url: &str, token: &str) -> bool {
    let want = owner_of(server_url, token);
    let path = root(data_dir).join(OWNER_FILE);
    let have: Option<Owner> = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    if have.as_ref() == Some(&want) {
        return false;
    }
    let changed = path.exists();
    if changed {
        forget_training_crops(data_dir);
    }
    match serde_json::to_vec(&want) {
        Ok(bytes) => {
            if let Err(error) = write_private(&path, &bytes) {
                tracing::warn!(%error, "could not record the training crop owner");
            }
        }
        Err(error) => tracing::warn!(%error, "could not record the training crop owner"),
    }
    changed
}

// ------------------------------------------------------------ delete marker

/// Record that the member asked the website to delete every own training
/// crop, now. The desktop app calls this before `DELETE /mine`.
pub fn mark_training_delete(data_dir: &Path) -> std::io::Result<()> {
    let millis = chrono::Utc::now().timestamp_millis().max(0);
    write_private(
        &root(data_dir).join(DELETED_AT_FILE),
        millis.to_string().as_bytes(),
    )
}

/// When the last delete was asked for, in Unix milliseconds.
pub(super) fn training_delete_at(data_dir: &Path) -> Option<i64> {
    std::fs::read_to_string(root(data_dir).join(DELETED_AT_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The time a crop was kept, from its id.
pub(super) fn crop_kept_at(id: &str) -> Option<i64> {
    id.split('-').next()?.parse().ok()
}

// ------------------------------------------------------------- game tokens

#[derive(Debug, Default, Serialize, Deserialize)]
struct GameTokens {
    /// `(session id, token)`, newest last.
    games: Vec<(String, String)>,
}

/// The game token of `session_id`: the one already drawn for it, else a
/// new random one. `None` when no random token can be made or the file
/// cannot be written.
pub fn game_token_for(data_dir: &Path, session_id: &str) -> Option<String> {
    let path = root(data_dir).join(TOKENS_FILE);
    let mut tokens: GameTokens = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if let Some((_, token)) = tokens.games.iter().find(|(id, _)| id == session_id)
        && super::training::game_token_ok(token)
    {
        return Some(token.clone());
    }
    let token = random_token()?;
    tokens.games.retain(|(id, _)| id != session_id);
    tokens.games.push((session_id.to_string(), token.clone()));
    let excess = tokens.games.len().saturating_sub(TOKENS_KEEP);
    tokens.games.drain(..excess);
    let bytes = serde_json::to_vec(&tokens).ok()?;
    write_private(&path, &bytes).ok()?;
    Some(token)
}

/// A fresh `multipart/form-data` boundary.
pub(super) fn new_boundary() -> String {
    let random = random_token().unwrap_or_else(|| hex(&[0x5c; 16]));
    format!("scuffed-training-crops-{random}")
}
