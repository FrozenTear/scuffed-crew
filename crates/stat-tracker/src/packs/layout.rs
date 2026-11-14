//! The on-disk layout of the reader packs: which packs the tracker installs
//! ([`PACKS`]), the folder each one lands in, what may sit inside one
//! ([`PackLayout`]), the version and manifest records sync writes after an
//! install, the directory swap, the keep of a `templates.json` folder sync
//! did not install as it is ([`needs_keep`], [`swap_keeping`],
//! [`KEPT_DIR`]), and the file-name checks the readers share.
//! A leaf: the readers depend on this, not on the downloader in
//! `packs/mod.rs`, which re-exports these at their old `packs::` paths.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub mod vision_sizes;

/// Template directory under the data dir (heroes-v1).
pub const TEMPLATE_SUBDIR: &str = "templates/heroes";
/// heroes-v2 pack directory under the data dir (1440p-scale cuts).
pub const TEMPLATE_SUBDIR_V2: &str = "templates/heroes-v2";
/// heroes-v2-1080 pack directory under the data dir (1080p-scale cuts).
pub const TEMPLATE_SUBDIR_V2_1080: &str = "templates/heroes-v2-1080";

/// File name of the names-v1 letter pack on the pack list
/// (`names::pack::PACK_NAME`).
pub const NAMES_PACK_NAME: &str = "names-v1.tar";
/// Folder of the names-v1 pack under `templates/`
/// (`names::pack::PACK_SUBDIR`).
pub const NAMES_PACK_SUBDIR: &str = "names-v1";

/// Where the pack is installed under the tracker's data dir.
pub fn names_dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join(NAMES_PACK_SUBDIR)
}

/// File name of the bans-picks-v1 pack on the pack list: the 100 by 96 hero
/// icons the cv-v1 bans and picks reader matches.
pub const BANS_PICKS_PACK_NAME: &str = "bans-picks-v1.tar";

/// Folder of the bans-picks-v1 pack under the data dir
/// (`shadow::bans_picks::TEMPLATE_DIR`).
pub const BANS_PICKS_PACK_DIR: &str = "templates/bans-picks-v1";

/// File name of the rank-v1 pack on the pack list: the glyph and word
/// templates of the rank update reader, one `templates.json`.
pub const RANK_PACK_NAME: &str = "rank-v1.tar";

/// Folder of the rank-v1 pack under the data dir
/// (`shadow::rank::TEMPLATE_DIR`).
pub const RANK_PACK_DIR: &str = "templates/rank-v1";

/// File name of the queue-title-v1 pack on the pack list: the title
/// templates of the queue title reader, one `templates.json`.
pub const QUEUE_TITLE_PACK_NAME: &str = "queue-title-v1.tar";

/// Folder of the queue-title-v1 pack under the data dir
/// (`shadow::queue_title::TEMPLATE_DIR`).
pub const QUEUE_TITLE_PACK_DIR: &str = "templates/queue-title-v1";

/// File name of the hero template pack on the pack list.
pub const HEROES_PACK_NAME: &str = "heroes-v1.tar";

/// File name of the hero portrait pack the new reader reads at 1440p. To
/// move the tracker to a newer build (say `heroes-v3.tar`), change this
/// name, [`HEROES_PORTRAIT_1080_PACK_NAME`], and the folders
/// [`TEMPLATE_SUBDIR_V2`] and [`TEMPLATE_SUBDIR_V2_1080`].
pub const HEROES_PORTRAIT_PACK_NAME: &str = "heroes-v2.tar";

/// File name of the hero portrait pack the new reader reads at 1080p.
pub const HEROES_PORTRAIT_1080_PACK_NAME: &str = "heroes-v2-1080.tar";

