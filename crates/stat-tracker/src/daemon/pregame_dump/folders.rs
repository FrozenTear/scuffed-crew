//! The pre-game saves' folders on disk: names, order, pruning, and the
//! writes the writer thread makes. A child module of [`super`].

use super::*;

/// The pre-game folder under the data dir.
pub(crate) fn pregame_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("debug").join("pregame")
}

/// The folder name for a game that starts at `at`.
pub(crate) fn pregame_folder_name(at: chrono::DateTime<Utc>) -> String {
    at.format(FOLDER_FORMAT).to_string()
}

/// A new game's folder name under `root`: the time now, a second later per
/// name already there (a clock set back can land on an old folder).
pub(crate) fn unique_pregame_folder(root: &std::path::Path, at: chrono::DateTime<Utc>) -> String {
    (0..60)
        .map(|s| pregame_folder_name(at + chrono::Duration::seconds(s)))
        .find(|name| std::fs::symlink_metadata(root.join(name)).is_err())
        .unwrap_or_else(|| pregame_folder_name(at))
}

/// Exactly a name [`pregame_folder_name`] makes: a real date and time in
/// `YYYYMMDD_HHMMSS`, nothing more. Only these are pruned; anything else in
/// `debug/pregame/` is left alone.
pub(crate) fn is_game_folder(name: &str) -> bool {
    name.len() == 15
        && chrono::NaiveDateTime::parse_from_str(name, FOLDER_FORMAT)
            .is_ok_and(|t| t.format(FOLDER_FORMAT).to_string() == name)
}

/// Game folders in the order they were made, oldest first, one per line.
/// Kept across runs so a clock set back (which gives a new folder an old
/// name) does not make the next run prune its newest games first.
pub(super) const ORDER_FILE: &str = ".order";

/// The folders [`ORDER_FILE`] lists, oldest first, each once (a repeat
/// keeps its first place); empty when it is missing.
pub(crate) fn load_pregame_order(root: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join(ORDER_FILE)).unwrap_or_default();
    let mut made: Vec<String> = Vec::new();
    for name in text.lines().filter(|name| is_game_folder(name)) {
        if !made.iter().any(|m| m == name) {
            made.push(name.to_string());
        }
    }
    made
}

/// Write [`ORDER_FILE`] through a temporary file and a rename.
pub(super) fn save_pregame_order(root: &std::path::Path, made: &[String]) {
    let tmp = root.join(format!("{ORDER_FILE}.tmp"));
    let text: String = made.iter().map(|name| format!("{name}\n")).collect();
    let saved =
        std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, root.join(ORDER_FILE)));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder order not saved");
    }
}

/// Delete the oldest game folders under `root` so at most `keep` are left.
/// Only real directories with an exact game-folder name count: no file, no
/// symlink, never `root` itself. Age order is name order (UTC timestamps)
/// for folders not in `made`, then `made` (the folders in the order they
/// were made, oldest first) after them all: a clock set back gives a new
/// folder an old name, and it still goes last. The newest of `made` is
/// never deleted. A folder with only staged or temporary files in it
/// does not count. A folder whose only saved screens are turned-down
/// candidates (`rejected_<label>.png`) takes no slot either, so it never
/// pushes a real game out: those folders have their own cap,
/// [`PREGAME_KEEP_REJECTED_ONLY`], oldest deleted first.
pub(crate) fn prune_pregame_games(root: &std::path::Path, keep: usize, made: &[String]) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let games: Vec<String> = entries
        .flatten()
        // `DirEntry::file_type` does not follow symlinks.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_game_folder(name))
        .collect();
    // The game in progress: never deleted, even with nothing saved yet.
    let newest = made.iter().rev().find(|name| games.contains(name));
    let mut real = Vec::new();
    let mut rejected_only = Vec::new();
    for name in games {
        match folder_kind(&root.join(&name)) {
            // A folder holding only staged or temporary files is no saved
            // game: it takes no slot and pushes no game out
            // ([`sweep_staged`] deletes it).
            FolderKind::Leftovers => {}
            FolderKind::RejectedOnly => rejected_only.push(name),
            FolderKind::Game => real.push(name),
        }
    }
    prune_oldest(root, real, keep, made, newest);
    prune_oldest(
        root,
        rejected_only,
        PREGAME_KEEP_REJECTED_ONLY,
        made,
        newest,
    );
}

