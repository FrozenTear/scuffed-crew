//! The `sync_auth.json` pause file and the status text the GUI shows for
//! it: a refused or expired token, or a pending stats delete.

use std::path::Path;

/// Overview health copy when the server refused the bearer token.
/// The GUI shows this string; the hint is the Settings path.
pub const SYNC_TOKEN_REJECTED_STATUS: &str = "Sync token rejected: update the token in Settings";

/// Overview health copy when the server says the bearer token has expired.
/// The GUI shows this string; the member relinks from Settings.
pub const SYNC_TOKEN_EXPIRED_STATUS: &str = "Sync token expired, relink in Settings";

/// Overview health copy while a stats delete is pending. The GUI shows this
/// string. Automatic sync stays off until a new token is saved. Undoing the
/// delete does not restore the old token.
pub const SYNC_STATS_DELETE_PENDING_STATUS: &str = "Your site stats are scheduled for deletion. To resume syncing, undo it on the website account page, then make a new sync token there and paste it into the app.";

/// JSON `code` on HTTP 409 while a stats delete is pending.
pub(super) const STATS_DELETE_PENDING_CODE: &str = "stats_delete_pending";

pub(super) const AUTH_PAUSE_FILE: &str = "sync_auth.json";

/// Why periodic sync is paused for this URL and token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthPauseReason {
    /// HTTP 401 or 403, other than `token_expired`.
    Rejected,
    /// HTTP 401 with code `token_expired`.
    Expired,
    /// HTTP 409 with code `stats_delete_pending`. No automatic retry.
    /// Sync resumes when a new token is saved.
    DeletePending,
}

fn auth_pause_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join(AUTH_PAUSE_FILE)
}

/// Stable equality token for the bearer. Not a password hash, the file is
/// mode 0600 beside `config.toml`, which already stores the token. It only
/// answers "did this exact token change?".
fn token_fingerprint(token: &str) -> String {
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for (i, byte) in token.as_bytes().iter().enumerate() {
        h1 ^= u64::from(*byte);
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 ^= u64::from(*byte).wrapping_add(i as u64);
        h2 = h2.wrapping_mul(0xcbf29ce484222325);
    }
    format!("{h1:016x}{h2:016x}")
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AuthPauseFile {
    server_url: String,
    token_fingerprint: String,
    /// `"expired"` for [`AuthPauseReason::Expired`]. `"stats_delete_pending"`
    /// for [`AuthPauseReason::DeletePending`]. Absent on older files and on
    /// a plain rejection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// Raw `stats_delete_scheduled_at` from the 409 body. Formatted when the
    /// status line is read, so the date follows the machine's locale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stats_delete_scheduled_at: Option<String>,
}

/// Remember that this URL + token was refused (401/403). Mode 0600.
pub fn write_auth_pause(data_dir: &Path, server_url: &str, token: &str) -> std::io::Result<()> {
    write_auth_pause_reason(data_dir, server_url, token, AuthPauseReason::Rejected)
}

/// [`write_auth_pause`], with the GUI copy selected by `reason`.
pub fn write_auth_pause_reason(
    data_dir: &Path,
    server_url: &str,
    token: &str,
    reason: AuthPauseReason,
) -> std::io::Result<()> {
    write_auth_pause_file(data_dir, server_url, token, reason, None)
}

/// [`write_auth_pause_reason`] for a stats delete, keeping the server's
/// `stats_delete_scheduled_at` so the status line can show the local date.
pub fn write_delete_pending_pause(
    data_dir: &Path,
    server_url: &str,
    token: &str,
    scheduled_at: Option<&str>,
) -> std::io::Result<()> {
    write_auth_pause_file(
        data_dir,
        server_url,
        token,
        AuthPauseReason::DeletePending,
        scheduled_at,
    )
}

fn write_auth_pause_file(
    data_dir: &Path,
    server_url: &str,
    token: &str,
    reason: AuthPauseReason,
    scheduled_at: Option<&str>,
) -> std::io::Result<()> {
    if let Some(parent) = auth_pause_path(data_dir).parent() {
        std::fs::create_dir_all(parent)?;
        crate::fs_mode::tighten_private_dir(parent);
    }
    let scheduled_at = match reason {
        AuthPauseReason::DeletePending => scheduled_at
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        AuthPauseReason::Rejected | AuthPauseReason::Expired => None,
    };
    let file = AuthPauseFile {
        server_url: server_url.to_string(),
        token_fingerprint: token_fingerprint(token),
        reason: match reason {
            AuthPauseReason::Rejected => None,
            AuthPauseReason::Expired => Some("expired".to_string()),
            AuthPauseReason::DeletePending => Some(STATS_DELETE_PENDING_CODE.to_string()),
        },
        stats_delete_scheduled_at: scheduled_at,
    };
    let path = auth_pause_path(data_dir);
    let bytes = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, &path)?;
    crate::fs_mode::tighten_private_file(&path);
    Ok(())
}

