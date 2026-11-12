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
//! - `status.json`: the warning of the last send pass, for the desktop app,
//!   and the delete marker a "press Delete my training crops again" warning
//!   was left under.
//! - `game_tokens.json`: the random `game_token` of recent games, by local
//!   session id. The session id never leaves the machine.
//! - `owner.json`: the website URL and a fingerprint of the token the
//!   outbox and cached consent belong to. When the saved website or token
//!   changes, the outbox, the cached consent and the game tokens are dropped
//!   before any send, so crops kept under one site's consent never go to
//!   another. A website change also drops the status and the ids still to
//!   delete; a new token for the same website keeps them.
//! - `deleted_at`: when the desktop app last asked the website to delete
//!   every own training crop (Unix milliseconds). A pass drops crops kept
//!   before it. When the file changed while an upload ran, the pass removes
//!   again by id what that upload stored.
//! - `deleted_ok`: the `deleted_at` value of the last delete the website
//!   answered with success. A "press Delete my training crops again"
//!   warning goes only once this equals a `deleted_at` newer than the one
//!   it was left under, so a failed delete does not hide it and a delete
//!   that worked hides it at once.
//! - `pending_deletes.json`: ids of crops the site stored that such a
//!   removal could not delete yet. Each pass tries them again before it
//!   sends anything, and warns until they are gone, the website changes or
//!   Upload games goes off.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use scuffed_types::api::training_crops::{TrainingCropConsent, TrainingCropUploadMeta};
use serde::{Deserialize, Serialize};

use super::training::{
    QueuedCrop, UploadKind, batch_len, check_crop, hex, map_name_meta, random_token,
};

const DIR: &str = "training_crops";
const OUTBOX: &str = "outbox";
const CONSENT_FILE: &str = "consent.json";
const STATUS_FILE: &str = "status.json";
const TOKENS_FILE: &str = "game_tokens.json";
const OWNER_FILE: &str = "owner.json";
const DELETED_AT_FILE: &str = "deleted_at";
const DELETED_OK_FILE: &str = "deleted_ok";
const PENDING_DELETES_FILE: &str = "pending_deletes.json";

/// Held while the owner is changed, and while a send pass writes what it
/// learned, so a pass that ran for the old website or token never writes
/// under the new one.
static OWNER_LOCK: Mutex<()> = Mutex::new(());

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
    /// Crops were dropped, or crops a delete left on the website are still
    /// there. Such a warning stays until a pass sends something or those
    /// crops are gone, so a later pass with nothing to send does not hide
    /// it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sticky: bool,
    /// The delete marker (see [`mark_training_delete`]) under which a send
    /// got no answer while the member deleted. The warning asks the member
    /// to delete again, and stays until a later delete the website answered
    /// with success (see [`deleted_again_since`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_again_at: Option<i64>,
}