/// Delete the oldest of `folders` under `root` so at most `keep` are left,
/// in the age order of [`prune_pregame_games`], never `newest`.
pub(super) fn prune_oldest(
    root: &std::path::Path,
    mut older: Vec<String>,
    keep: usize,
    made: &[String],
    newest: Option<&String>,
) {
    // Each folder once, even if `made` repeats it.
    let mut ours: Vec<String> = Vec::new();
    for name in made {
        if older.contains(name) && !ours.contains(name) {
            ours.push(name.clone());
        }
    }
    older.retain(|name| !made.contains(name));
    older.sort();
    let total = older.len() + ours.len();
    if total <= keep {
        return;
    }
    for name in older
        .iter()
        .chain(ours.iter())
        .filter(|name| Some(*name) != newest)
        .take(total - keep)
    {
        if let Err(e) = std::fs::remove_dir_all(root.join(name)) {
            tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not deleted");
        }
    }
}

/// A staged frame ([`staged_name`]) or a temporary file of a write (a
/// dot name ending `.tmp`): never a saved screen.
pub(super) fn is_leftover(name: &str) -> bool {
    name.starts_with('.') && (name.ends_with(".staged") || name.ends_with(".tmp"))
}

/// A turned-down candidate's frame, `rejected_<label>.png` ([`rejects`]).
pub(super) fn is_rejected(name: &str) -> bool {
    name.starts_with("rejected_") && name.ends_with(".png")
}

/// What a game folder holds, for the retention cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FolderKind {
    /// Files, all of them leftovers ([`is_leftover`]).
    Leftovers,
    /// Files, at least one `rejected_<label>.png` ([`is_rejected`]) and
    /// nothing but those and leftovers.
    RejectedOnly,
    /// Anything else: a saved screen, a subfolder, an empty or unreadable
    /// folder.
    Game,
}

pub(super) fn folder_kind(dir: &std::path::Path) -> FolderKind {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return FolderKind::Game;
    };
    let (mut leftovers, mut rejected) = (false, false);
    for entry in entries {
        let Ok(entry) = entry else {
            return FolderKind::Game;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            return FolderKind::Game;
        }
        match entry.file_name().to_str() {
            Some(name) if is_leftover(name) => leftovers = true,
            Some(name) if is_rejected(name) => rejected = true,
            _ => return FolderKind::Game,
        }
    }
    match (rejected, leftovers) {
        (true, _) => FolderKind::RejectedOnly,
        (false, true) => FolderKind::Leftovers,
        (false, false) => FolderKind::Game,
    }
}

/// Delete what a write left half done under `root`: staged frames
/// ([`staged_name`]) and temporary files, in `root` itself and in every
/// game folder but `except`, and then each of those folders it left
/// empty. Run at start and when a new game folder is made, since a staged
/// frame belongs to the game in progress only. A saved screen is never
/// touched, and a folder with one in it is never deleted.
pub(crate) fn sweep_staged(root: &std::path::Path, except: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_file() && is_leftover(&name) {
            // The order file's temporary copy.
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        // The pending slot's staged roster belongs to no game folder yet:
        // only the sweep at start (no game in progress) deletes it.
        let pending = name == PENDING_DIR && except.is_none();
        if !kind.is_dir() || !(pending || is_game_folder(&name)) || except == Some(name.as_str()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        let mut swept = false;
        for file in files.flatten() {
            let leftover = file.file_name().to_str().is_some_and(is_leftover);
            if leftover && file.file_type().is_ok_and(|t| t.is_file()) {
                swept |= std::fs::remove_file(file.path()).is_ok();
            }
        }
        if swept {
            // Only an empty folder goes: `remove_dir` fails on any other.
            let _ = std::fs::remove_dir(entry.path());
        }
    }
}

/// Write `frame` under `root` at full size as `<folder>/<file>`, to a
/// temporary file then renamed, so a crash leaves no half PNG under the
/// final name. After a new game folder (added to `made`), keep the newest
/// `keep`, save the order and sweep staged frames left in the others.
pub(super) fn write_frame(
    root: &std::path::Path,
    folder: &str,
    file: &str,
    frame: &image::DynamicImage,
    keep: usize,
    made: &mut Vec<String>,
) {
    let dir = root.join(folder);
    let new_folder = !dir.is_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game folder not made");
        return;
    }
    let tmp = dir.join(format!(".{file}.tmp"));
    let saved = frame
        .save_with_format(&tmp, image::ImageFormat::Png)
        .map_err(|e| e.to_string())
        .and_then(|()| std::fs::rename(&tmp, dir.join(file)).map_err(|e| e.to_string()));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game frame not saved");
    }
    // The pending slot's folder is no game folder.
    if new_folder && folder != PENDING_DIR {
        note_new_folder(root, folder, keep, made);
    }
}

