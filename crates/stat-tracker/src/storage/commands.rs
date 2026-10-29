//! The GUI to daemon command queue: [`StoreCommand`] files in
//! `<data_dir>/commands/`, and [`LocalStore::apply_command`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::LocalStore;
use super::rules::MatchEdit;

/// A store mutation requested by the GUI. SurrealKV is single-process, so
/// while the daemon holds the store the GUI can't write — it queues commands
/// as JSON files in `<data_dir>/commands/`, which the daemon applies within a
/// few seconds (and then refreshes the live snapshot). When no daemon runs,
/// the GUI applies commands directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StoreCommand {
    /// Manually set a session's outcome ("victory"/"defeat"/"draw"/"unknown"),
    /// back-filled onto all its snapshots and re-queued for sync.
    SetOutcome { session_id: String, outcome: String },
    /// Remove a session and all its snapshots.
    DeleteSession { session_id: String },
    /// Apply a manual stat correction to a game (see [`MatchEdit`]). The
    /// corrected values overlay the immutable OCR reads and re-queue the game
    /// for sync.
    EditMatch { session_id: String, edit: MatchEdit },
    /// Confirm or dismiss a derived hero segment of a game (HS-1), keyed by
    /// segment index. `action` is [`SEGMENT_CONFIRM`], [`SEGMENT_DISMISS`],
    /// or [`SEGMENT_CLEAR`] to undo either (any other action also clears).
    /// Persists the decision and re-derives the hero timeline.
    ///
    /// [`SEGMENT_CONFIRM`]: super::SEGMENT_CONFIRM
    /// [`SEGMENT_DISMISS`]: super::SEGMENT_DISMISS
    /// [`SEGMENT_CLEAR`]: super::SEGMENT_CLEAR
    // The app's hero timeline queues this from its Confirm, Dismiss and
    // Undo buttons; a `resolve_segment` file dropped into
    // <data_dir>/commands/ is applied the same way.
    ResolveSegment {
        session_id: String,
        segment: u32,
        action: String,
    },
    /// Clear a quarantined upload error and queue the session again.
    RetryUpload { session_id: String },
    /// Clear every quarantined upload and queue those sessions again.
    RetryRejectedUploads,
}

impl LocalStore {
    /// Apply a queued store command (see [`StoreCommand`]).
    pub async fn apply_command(
        &self,
        cmd: &StoreCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _rev = self.rev_guard();
        match cmd {
            StoreCommand::SetOutcome {
                session_id,
                outcome,
            } => {
                // The raw outcome makes the row eligible for the next sync.
                // The overlay marks it edited so the upload carries edited=true.
                self.set_session_outcome(session_id, outcome).await?;
                self.edit_match(
                    session_id,
                    &MatchEdit {
                        outcome: Some(outcome.clone()),
                        ..MatchEdit::default()
                    },
                )
                .await
            }
            StoreCommand::DeleteSession { session_id } => self.delete_session(session_id).await,
            StoreCommand::EditMatch { session_id, edit } => self.edit_match(session_id, edit).await,
            StoreCommand::ResolveSegment {
                session_id,
                segment,
                action,
            } => {
                self.resolve_hero_segment(session_id, *segment, action)
                    .await
            }
            StoreCommand::RetryUpload { session_id } => self.clear_upload_reject(session_id).await,
            StoreCommand::RetryRejectedUploads => self.clear_rejected_uploads().await,
        }
    }
}

fn commands_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("commands")
}

/// Queue a command for the daemon to apply (atomic via tmp+rename).
pub fn queue_command(
    data_dir: &Path,
    cmd: &StoreCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = commands_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    crate::fs_mode::tighten_private_dir(data_dir);
    crate::fs_mode::tighten_private_dir(&dir);
    let name = format!(
        "cmd_{}_{}.json",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let tmp = dir.join(format!("{name}.tmp"));
    let dest = dir.join(&name);
    std::fs::write(&tmp, serde_json::to_vec(cmd)?)?;
    crate::fs_mode::tighten_private_file(&tmp);
    std::fs::rename(&tmp, &dest)?;
    crate::fs_mode::tighten_private_file(&dest);
    Ok(())
}

/// First phase of the command queue: list queued commands with their backing
/// files, oldest first, WITHOUT removing them. The caller applies each command
/// and then calls [`remove_command_file`] — two-phase, so a crash between read
/// and apply retries the command instead of silently losing a manual edit.
/// Unparseable files are removed immediately (never retried forever).
pub fn read_commands(data_dir: &Path) -> Vec<(PathBuf, StoreCommand)> {
    let Ok(entries) = std::fs::read_dir(commands_dir(data_dir)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut cmds = Vec::new();
    for f in files {
        match std::fs::read(&f)
            .map_err(|e| e.to_string())
            .and_then(|c| serde_json::from_slice::<StoreCommand>(&c).map_err(|e| e.to_string()))
        {
            Ok(cmd) => cmds.push((f, cmd)),
            Err(e) => {
                tracing::warn!(file = %f.display(), error = %e, "discarding bad command file");
                let _ = std::fs::remove_file(&f);
            }
        }
    }
    cmds
}

/// Second phase of the command queue: drop a command file once its command has
/// been successfully applied.
pub fn remove_command_file(path: &Path) {
    let _ = std::fs::remove_file(path);
}