/// The status file, or `None` when there is none or it does not parse.
pub fn read_training_status(data_dir: &Path) -> Option<TrainingStatus> {
    let bytes = std::fs::read(root(data_dir).join(STATUS_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The warning to show, if the last pass left one. An ask to delete again
/// that a delete the website answered has met is left out at once, before
/// the next pass clears it.
pub fn training_warning(data_dir: &Path) -> Option<String> {
    let status = read_training_status(data_dir)?;
    let warning = status.warning?;
    if !deleted_again_since(data_dir, status.delete_again_at) {
        return Some(warning);
    }
    let rest = warning
        .strip_prefix(super::training_pass::TRAINING_DELETE_AGAIN)
        .unwrap_or(&warning)
        .trim_start();
    (!rest.is_empty()).then(|| rest.to_string())
}

/// Leave `warning` for the desktop app, or clear the last one (`None`).
/// A clear keeps a sticky warning unless `clear_sticky` (a send worked, the
/// crops a delete left on the website are gone, or the member deleted
/// again). `delete_again_at` is saved with the warning (see
/// [`TrainingStatus::delete_again_at`]).
pub(super) fn write_training_warning(
    data_dir: &Path,
    warning: Option<&str>,
    sticky: bool,
    clear_sticky: bool,
    delete_again_at: Option<i64>,
) {
    let path = root(data_dir).join(STATUS_FILE);
    let Some(warning) = warning else {
        let keep = !clear_sticky && read_training_status(data_dir).is_some_and(|s| s.sticky);
        if !keep {
            let _ = std::fs::remove_file(path);
        }
        return;
    };
    let status = TrainingStatus {
        warning: Some(warning.to_string()),
        at: Some(chrono::Utc::now().to_rfc3339()),
        sticky,
        delete_again_at,
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
    if !kind.consented(&cached_training_consent(data_dir)) {
        return false;
    }
    let Some(id) = new_crop_id() else {
        return false;
    };
    let crop = QueuedCrop { id, meta, png };
    if let Err(refusal) = check_crop(kind, &crop) {
        tracing::debug!(kind = kind.as_str(), ?refusal, "training crop not kept");
        return false;
    }
    let dir = kind_dir(data_dir, kind);
    let meta = serde_json::to_vec(&crop.meta).unwrap_or_default();
    let written = write_private(&dir.join(format!("{}.json", crop.id)), &meta)
        .and_then(|()| write_private(&dir.join(format!("{}.png", crop.id)), &crop.png));
    if let Err(error) = written {
        tracing::warn!(%error, "could not keep a training crop");
        remove_crop(&dir, &crop.id);
        return false;
    }
    let ids = crop_ids(&dir);
    for old in ids
        .iter()
        .take(ids.len().saturating_sub(OUTBOX_KEEP_PER_KIND))
    {
        remove_crop(&dir, old);
    }
    true
}

/// A `map_name` crop for training, from the map-label PNG the tracker just
/// saved for `session_id` from a Tab the tracker keeps for its game
/// (`prepare_capture_row` returns Store). `width` and
/// `height` are the capture size. Nothing is read or kept while the site's
/// last answer has `map_name` consent off.
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

/// One waiting crop, or `None` when its files do not parse.
fn read_crop(dir: &Path, id: &str) -> Option<QueuedCrop> {
    let meta = std::fs::read(dir.join(format!("{id}.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())?;
    let png = std::fs::read(dir.join(format!("{id}.png"))).ok()?;
    Some(QueuedCrop {
        id: id.to_string(),
        meta,
        png,
    })
}

/// The waiting crops of `kind`, oldest first. A crop whose files do not
/// parse is removed.
pub fn queued_crops(data_dir: &Path, kind: UploadKind) -> Vec<QueuedCrop> {
    let dir = kind_dir(data_dir, kind);
    let mut out = Vec::new();
    for id in crop_ids(&dir) {
        match read_crop(&dir, &id) {
            Some(crop) => out.push(crop),
            None => remove_crop(&dir, &id),
        }
    }
    out
}

/// The next request's worth of `kind`, oldest first ([`batch_len`]). Only
/// the crops in it are read from disk. On the way, crops kept at or before
/// `deleted_at` (the delete marker the caller read), crops whose files do
/// not parse and crops over the kind's caps are removed, never sent.
pub(super) fn next_batch(
    data_dir: &Path,
    kind: UploadKind,
    deleted_at: Option<i64>,
) -> Vec<QueuedCrop> {
    let dir = kind_dir(data_dir, kind);
    let mut batch = Vec::new();
    for id in crop_ids(&dir) {
        let before_delete = deleted_at
            .zip(crop_kept_at(&id))
            .is_some_and(|(at, kept)| kept <= at);
        if before_delete {
            remove_crop(&dir, &id);
            continue;
        }
        let Some(crop) = read_crop(&dir, &id) else {
            remove_crop(&dir, &id);
            continue;
        };
        if let Err(refusal) = check_crop(kind, &crop) {
            tracing::info!(
                kind = kind.as_str(),
                ?refusal,
                "training crop over a cap: dropped, not sent"
            );
            remove_crop(&dir, &id);
            continue;
        }
        batch.push(crop);
        if batch_len(&batch) < batch.len() {
            // Full: this one waits for the next request.
            batch.pop();
            break;
        }
    }
    batch
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

/// Drop every waiting crop and the game tokens: the member deleted their
/// training crops, or the token changed. The cached consent and the status
/// file stay, so a warning is not lost.
pub fn clear_training_outbox(data_dir: &Path) {
    let _ = std::fs::remove_dir_all(root(data_dir).join(OUTBOX));
    let _ = std::fs::remove_file(root(data_dir).join(TOKENS_FILE));
}

/// [`clear_training_outbox`], plus the cached consent, the owner, the
/// warning and the ids still to delete: Upload games is off (sign out) or
/// the website changed, so no crop is kept until the site is read again,
/// and there is no send to warn about. The ids to delete go too: with no
/// token, or on another website, the tracker cannot remove them, so a crop
/// the old website kept stays there until the member connects the tracker
/// to that website again and presses Delete my training crops (the
/// website's own delete removes hero icons only). A
/// new token for the same website does not come here (see
/// [`adopt_training_owner`]).
pub fn forget_training_crops(data_dir: &Path) {
    clear_training_outbox(data_dir);
    let _ = std::fs::remove_file(root(data_dir).join(CONSENT_FILE));
    let _ = std::fs::remove_file(root(data_dir).join(STATUS_FILE));
    let _ = std::fs::remove_file(root(data_dir).join(OWNER_FILE));
    let _ = std::fs::remove_file(root(data_dir).join(PENDING_DELETES_FILE));
}

/// True when a failed send is worth a warning: a kind's cached consent is
/// on, a crop waits, or a crop still has to be deleted from the website.
/// A member who never turned a kind on sees nothing.
pub(super) fn training_in_use(data_dir: &Path) -> bool {
    let consent = cached_training_consent(data_dir);
    UploadKind::ALL
        .into_iter()
        .any(|kind| kind.consented(&consent))
        || has_queued_crops(data_dir)
        || !pending_deletes(data_dir).is_empty()
}

// ---------------------------------------------------------- pending deletes

/// Ids of crops the website stored during a delete that are still to be
/// removed by id.
pub(super) fn pending_deletes(data_dir: &Path) -> Vec<String> {
    std::fs::read(root(data_dir).join(PENDING_DELETES_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default()
}

/// Save the ids still to delete; none removes the file. Only hex ids are
/// kept, the shape the website gives and the delete call takes.
pub(super) fn write_pending_deletes(data_dir: &Path, ids: &[String]) {
    let path = root(data_dir).join(PENDING_DELETES_FILE);
    let mut keep: Vec<&String> = ids
        .iter()
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    keep.sort();
    keep.dedup();
    if keep.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    let written = serde_json::to_vec(&keep)
        .map_err(std::io::Error::other)
        .and_then(|bytes| write_private(&path, &bytes));
    if let Err(error) = written {
        tracing::warn!(%error, "could not save the training crops still to delete");
    }
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
/// tokens are dropped first; `true` then. On another website the status
/// and the ids still to delete go too ([`forget_training_crops`]). With
/// only a new token for the same website they stay: the website lets any
/// sync token of the member who owns a crop delete it by id, and answers
/// 404 `crop_not_found` for another member's crop, which a pass counts as
/// done. With no owner recorded yet the pair is only recorded: consent is
/// cached only by a send pass, which records its owner before it reads the
/// consent.
pub fn adopt_training_owner(data_dir: &Path, server_url: &str, token: &str) -> bool {
    let _owner = OWNER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let want = owner_of(server_url, token);
    let path = root(data_dir).join(OWNER_FILE);
    let have = recorded_owner(&path);
    if have.as_ref() == Some(&want) {
        return false;
    }
    let changed = path.exists();
    let same_site = have.is_some_and(|have| have.server_url == want.server_url);
    if changed && same_site {
        clear_training_outbox(data_dir);
        let _ = std::fs::remove_file(root(data_dir).join(CONSENT_FILE));
    } else if changed {
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

fn recorded_owner(path: &Path) -> Option<Owner> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

/// Run `write` only while the training crop files still belong to this
/// website and token, and hold off an owner change until it is done.
/// `None` when they belong to another (or to none: sync went off), so a
/// send pass that ran for the old pair writes nothing under the new one.
pub(super) fn while_owner<T>(
    data_dir: &Path,
    server_url: &str,
    token: &str,
    write: impl FnOnce() -> T,
) -> Option<T> {
    let _owner = OWNER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = root(data_dir).join(OWNER_FILE);
    (recorded_owner(&path) == Some(owner_of(server_url, token))).then(write)
}

// ------------------------------------------------------------ delete marker

/// Record that the member asked the website to delete every own training
/// crop, now. The desktop app calls this before `DELETE /mine`. The marker
/// written, for [`record_training_delete_ok`].
pub fn mark_training_delete(data_dir: &Path) -> std::io::Result<i64> {
    let millis = chrono::Utc::now().timestamp_millis().max(0);
    write_private(
        &root(data_dir).join(DELETED_AT_FILE),
        millis.to_string().as_bytes(),
    )?;
    Ok(millis)
}

/// Record that the website answered the delete marked as `marker` (the
/// value [`mark_training_delete`] returned) with success. The desktop app
/// calls this after `DELETE /mine` worked.
pub fn record_training_delete_ok(data_dir: &Path, marker: i64) -> std::io::Result<()> {
    write_private(
        &root(data_dir).join(DELETED_OK_FILE),
        marker.to_string().as_bytes(),
    )
}

fn read_marker(data_dir: &Path, file: &str) -> Option<i64> {
    std::fs::read_to_string(root(data_dir).join(file))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// True when an ask to delete again left under `asked_at` was met: the
/// last delete marked after it is the one the website answered with
/// success. A delete that failed (offline, 429) leaves it unmet.
pub(super) fn deleted_again_since(data_dir: &Path, asked_at: Option<i64>) -> bool {
    let Some(asked_at) = asked_at else {
        return false;
    };
    let marker = training_delete_at(data_dir);
    marker.is_some() && marker != Some(asked_at) && read_marker(data_dir, DELETED_OK_FILE) == marker
}

/// When the last delete was asked for, in Unix milliseconds. A send pass
/// reads it before it picks a batch, and again after the upload: any
/// change means a delete ran meanwhile. The two values are only compared
/// for equality, never as times, since the desktop app wrote this with its
/// own clock.
pub(super) fn training_delete_at(data_dir: &Path) -> Option<i64> {
    read_marker(data_dir, DELETED_AT_FILE)
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