/// What may sit inside one pack. Paths are checked before anything is
/// written, and a path outside the layout refuses that pack only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackLayout {
    /// `heroes-v1.tar`, unchanged from alpha.6: one top-level file, or one
    /// file in `special/`.
    HeroesV1,
    /// Hero portraits: top-level `<hero-key>.png` or `special/<name>.png`.
    Portraits,
    /// Ban icons: top-level `<hero-key>.png` only.
    BanIcons,
    /// Map-name glyphs: `atlas.json` and `g<4 hex digits>.png`.
    MapGlyphs,
    /// Word art: top-level `<word_id>.png` and `<name>.json`.
    Words,
    /// Letter templates: the six files [`crate::names::pack`] loads, and
    /// nothing else. No subdirectory.
    Names,
    /// One `templates.json` and nothing else (rank-v1, queue-title-v1).
    TemplatesJson,
}

/// One pack the tracker installs.
#[derive(Debug, Clone, Copy)]
pub struct PackSpec {
    /// Exact `name` on the server list and in the inner manifest.
    pub name: &'static str,
    /// Folder under the data dir this pack is unpacked into.
    pub dir: fn(&Path) -> PathBuf,
    pub layout: PackLayout,
    /// When the server lists packs but not this one, report a mismatch
    /// instead of `PackSync::Missing`. Only `heroes-v1.tar`, as in alpha.6.
    pub required: bool,
}

impl PackSpec {
    pub fn dir_in(&self, data_dir: &Path) -> PathBuf {
        (self.dir)(data_dir)
    }
}

/// Every pack the tracker syncs, in download order. `heroes-v1.tar` goes
/// first so a rate limit or refused token hits it the way it did in alpha.6.
/// The hero packs are read by a reader today (heroes-v1 by the shadow log and
/// as the new reader's fallback, heroes-v2 and heroes-v2-1080 by the new
/// reader), and so are maps-v1 (the new reader's map banner), words-v1 (its
/// result words), names-v1 (the name reader's letter pack),
/// bans-picks-v1 (the cv-v1 bans and picks reader), rank-v1 (the rank
/// update reader) and queue-title-v1 (the queue title reader). bans-v1 is
/// not opened by a reader yet. rank-v1 and queue-title-v1 are read once,
/// when the daemon starts, so a pack that lands later is used from the
/// next start.
pub const PACKS: &[PackSpec] = &[
    PackSpec {
        name: HEROES_PACK_NAME,
        dir: |data_dir| data_dir.join(TEMPLATE_SUBDIR),
        layout: PackLayout::HeroesV1,
        required: true,
    },
    PackSpec {
        name: HEROES_PORTRAIT_PACK_NAME,
        dir: |data_dir| data_dir.join(TEMPLATE_SUBDIR_V2),
        layout: PackLayout::Portraits,
        required: false,
    },
    PackSpec {
        name: HEROES_PORTRAIT_1080_PACK_NAME,
        dir: |data_dir| data_dir.join(TEMPLATE_SUBDIR_V2_1080),
        layout: PackLayout::Portraits,
        required: false,
    },
    PackSpec {
        name: "bans-v1.tar",
        dir: |data_dir| data_dir.join("templates").join("bans-v1"),
        layout: PackLayout::BanIcons,
        required: false,
    },
    PackSpec {
        name: "maps-v1.tar",
        dir: |data_dir| data_dir.join("templates").join("maps-v1"),
        layout: PackLayout::MapGlyphs,
        required: false,
    },
    PackSpec {
        name: "words-v1.tar",
        dir: |data_dir| data_dir.join("templates").join("words-v1"),
        layout: PackLayout::Words,
        required: false,
    },
    PackSpec {
        name: NAMES_PACK_NAME,
        dir: names_dir_in,
        layout: PackLayout::Names,
        required: false,
    },
    PackSpec {
        name: BANS_PICKS_PACK_NAME,
        dir: |data_dir| data_dir.join(BANS_PICKS_PACK_DIR),
        layout: PackLayout::BanIcons,
        required: false,
    },
    PackSpec {
        name: RANK_PACK_NAME,
        dir: |data_dir| data_dir.join(RANK_PACK_DIR),
        layout: PackLayout::TemplatesJson,
        required: false,
    },
    PackSpec {
        name: QUEUE_TITLE_PACK_NAME,
        dir: |data_dir| data_dir.join(QUEUE_TITLE_PACK_DIR),
        layout: PackLayout::TemplatesJson,
        required: false,
    },
];

