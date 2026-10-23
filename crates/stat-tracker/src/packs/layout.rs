//! The on-disk layout of the reader packs: which packs the tracker installs
//! ([`PACKS`]), the folder each one lands in, what may sit inside one
//! ([`PackLayout`]), the version and manifest records sync writes after an
//! install, the directory swap, and the file-name checks the readers share.
//! A leaf: the readers depend on this, not on the downloader in
//! `packs/mod.rs`, which re-exports these at their old `packs::` paths.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
/// result words) and names-v1 (the name reader's letter pack). bans-v1 is
/// not opened by a reader yet.
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
    let Ok(bytes) = std::fs::read(versions_path(data_dir)) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// `templates/pack-manifests.json`: pack name to the sha256 of the inner
/// manifest that install checked every file against, for the layouts that
/// keep their manifest ([`PackLayout::keeps_manifest`]). Written under the
/// sync lock right after the versions file, by the daemon and the app alike.
pub(super) fn manifests_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join("pack-manifests.json")
}

pub(super) fn read_manifests(data_dir: &Path) -> BTreeMap<String, String> {
    let Ok(bytes) = std::fs::read(manifests_path(data_dir)) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
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