/// A new game folder was made (by a pre-game or a post-game write): add it
/// to `made` as the newest, keep the newest `keep`, save the order and
/// sweep staged frames left in the others.
pub(super) fn note_new_folder(
    root: &std::path::Path,
    folder: &str,
    keep: usize,
    made: &mut Vec<String>,
) {
    made.retain(|name| name != folder);
    made.push(folder.to_string());
    prune_pregame_games(root, keep, made);
    made.retain(|name| root.join(name).is_dir());
    save_pregame_order(root, made);
    sweep_staged(root, Some(folder));
}

/// The post-game sidecar to write: `<root>/<folder>/<name>`, replaced
/// whole each time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NoteFile {
    pub(crate) folder: String,
    pub(crate) name: &'static str,
    pub(crate) doc: super::postgame_dump::PostSidecar,
}

/// Write `note` the way [`write_frame`] writes a PNG: a temporary file then
/// a rename, and a new game folder counts for the retention. It lists only
/// the frames on disk: those whose write failed are left out.
pub(crate) fn write_note(
    root: &std::path::Path,
    note: &NoteFile,
    keep: usize,
    made: &mut Vec<String>,
) {
    let dir = root.join(&note.folder);
    let new_folder = !dir.is_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "post-game folder not made");
        return;
    }
    let tmp = dir.join(format!(".{}.tmp", note.name));
    let text = note.doc.render(|file| dir.join(file).is_file());
    let saved =
        std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, dir.join(note.name)));
    if let Err(e) = saved {
        let _ = std::fs::remove_file(&tmp);
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "post-game sidecar not saved");
    }
    if new_folder {
        note_new_folder(root, &note.folder, keep, made);
    }
}

/// Carry out `ops` under `root`, in order: saves and stages through
/// [`write_frame`], a promote as one rename of the staged frame, a discard
/// as its delete. A stage first deletes the staged frame it replaces, so a
/// stage whose write fails leaves none, and a promote or discard with no
/// staged frame changes nothing: an older frame is never saved in place of
/// a newer one that failed to write.
pub(crate) fn write_pregame_ops(
    root: &std::path::Path,
    ops: &[PregameOp<Arc<image::DynamicImage>>],
    keep: usize,
    made: &mut Vec<String>,
) {
    for op in ops {
        match op {
            PregameOp::Save(shot) => {
                write_frame(root, &shot.folder, shot.name, &shot.frame, keep, made)
            }
            PregameOp::Stage(shot) => {
                let staged = staged_name(shot.name);
                let _ = std::fs::remove_file(root.join(&shot.folder).join(&staged));
                write_frame(root, &shot.folder, &staged, &shot.frame, keep, made)
            }
            PregameOp::Promote { folder, name } => {
                let dir = root.join(folder);
                if let Err(e) = std::fs::rename(dir.join(staged_name(name)), dir.join(name)) {
                    tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game staged frame not saved");
                }
            }
            PregameOp::Discard { folder, name } => {
                let _ = std::fs::remove_file(root.join(folder).join(staged_name(name)));
            }
            PregameOp::Adopt { folder, name } => adopt_pending(root, folder, name, keep, made),
        }
    }
}

/// [`PregameOp::Adopt`]: move the staged `name` in [`PENDING_DIR`] into
/// `folder` as `name`; a new folder counts for the retention. With nothing
/// staged (its stage dropped or failed) no folder is made.
pub(super) fn adopt_pending(
    root: &std::path::Path,
    folder: &str,
    name: &str,
    keep: usize,
    made: &mut Vec<String>,
) {
    let from = root.join(PENDING_DIR).join(staged_name(name));
    if !from.is_file() {
        tracing::debug!(target: "scuffed_stat_tracker", "pre-game pending frame not on disk");
        return;
    }
    let dir = root.join(folder);
    let new_folder = !dir.is_dir();
    let moved = std::fs::create_dir_all(&dir).and_then(|()| std::fs::rename(&from, dir.join(name)));
    if let Err(e) = moved {
        tracing::debug!(target: "scuffed_stat_tracker", error = %e, "pre-game pending frame not saved");
    }
    if new_folder && dir.is_dir() {
        note_new_folder(root, folder, keep, made);
    }
}