/// The spec for `name`, if the tracker installs that pack.
pub fn pack_spec(name: &str) -> Option<&'static PackSpec> {
    PACKS.iter().find(|spec| spec.name == name)
}

/// Last path part of a pack folder, for staging and backup names.
pub(super) fn dir_label(live: &Path) -> String {
    live.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "pack".to_string())
}

pub(super) fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    )
}

pub(crate) fn swap_dir(staging: &Path, live: &Path) -> Result<(), String> {
    let _ = sync_dir(staging);
    if live.exists() {
        match exchange_paths(staging, live) {
            Ok(()) => {
                if let Err(err) = std::fs::remove_dir_all(staging) {
                    tracing::warn!(
                        error = %err,
                        "previous reader pack was left beside the new one"
                    );
                }
                let _ = sync_dir(live.parent().unwrap_or(live));
                return Ok(());
            }
            Err(err) => {
                tracing::warn!(error = %err, "atomic pack exchange was not available");
            }
        }
    } else {
        std::fs::rename(staging, live).map_err(|err| err.to_string())?;
        let _ = sync_dir(live.parent().unwrap_or(live));
        return Ok(());
    }

    let parent = live.parent().unwrap_or_else(|| Path::new("."));
    let backup = parent.join(format!(".{}-prev-{}", dir_label(live), unique_suffix()));
    std::fs::rename(live, &backup).map_err(|err| err.to_string())?;
    if let Err(err) = std::fs::rename(staging, live) {
        let _ = std::fs::rename(&backup, live);
        return Err(err.to_string());
    }
    let _ = std::fs::remove_dir_all(&backup);
    let _ = sync_dir(parent);
    Ok(())
}

/// Folder under the data dir that a [`PackLayout::TemplatesJson`] folder
/// sync did not install as it is now (placed by hand, or edited since) is
/// moved into when the site's pack replaces it ([`needs_keep`]). A sibling
/// of `templates/`, so the stale sweep, which only walks `templates/`,
/// never removes it. Nothing removes it: not sync, not the app, not
/// `uninstall.sh --purge`. The member deletes it by hand. Each kept folder
/// is named `<pack folder>-<UTC YYYYMMDDTHHMMSSZ>-<pid>`, for example
/// `rank-v1-20261114T065156Z-4242`, with `-2`, `-3`... added when that
/// name is taken ([`kept_path`]).
pub const KEPT_DIR: &str = "templates-kept";

/// The one file a [`PackLayout::TemplatesJson`] pack installs.
pub(super) const TEMPLATES_JSON: &str = "templates.json";

/// `templates/pack-file-hashes.json`: pack name to the sha256 of the
/// `templates.json` sync installed, for [`PackLayout::TemplatesJson`]
/// packs only. Written under the sync lock right after the swap.
pub(super) fn file_hashes_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join("pack-file-hashes.json")
}