pub fn clear_auth_pause(data_dir: &Path) {
    let _ = std::fs::remove_file(auth_pause_path(data_dir));
}

/// True when this saved pair is still paused.
///
/// A stats-delete pause matches the token alone, because a URL change does
/// not restore the old token. Other pauses match the URL and the token.
pub fn auth_pause_matches(data_dir: &Path, server_url: &str, token: &str) -> bool {
    auth_pause_status(data_dir, server_url, token).is_some()
}

/// Why this saved pair is paused. `None` when sync should keep running.
///
/// A stats-delete pause stays in force for the same token even if the saved
/// URL changed. A new token clears it.
pub fn auth_pause_reason(
    data_dir: &Path,
    server_url: &str,
    token: &str,
) -> Option<AuthPauseReason> {
    let bytes = std::fs::read(auth_pause_path(data_dir)).ok()?;
    let file = serde_json::from_slice::<AuthPauseFile>(&bytes).ok()?;
    let token_ok = file.token_fingerprint == token_fingerprint(token);
    let url_ok = file.server_url == server_url;
    let reason = match file.reason.as_deref() {
        Some("expired") => AuthPauseReason::Expired,
        Some(STATS_DELETE_PENDING_CODE) => AuthPauseReason::DeletePending,
        _ => AuthPauseReason::Rejected,
    };
    match reason {
        AuthPauseReason::DeletePending if token_ok => Some(AuthPauseReason::DeletePending),
        AuthPauseReason::DeletePending => None,
        other if token_ok && url_ok => Some(other),
        _ => None,
    }
}

/// GUI health line when this URL and token are paused. `None` when sync
/// should keep running. A stats-delete pause includes the local calendar
/// date when the pause file stored `stats_delete_scheduled_at`.
pub fn auth_pause_status(data_dir: &Path, server_url: &str, token: &str) -> Option<String> {
    let reason = auth_pause_reason(data_dir, server_url, token)?;
    let scheduled_at = match reason {
        AuthPauseReason::DeletePending => std::fs::read(auth_pause_path(data_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AuthPauseFile>(&bytes).ok())
            .and_then(|file| file.stats_delete_scheduled_at),
        AuthPauseReason::Rejected | AuthPauseReason::Expired => None,
    };
    Some(match reason {
        AuthPauseReason::Rejected => SYNC_TOKEN_REJECTED_STATUS.to_string(),
        AuthPauseReason::Expired => SYNC_TOKEN_EXPIRED_STATUS.to_string(),
        AuthPauseReason::DeletePending => delete_pending_status(scheduled_at.as_deref()),
    })
}

/// Status sentence for a pending stats delete. `scheduled_at` is the raw
/// server timestamp. A missing or unparseable value keeps the sentence
/// without a date.
pub fn delete_pending_status(scheduled_at: Option<&str>) -> String {
    match scheduled_at.and_then(format_local_date) {
        Some(date) => format!(
            "Your site stats are scheduled for deletion on {date}. To resume syncing, undo it on the website account page, then make a new sync token there and paste it into the app."
        ),
        None => SYNC_STATS_DELETE_PENDING_STATUS.to_string(),
    }
}

/// English calendar date: full month name, day, and year, for example
/// October 26, 2026. The day is the machine's local calendar day, with no
/// clock time. `None` when `raw` is not an RFC3339 timestamp.
pub fn format_local_date(raw: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(raw.trim()).ok()?;
    let formatted = parsed
        .with_timezone(&chrono::Local)
        .format("%B %-d, %Y")
        .to_string();
    if formatted.is_empty() {
        None
    } else {
        Some(formatted)
    }
}
