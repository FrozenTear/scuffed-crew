//! Private recognizer asset packs (`PACKS_DIR`).
//!
//! The directory is operator-owned and read-only to the server. It is not the
//! upload tree, not the bug-report tree, and not the web root. Game art stays
//! out of git. A missing or unreadable directory leaves the process up; the
//! pack routes answer 503.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Manifest and list cap. A larger file is rejected.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Upper bound on entries in one manifest.
const MAX_PACKS: usize = 32;

/// One pack, as `manifest.json` lists it and as `GET /api/tracker/packs` returns it.
#[derive(Debug, Clone, Serialize)]
pub struct PackEntry {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
}

/// Canonical pack directory plus the manifest entries for this request.
#[derive(Debug, Clone)]
pub struct LoadedPacks {
    pub root: PathBuf,
    pub entries: Vec<PackEntry>,
}

/// `PACKS_DIR` when it is set and not blank. Unset means pack downloads are off.
pub fn packs_dir_from_env() -> Option<PathBuf> {
    match std::env::var("PACKS_DIR") {
        Ok(value) if !value.trim().is_empty() => Some(PathBuf::from(value.trim())),
        _ => None,
    }
}

/// Remember `PACKS_DIR` when it does not overlap uploads, reports, or `dist`.
///
/// The directory does not have to be readable yet. The process still starts,
/// and each request returns 503 until a read succeeds. A conflicting path is
/// dropped so those routes stay off.
pub fn open_packs_dir(
    configured: Option<&Path>,
    upload_dir: &Path,
    reports_dir: &Path,
) -> Option<PathBuf> {
    let Some(configured) = configured else {
        tracing::info!("PACKS_DIR is unset; recognizer pack downloads are off");
        return None;
    };
    if packs_dir_conflicts(configured, upload_dir, reports_dir) {
        tracing::error!(
            dir = %configured.display(),
            "PACKS_DIR overlaps uploads, reports, or the web root; pack downloads are off"
        );
        return None;
    }
    match std::fs::metadata(configured) {
        Ok(meta) if meta.is_dir() => {
            if std::fs::read_dir(configured).is_err() {
                tracing::error!(
                    dir = %configured.display(),
                    "PACKS_DIR is not readable; pack routes will return 503 until it is"
                );
            }
        }
        Ok(_) => {
            tracing::error!(
                dir = %configured.display(),
                "PACKS_DIR is not a directory; pack routes will return 503 until it is"
            );
        }
        Err(error) => {
            tracing::error!(
                dir = %configured.display(),
                %error,
                "PACKS_DIR is not readable; pack routes will return 503 until it is"
            );
        }
    }
    Some(configured.to_path_buf())
}

/// Read `manifest.json` from a directory that is still safe to serve.
pub async fn load_packs(
    configured: &Path,
    upload_dir: &Path,
    reports_dir: &Path,
) -> Result<LoadedPacks, &'static str> {
    let meta = tokio::fs::metadata(configured)
        .await
        .map_err(|_| "PACKS_DIR is not readable")?;
    if !meta.is_dir() {
        return Err("PACKS_DIR is not a directory");
    }
    let root = tokio::fs::canonicalize(configured)
        .await
        .map_err(|_| "PACKS_DIR could not be resolved")?;
    if packs_dir_conflicts(&root, upload_dir, reports_dir) {
        return Err("PACKS_DIR overlaps uploads, reports, or the web root");
    }
    let entries = read_manifest(&root).await?;
    Ok(LoadedPacks { root, entries })
}

/// File inside `root` for an allowlisted name.
///
/// The name is one path segment. The opened path is canonicalized and must
/// still be a direct file inside `root`, so a symlink that leaves the
/// directory is rejected.
pub async fn resolve_pack_file(root: &Path, name: &str) -> Option<PathBuf> {
    if !is_pack_name(name) {
        return None;
    }
    let candidate = root.join(name);
    if candidate.parent() != Some(root) {
        return None;
    }
    let canon = tokio::fs::canonicalize(&candidate).await.ok()?;
    if !canon.starts_with(root) || canon.parent() != Some(root) {
        return None;
    }
    let meta = tokio::fs::metadata(&canon).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(canon)
}

/// Single file name. Not a path, not `..`, and not the manifest itself.
pub fn is_pack_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 || name == "manifest.json" || name.contains("..") {
        return false;
    }
    bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn is_pack_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