/// True when installing `spec` over `live` must keep the folder there
/// first: a [`PackLayout::TemplatesJson`] folder that is not exactly one
/// regular file named `templates.json` whose sha256 is the one recorded
/// in [`file_hashes_path`]. A missing record never matches, so a folder
/// placed by hand, or installed by a tracker before this record, is kept
/// once, and so is an official folder edited or added to by hand since.
/// Other layouts never keep. Blocking (one small read and one hash).
pub(super) fn needs_keep(data_dir: &Path, spec: &PackSpec, live: &Path) -> bool {
    if spec.layout != PackLayout::TemplatesJson || live.symlink_metadata().is_err() {
        return false;
    }
    let Some(recorded) = read_map(&file_hashes_path(data_dir)).remove(spec.name) else {
        return true;
    };
    let file = live.join(TEMPLATES_JSON);
    let only_file = live.symlink_metadata().is_ok_and(|meta| meta.is_dir())
        && std::fs::read_dir(live).is_ok_and(|entries| {
            let names: Vec<String> = entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names == [TEMPLATES_JSON]
        })
        && file.symlink_metadata().is_ok_and(|meta| meta.is_file());
    !(only_file && std::fs::read(&file).is_ok_and(|bytes| sha256_hex(&bytes) == recorded))
}

/// Record the sha256 of the `templates.json` just swapped into `live`. A
/// record that is not written is logged only: the next install then keeps
/// that folder once, so nothing is lost.
pub(super) fn record_templates_hash(data_dir: &Path, name: &str, live: &Path) {
    let path = file_hashes_path(data_dir);
    let mut hashes = read_map(&path);
    let written = std::fs::read(live.join(TEMPLATES_JSON))
        .map_err(|err| err.to_string())
        .and_then(|bytes| {
            hashes.insert(name.to_string(), sha256_hex(&bytes));
            write_map(&path, HASHES_TMP_PREFIX, &hashes)
        });
    if let Err(message) = written {
        tracing::warn!(pack = name, error = %message, "reader pack file hash was not stored");
    }
}

/// Why [`swap_keeping`] did not install the pack.
#[derive(Debug)]
pub(super) enum KeepSwap {
    /// The old tree could not be kept. It is the live folder again and
    /// `staging` holds only the new pack.
    NotKept(String),
    /// The swap failed after the keep. The old folder was moved back, or,
    /// if that failed too, a warning names the path it is kept at.
    NotSwapped(String),
    /// After the exchange the old tree could be neither kept nor swapped
    /// back: it is still at `staging`, so `staging` must not be removed.
    /// The live folder holds the new pack.
    Stranded(String),
}

impl KeepSwap {
    /// Remove `staging` after a failed swap, unless it still holds the old
    /// tree ([`KeepSwap::Stranded`]).
    pub(super) fn remove_staging(&self, staging: &Path) {
        if !matches!(self, Self::Stranded(_)) {
            let _ = std::fs::remove_dir_all(staging);
        }
    }
}

