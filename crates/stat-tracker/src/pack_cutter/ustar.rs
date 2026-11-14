//! Plain ustar, matching the header layout of `ustar_file` / `build_pack`
//! in `packs/mod.rs` (mtime 0, mode 0644, uid and gid 0, empty uname and
//! gname, regular files only). No tar crate.

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const SOURCE: &str = "scuffed-pack-cutter";

pub struct Member {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Serialize)]
struct InnerFile {
    path: String,
    sha256: String,
    size: u64,
}

#[derive(Serialize)]
struct InnerManifest {
    name: String,
    version: String,
    files: Vec<InnerFile>,
    source: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A ustar whose first member is the inner manifest and whose later members
/// are `files`, in order. The manifest lists those files, not itself.
pub fn archive(name: &str, version: &str, files: &[Member]) -> Result<Vec<u8>, String> {
    let listed: Vec<InnerFile> = files
        .iter()
        .map(|file| InnerFile {
            path: file.path.clone(),
            sha256: sha256_hex(&file.bytes),
            size: file.bytes.len() as u64,
        })
        .collect();
    let manifest = InnerManifest {
        name: name.to_string(),
        version: version.to_string(),
        files: listed,
        source: SOURCE.to_string(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    let mut members = vec![Member {
        path: "manifest.json".into(),
        bytes: manifest_bytes,
    }];
    members.extend(files.iter().map(|file| Member {
        path: file.path.clone(),
        bytes: file.bytes.clone(),
    }));
    let mut bytes = Vec::new();
    for member in &members {
        check_member(&member.path)?;
        bytes.extend(ustar_file(&member.path, &member.bytes)?);
    }
    bytes.extend(std::iter::repeat_n(0u8, 1024));
    Ok(bytes)
}

fn check_member(name: &str) -> Result<(), String> {
    let bad = name.is_empty()
        || name.len() > 100
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.as_bytes().get(1) == Some(&b':')
        || name.contains('\0');
    let bad = bad
        || name
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part == "." || part == "..");
    if bad {
        Err(format!("refusing tar member {name:?}"))
    } else {
        Ok(())
    }
}

fn ustar_file(name: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    // 11 octal digits, the same width `ustar_file` in packs/mod.rs writes.
    if data.len() as u64 > 0o777_7777_7777 {
        return Err(format!("{name} does not fit in a ustar size field"));
    }
    let mut bytes = ustar_header(name, data.len() as u64).to_vec();
    bytes.extend_from_slice(data);
    let pad = (512 - (data.len() % 512)) % 512;
    bytes.extend(std::iter::repeat_n(0u8, pad));
    Ok(bytes)
}

fn ustar_header(name: &str, size: u64) -> [u8; 512] {
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..107].copy_from_slice(b"0000644");
    header[108..115].copy_from_slice(b"0000000");
    header[116..123].copy_from_slice(b"0000000");
    let size_field = format!("{size:011o}");
    header[124..135].copy_from_slice(size_field.as_bytes());
    header[136..147].copy_from_slice(b"00000000000");
    for byte in &mut header[148..156] {
        *byte = b' ';
    }
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
    let checksum = format!("{sum:06o}\0 ");
    header[148..156].copy_from_slice(checksum.as_bytes());
    header
}