async fn read_manifest(root: &Path) -> Result<Vec<PackEntry>, &'static str> {
    let manifest_path = root.join("manifest.json");
    let meta = tokio::fs::metadata(&manifest_path)
        .await
        .map_err(|_| "packs manifest is missing")?;
    if !meta.is_file() {
        return Err("packs manifest is not a file");
    }
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err("packs manifest is too large");
    }
    let canon = tokio::fs::canonicalize(&manifest_path)
        .await
        .map_err(|_| "packs manifest could not be resolved")?;
    if !canon.starts_with(root) || canon.parent() != Some(root) {
        return Err("packs manifest is not inside PACKS_DIR");
    }
    let bytes = tokio::fs::read(&canon)
        .await
        .map_err(|_| "packs manifest is not readable")?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("packs manifest is too large");
    }
    let raw: Vec<ManifestEntry> =
        serde_json::from_slice(&bytes).map_err(|_| "packs manifest is not a JSON array")?;
    if raw.len() > MAX_PACKS {
        return Err("packs manifest has too many entries");
    }
    let mut seen = HashSet::with_capacity(raw.len());
    let mut entries = Vec::with_capacity(raw.len());
    for item in raw {
        if !is_pack_name(&item.name) {
            return Err("packs manifest has a name that is not a single file");
        }
        if !is_pack_version(&item.version) {
            return Err("packs manifest has a bad version");
        }
        if !is_sha256_hex(&item.sha256) {
            return Err("packs manifest has a bad sha256");
        }
        if !seen.insert(item.name.clone()) {
            return Err("packs manifest has a duplicate name");
        }
        entries.push(PackEntry {
            name: item.name,
            version: item.version,
            sha256: item.sha256,
            size: item.size,
        });
    }
    Ok(entries)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestEntry {
    name: String,
    version: String,
    sha256: String,
    size: u64,
}

fn packs_dir_conflicts(packs: &Path, upload_dir: &Path, reports_dir: &Path) -> bool {
    let packs = canonicalize_for_check(packs);
    if same_or_nested(&packs, &canonicalize_for_check(upload_dir)) {
        return true;
    }
    if same_or_nested(&packs, &canonicalize_for_check(reports_dir)) {
        return true;
    }
    let dist = canonicalize_for_check(Path::new("dist"));
    same_or_nested(&packs, &dist)
}

fn canonicalize_for_check(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    let mut suffix = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        if let Ok(canon) = cursor.canonicalize() {
            let mut out = canon;
            for part in suffix.iter().rev() {
                out.push(part);
            }
            return out;
        }
        let Some(parent) = cursor.parent() else {
            return path.to_path_buf();
        };
        if parent == cursor {
            return path.to_path_buf();
        }
        if let Some(name) = cursor.file_name() {
            suffix.push(name.to_os_string());
        }
        cursor = parent.to_path_buf();
    }
}

fn same_or_nested(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scuffed-packs-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn pack_names_are_single_file_segments() {
        assert!(is_pack_name("hero-icons.bin"));
        assert!(is_pack_name("a"));
        assert!(!is_pack_name(""));
        assert!(!is_pack_name(".."));
        assert!(!is_pack_name("."));
        assert!(!is_pack_name("../secret.bin"));
        assert!(!is_pack_name("foo/bar"));
        assert!(!is_pack_name("foo\\bar"));
        assert!(!is_pack_name(".hidden"));
        assert!(!is_pack_name("manifest.json"));
        assert!(!is_pack_name("a..b"));
    }

    #[test]
    fn open_drops_a_directory_that_overlaps_uploads() {
        let root = scratch("overlap");
        let uploads = root.join("uploads");
        let reports = root.join("reports");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::create_dir_all(&reports).unwrap();
        assert!(open_packs_dir(None, &uploads, &reports).is_none());
        assert!(open_packs_dir(Some(&uploads), &uploads, &reports).is_none());
        let packs = root.join("packs");
        std::fs::create_dir_all(&packs).unwrap();
        let opened = open_packs_dir(Some(&packs), &uploads, &reports).unwrap();
        assert_eq!(opened, packs);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn manifest_traversal_name_is_rejected_and_symlink_cannot_leave() {
        let root = scratch("resolve");
        let packs = root.join("packs");
        let outside = root.join("outside");
        std::fs::create_dir_all(&packs).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("secret.bin");
        std::fs::write(&secret, b"SECRET-BYTES-NOT-A-PACK\n").unwrap();
        let uploads = root.join("uploads");
        let reports = root.join("reports");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::create_dir_all(&reports).unwrap();

        let bad = packs.join("manifest.json");
        std::fs::write(
            &bad,
            br#"[{"name":"../secret.bin","version":"1","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}]"#,
        )
        .unwrap();
        let err = load_packs(&packs, &uploads, &reports).await.unwrap_err();
        assert_eq!(err, "packs manifest has a name that is not a single file");

        std::os::unix::fs::symlink(&secret, packs.join("linked.bin")).unwrap();
        let resolved = resolve_pack_file(&packs.canonicalize().unwrap(), "linked.bin").await;
        assert!(resolved.is_none());
        assert!(
            resolve_pack_file(&packs.canonicalize().unwrap(), "..")
                .await
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