/// [`swap_dir`] for a folder [`needs_keep`]: the old tree is moved to a new
/// folder under `<data_dir>/templates-kept/` instead of being deleted, and
/// that path is logged as soon as the move is done. With `RENAME_EXCHANGE`
/// the new pack is swapped in first, so the live folder is never missing,
/// and then the old tree is moved out of the staging path. Should that
/// move fail, the old tree goes beside the live folder under the same
/// name (same disk, a name the stale sweep skips). Should that fail too,
/// the two folders are exchanged back and the pack is not installed
/// ([`KeepSwap::NotKept`]); if even that fails, the old tree stays at the
/// staging path, which the stale sweep may remove, and a warning names it
/// ([`KeepSwap::Stranded`]). Without the exchange, the old folder is moved
/// to `templates-kept/` first, then the new one into place, and back if
/// that fails. Returns where the old tree is kept.
pub(super) fn swap_keeping(
    data_dir: &Path,
    pack: &str,
    staging: &Path,
    live: &Path,
) -> Result<PathBuf, KeepSwap> {
    let root = data_dir.join(KEPT_DIR);
    std::fs::create_dir_all(&root).map_err(|err| KeepSwap::NotKept(err.to_string()))?;
    let kept = kept_path(&root, live);
    let parent = live.parent().unwrap_or_else(|| Path::new("."));
    let log_kept = |at: &Path| {
        tracing::warn!(
            pack,
            kept = %at.display(),
            "reader pack folder that was not the one sync installed was moved aside; the old folder is kept at this path"
        );
    };
    let _ = sync_dir(staging);
    match exchange_paths(staging, live) {
        Ok(()) => {
            // `staging` now holds the old tree.
            let err = match std::fs::rename(staging, &kept) {
                Ok(()) => {
                    log_kept(&kept);
                    let _ = sync_dir(&root);
                    let _ = sync_dir(parent);
                    return Ok(kept);
                }
                Err(err) => err,
            };
            tracing::warn!(pack, error = %err, "reader pack folder was not moved to templates-kept");
            let beside = parent.join(kept.file_name().unwrap_or_default());
            match std::fs::rename(staging, &beside) {
                Ok(()) => {
                    log_kept(&beside);
                    let _ = sync_dir(parent);
                    return Ok(beside);
                }
                Err(beside_err) => {
                    tracing::warn!(pack, error = %beside_err, "reader pack folder was not moved beside the live folder");
                }
            }
            if let Err(back) = exchange_paths(staging, live) {
                tracing::warn!(
                    pack,
                    at = %staging.display(),
                    error = %back,
                    "reader pack folder was not put back; the old folder is still at this staging path, which a later sync may remove as stale"
                );
                return Err(KeepSwap::Stranded(err.to_string()));
            }
            tracing::warn!(pack, live = %live.display(), "reader pack folder was put back in place");
            let _ = sync_dir(parent);
            return Err(KeepSwap::NotKept(err.to_string()));
        }
        Err(err) => {
            tracing::warn!(pack, error = %err, "reader pack folders were not exchanged; the old folder is moved to templates-kept first");
        }
    }
    std::fs::rename(live, &kept).map_err(|err| KeepSwap::NotKept(err.to_string()))?;
    log_kept(&kept);
    let _ = sync_dir(&root);
    if let Err(err) = std::fs::rename(staging, live) {
        match std::fs::rename(&kept, live) {
            Ok(()) => {
                tracing::warn!(pack, live = %live.display(), "reader pack folder was put back in place")
            }
            Err(back) => tracing::warn!(
                pack,
                kept = %kept.display(),
                error = %back,
                "reader pack folder was not put back; it is kept at this path"
            ),
        }
        return Err(KeepSwap::NotSwapped(err.to_string()));
    }
    let _ = sync_dir(parent);
    Ok(kept)
}

/// A new path under `root` for the folder at `live`:
/// `<label>-<UTC YYYYMMDDTHHMMSSZ>-<pid>`, with `-2`, `-3`... added when
/// that name is taken.
fn kept_path(root: &Path, live: &Path) -> PathBuf {
    let base = format!(
        "{}-{}-{}",
        dir_label(live),
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    );
    (1..)
        .map(|n| match n {
            1 => root.join(&base),
            n => root.join(format!("{base}-{n}")),
        })
        .find(|path| path.symlink_metadata().is_err())
        .unwrap_or_else(|| root.join(base))
}

/// Daemon start: when `templates-kept/` holds folders, or a folder was
/// kept beside a live pack folder ([`is_kept_beside`]), log the
/// `templates-kept/` path once per daemon start with both counts, so a
/// member can find what sync moved aside. Silent when there are none.
pub fn log_kept_folders(data_dir: &Path) {
    fn count(dir: &Path, keep: impl Fn(&str) -> bool) -> usize {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .filter(|entry| keep(&entry.file_name().to_string_lossy()))
                    .count()
            })
            .unwrap_or(0)
    }
    let root = data_dir.join(KEPT_DIR);
    let folders = count(&root, |_| true);
    let beside = PACKS
        .iter()
        .filter(|spec| spec.layout == PackLayout::TemplatesJson)
        .map(|spec| {
            let live = spec.dir_in(data_dir);
            let parent = live.parent().unwrap_or_else(|| Path::new("."));
            count(parent, |name| is_kept_beside(&dir_label(&live), name))
        })
        .sum::<usize>();
    if folders + beside > 0 {
        tracing::info!(
            path = %root.display(),
            folders,
            beside,
            "reader pack folders that sync moved aside are kept here, and beside the live pack folders under templates/ when that count is not 0; nothing removes them"
        );
    }
}

