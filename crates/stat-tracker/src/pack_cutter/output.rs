//! Build output stays outside any git work tree. The out directory is
//! mode 0700 and each tar is mode 0600.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListEntry {
    name: String,
    version: String,
    sha256: String,
    size: u64,
}

/// Absolute `--out`, or an error when that path sits inside a git work tree.
pub fn check_out(path: &Path) -> Result<PathBuf, String> {
    let resolved = resolve(path)?;
    if inside_git(&resolved) {
        return Err(format!(
            "refusing to write {}: it is inside a git work tree",
            resolved.display()
        ));
    }
    Ok(resolved)
}

pub fn check_version(version: &str) -> Result<(), String> {
    let bytes = version.as_bytes();
    if (1..=32).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(format!(
            "version {version:?} must be 1 to 32 of A-Z, a-z, 0-9, '.', '_' or '-'"
        ))
    }
}

/// Publish accepts a pack name of `[a-z0-9][a-z0-9-]*.tar`.
pub fn check_pack_name(name: &str) -> Result<(), String> {
    let Some(stem) = name.strip_suffix(".tar") else {
        return Err(format!("pack name {name:?} must end in .tar"));
    };
    let bytes = stem.as_bytes();
    let ok = name.len() <= 64
        && bytes.split_first().is_some_and(|(first, rest)| {
            (first.is_ascii_lowercase() || first.is_ascii_digit())
                && rest
                    .iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(format!(
            "pack name {name:?} must match [a-z0-9][a-z0-9-]*.tar"
        ))
    }
}

/// Write `bytes` to `<out>/packs/<name>` and update `<out>/packs/manifest.json`.
pub fn install_pack(out: &Path, name: &str, version: &str, bytes: &[u8]) -> Result<String, String> {
    let packs = out.join("packs");
    private_dir(out)?;
    private_dir(&packs)?;
    let dest = packs.join(name);
    write_private(&dest, bytes)?;
    let list_path = packs.join("manifest.json");
    let mut entries = read_list(&list_path)?;
    let entry = ListEntry {
        name: name.to_string(),
        version: version.to_string(),
        sha256: super::ustar::sha256_hex(bytes),
        size: bytes.len() as u64,
    };
    let mut replaced = false;
    let mut next = Vec::with_capacity(entries.len() + 1);
    for old in entries.drain(..) {
        if old.name == entry.name {
            if !replaced {
                next.push(entry.clone());
                replaced = true;
            }
        } else {
            next.push(old);
        }
    }
    if !replaced {
        next.push(entry.clone());
    }
    write_list(&list_path, &next)?;
    Ok(format!(
        "packed {name} version {version}\nsha256: {}\nsize: {}\nwrote: {}\nlist: {}\n",
        entry.sha256,
        entry.size,
        dest.display(),
        list_path.display()
    ))
}

pub fn private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("creating {}: {e}", path.display()))?;
    chmod_owner(path, 0o700)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_file_name(format!(
        ".{}.partial",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("pack")
    ));
    let write = (|| -> Result<(), String> {
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut file = opts
                .open(&tmp)
                .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
            std::io::Write::write_all(&mut file, bytes)
                .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
            file.sync_all()
                .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        }
        chmod_owner(&tmp, 0o600)?;
        std::fs::rename(&tmp, path).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write
}

fn read_list(path: &Path) -> Result<Vec<ListEntry>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("reading {}: {e}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

fn write_list(path: &Path, entries: &[ListEntry]) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let tmp = path.with_file_name(".manifest.json.partial");
    let write = (|| -> Result<(), String> {
        std::fs::write(&tmp, &bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(())
    })();
    if write.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write
}

fn chmod_owner(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| format!("setting mode {mode:o} on {}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

fn resolve(path: &Path) -> Result<PathBuf, String> {
    let abs =
        std::path::absolute(path).map_err(|e| format!("resolving {}: {e}", path.display()))?;
    let mut pending = Vec::new();
    let mut cur = abs;
    while !cur.exists() {
        match cur.file_name() {
            Some(name) => pending.push(name.to_os_string()),
            None => break,
        }
        if !cur.pop() {
            break;
        }
    }
    let mut resolved = if cur.exists() {
        cur.canonicalize()
            .map_err(|e| format!("resolving {}: {e}", cur.display()))?
    } else {
        cur
    };
    for name in pending.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

/// A `.git` file (a linked work tree) or directory on `path` or a parent.
fn inside_git(path: &Path) -> bool {
    let mut cur = path.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return true;
        }
        if !cur.pop() {
            return false;
        }
    }
}