/// A folder name [`swap_keeping`] gives an old tree kept beside the live
/// folder `label`: `<label>-<UTC YYYYMMDDTHHMMSSZ>-<pid>`, maybe with
/// `-2`, `-3`... after it.
fn is_kept_beside(label: &str, name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix(label)
        .and_then(|rest| rest.strip_prefix('-'))
    else {
        return false;
    };
    let mut parts = rest.split('-');
    let stamp_ok = parts.next().is_some_and(|stamp| {
        stamp.len() == 16
            && stamp.char_indices().all(|(at, c)| match at {
                8 => c == 'T',
                15 => c == 'Z',
                _ => c.is_ascii_digit(),
            })
    });
    let tail: Vec<&str> = parts.collect();
    stamp_ok
        && (1..=2).contains(&tail.len())
        && tail
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// A `.<pack>-next-*` or `.<pack>-prev-*` folder this old is left from a
/// sync that was killed. Live syncs hold the lock, so none of theirs is here.
pub(super) const STALE_STAGING: std::time::Duration = std::time::Duration::from_secs(3600);

/// Remove staging and backup folders left by a killed sync. Runs under the
/// sync lock. Only names this module makes, only real folders (not links),
/// and only when older than [`STALE_STAGING`]. Also removes record tmp
/// files left by a kill. Never `templates-kept/` (outside `templates/`) nor
/// a kept folder beside a live one (no leading dot).
pub(super) fn sweep_stale_staging(data_dir: &Path) {
    let templates = data_dir.join("templates");
    let Ok(entries) = std::fs::read_dir(&templates) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if is_record_tmp(&name) {
            // Only a run holding this lock writes one, so this one is left.
            if path.symlink_metadata().is_ok_and(|meta| meta.is_file())
                && let Err(err) = std::fs::remove_file(&path)
            {
                tracing::warn!(file = %name, error = %err, "left reader pack versions file was not removed");
            }
            continue;
        }
        // The pack this name belongs to, and whether it is a `-prev-` backup.
        let owner = name.strip_prefix('.').and_then(|rest| {
            PACKS.iter().find_map(|spec| {
                let tail = rest.strip_prefix(dir_label(&spec.dir_in(data_dir)).as_str())?;
                if tail.starts_with("-next-") {
                    Some((spec, false))
                } else if tail.starts_with("-prev-") {
                    Some((spec, true))
                } else {
                    None
                }
            })
        });
        let Some((spec, backup)) = owner else {
            continue;
        };
        // A `-prev-` folder is the old live folder renamed, so its mtime is
        // the old pack's. While the live folder is missing (a kill inside
        // the fallback swap) it is the only copy of that pack: keep it.
        if backup && !spec.dir_in(data_dir).exists() {
            continue;
        }
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        let stale = meta
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_STAGING);
        if meta.is_dir() && stale {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    tracing::info!(folder = %name, "removed a reader pack folder left by a stopped sync")
                }
                Err(err) => {
                    tracing::warn!(folder = %name, error = %err, "left reader pack folder was not removed")
                }
            }
        }
    }
}

/// Start of the tmp files the record writers make. Only a run holding the
/// sync lock writes one, so any found at the start of a run was left by a
/// kill ([`is_record_tmp`]).
pub(super) const VERSIONS_TMP_PREFIX: &str = "pack-versions.json.tmp-";
pub(super) const MANIFESTS_TMP_PREFIX: &str = "pack-manifests.json.tmp-";
const HASHES_TMP_PREFIX: &str = "pack-file-hashes.json.tmp-";

/// A tmp file a record writer left under `templates/`.
pub(super) fn is_record_tmp(name: &str) -> bool {
    [VERSIONS_TMP_PREFIX, MANIFESTS_TMP_PREFIX, HASHES_TMP_PREFIX]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Write `map` as JSON to `path` through a tmp file named `<prefix><n>`.
pub(super) fn write_map(
    path: &Path,
    prefix: &str,
    map: &BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    // A name of its own, so two writers never share one tmp file.
    let tmp = path.with_file_name(format!("{prefix}{}", unique_suffix()));
    let bytes = serde_json::to_vec(map).map_err(|err| err.to_string())?;
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.to_string());
    }
    Ok(())
}

fn read_map(path: &Path) -> BTreeMap<String, String> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sync_dir(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

/// Swap two directory entries in one syscall when the kernel allows it.
/// After this returns, `live` is the new tree and `staging` is the old one.
fn exchange_paths(staging: &Path, live: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let staging_c = std::ffi::CString::new(staging.as_os_str().as_bytes())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let live_c = std::ffi::CString::new(live.as_os_str().as_bytes())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            staging_c.as_ptr(),
            libc::AT_FDCWD,
            live_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub(super) fn versions_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join("pack-versions.json")
}

pub(super) fn read_versions(data_dir: &Path) -> BTreeMap<String, String> {
    read_map(&versions_path(data_dir))
}

/// `templates/pack-manifests.json`: pack name to the sha256 of the inner
/// manifest that install checked every file against, for the layouts that
/// keep their manifest ([`PackLayout::keeps_manifest`]). Written under the
/// sync lock right after the versions file, by the daemon and the app alike.
pub(super) fn manifests_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join("pack-manifests.json")
}

pub(super) fn read_manifests(data_dir: &Path) -> BTreeMap<String, String> {
    read_map(&manifests_path(data_dir))
}

/// What sync last installed for `name`: the version and the manifest
/// sha256, when both are recorded. `None` for a pack installed by hand or
/// by a tracker older than this record. Blocking (two small reads).
pub fn installed_manifest(data_dir: &Path, name: &str) -> Option<(String, String)> {
    let (version, sha) = installed_record(data_dir, name)?;
    Some((version, sha?))
}

/// The stored version of `name`, and the manifest sha256 when one is
/// recorded (a tracker before 0.5.0-rc.2 stored the version only). `None`
/// when no version is stored. Blocking (two small reads).
pub fn installed_record(data_dir: &Path, name: &str) -> Option<(String, Option<String>)> {
    let version = read_versions(data_dir).remove(name)?;
    let sha = read_manifests(data_dir)
        .remove(name)
        .filter(|sha| is_sha256(sha));
    Some((version, sha))
}

/// The two record files sync writes after every install, last of all.
/// A reader watching for another process's install waits for them.
pub fn record_paths(data_dir: &Path) -> [PathBuf; 2] {
    [versions_path(data_dir), manifests_path(data_dir)]
}

pub(super) fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

/// Longest file stem the strict layouts accept.
pub(super) const MAX_STEM: usize = 64;

/// Word ids: lowercase, digits and `_`.
pub(super) fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
}

/// A word id the words-v1 layout allows as a file stem: 1 to 64 of `a-z`,
/// `0-9`, `_`. No separator, no `..`, never absolute. The result reader
/// checks layout ids from a pack's `layouts.json` with this before it joins
/// them into a path.
pub(crate) fn is_word_id(stem: &str) -> bool {
    !stem.is_empty() && stem.len() <= MAX_STEM && stem.bytes().all(is_word_byte)
}

/// `g` plus four lowercase hex digits, then `.png`.
pub(crate) fn is_glyph_name(raw: &str) -> bool {
    raw.strip_prefix('g')
        .and_then(|rest| rest.strip_suffix(".png"))
        .is_some_and(|hex| {
            hex.len() == 4
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
}
