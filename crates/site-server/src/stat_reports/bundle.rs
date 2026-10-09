//! Zip acceptance for a tracker report bundle.
//!
//! The upload is checked against the manifest, then PNG ancillary chunks are
//! stripped and the stored zip is rebuilt so those chunks are not kept.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use scuffed_types::{
    CheckedManifest, FileRole, MAX_BUNDLE_BYTES, MAX_FILES_BESIDES_MANIFEST, parse_bundle_manifest,
};

use super::png::prepare_png;

pub enum BundleReject {
    TooLarge,
    Invalid(String),
}

pub struct AcceptedBundle {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub manifest: CheckedManifest,
}

pub fn accept_bundle(input: &[u8]) -> Result<AcceptedBundle, BundleReject> {
    if input.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(BundleReject::TooLarge);
    }
    if input.is_empty() {
        return Err(BundleReject::Invalid("empty body".into()));
    }

    let mut archive = ZipArchive::new(Cursor::new(input))
        .map_err(|_| BundleReject::Invalid("not a zip".into()))?;
    let count = archive.len();
    if count == 0 || count > MAX_FILES_BESIDES_MANIFEST + 1 {
        return Err(BundleReject::Invalid(
            "zip entry count is not allowed".into(),
        ));
    }

    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut total = 0u64;
    for index in 0..count {
        let mut file = archive
            .by_index(index)
            .map_err(|_| BundleReject::Invalid("zip entry is unreadable".into()))?;
        if file.is_dir() {
            return Err(BundleReject::Invalid("zip has a directory entry".into()));
        }
        if file.encrypted() {
            return Err(BundleReject::Invalid(
                "encrypted zip entries are not allowed".into(),
            ));
        }
        match file.compression() {
            CompressionMethod::Stored | CompressionMethod::Deflated => {}
            _ => {
                return Err(BundleReject::Invalid(
                    "zip compression method is not allowed".into(),
                ));
            }
        }
        let name = file.name().to_string();
        if !safe_entry_name(&name) {
            return Err(BundleReject::Invalid("zip path is not allowed".into()));
        }
        if file.enclosed_name().is_none() {
            return Err(BundleReject::Invalid("zip path is not allowed".into()));
        }
        let declared = file.size();
        if declared > MAX_BUNDLE_BYTES || total.saturating_add(declared) > MAX_BUNDLE_BYTES {
            return Err(BundleReject::TooLarge);
        }
        let data = read_capped(&mut file, MAX_BUNDLE_BYTES - total)?;
        if declared != data.len() as u64 {
            return Err(BundleReject::Invalid(
                "zip size does not match the entry".into(),
            ));
        }
        total += data.len() as u64;
        if entries.insert(name, data).is_some() {
            return Err(BundleReject::Invalid("duplicate zip entry".into()));
        }
    }

    let Some(manifest_bytes) = entries.get("manifest.json").cloned() else {
        return Err(BundleReject::Invalid("manifest.json is missing".into()));
    };
    let mut manifest = parse_bundle_manifest(&manifest_bytes).map_err(manifest_reject)?;

    let mut expected: BTreeSet<String> = manifest.files.iter().map(|f| f.path.clone()).collect();
    expected.insert("manifest.json".into());
    let actual: BTreeSet<String> = entries.keys().cloned().collect();
    if actual != expected {
        return Err(BundleReject::Invalid(
            "zip entries do not match the manifest".into(),
        ));
    }

    for file in &manifest.files {
        let data = entries
            .get(&file.path)
            .ok_or_else(|| BundleReject::Invalid("listed file is missing".into()))?;
        if data.len() as u64 != file.bytes {
            return Err(BundleReject::Invalid(
                "file size does not match the manifest".into(),
            ));
        }
        if sha256_hex(data) != file.sha256 {
            return Err(BundleReject::Invalid(
                "file hash does not match the manifest".into(),
            ));
        }
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Err(BundleReject::Invalid("jpeg is not allowed".into()));
        }
        match file.role {
            FileRole::Log => {
                if std::str::from_utf8(data).is_err() {
                    return Err(BundleReject::Invalid("log.txt is not utf-8".into()));
                }
            }
            FileRole::Crop | FileRole::OwnName | FileRole::Glyph => {
                let stripped = prepare_png(data).map_err(BundleReject::Invalid)?;
                if stripped != *data {
                    let sha = sha256_hex(&stripped);
                    let len = stripped.len() as u64;
                    rewrite_file_meta(&mut manifest.value, &file.path, &sha, len)?;
                    // The checked copy is updated below, after this loop.
                }
            }
        }
    }

    // Second pass applies stripped bytes so the hash check above saw the upload.
    for file in &mut manifest.files {
        if file.role == FileRole::Log {
            continue;
        }
        let original = entries
            .get(&file.path)
            .ok_or_else(|| BundleReject::Invalid("listed file is missing".into()))?;
        let stripped = prepare_png(original).map_err(BundleReject::Invalid)?;
        file.sha256 = sha256_hex(&stripped);
        file.bytes = stripped.len() as u64;
        entries.insert(file.path.clone(), stripped);
    }

    let manifest_out = serde_json::to_vec(&manifest.value)
        .map_err(|_| BundleReject::Invalid("manifest could not be stored".into()))?;
    entries.insert("manifest.json".into(), manifest_out);

    let mut stored_total = 0u64;
    for data in entries.values() {
        stored_total = stored_total.saturating_add(data.len() as u64);
        if stored_total > MAX_BUNDLE_BYTES {
            return Err(BundleReject::TooLarge);
        }
    }

    let bytes = rebuild_zip(&manifest, &entries)?;
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(BundleReject::TooLarge);
    }
    let sha256 = sha256_hex(&bytes);
    Ok(AcceptedBundle {
        bytes,
        sha256,
        manifest,
    })
}

fn manifest_reject(msg: String) -> BundleReject {
    if msg.contains("uncompressed size") {
        BundleReject::TooLarge
    } else {
        BundleReject::Invalid(msg)
    }
}

fn rewrite_file_meta(
    value: &mut serde_json::Value,
    path: &str,
    sha256: &str,
    bytes: u64,
) -> Result<(), BundleReject> {
    let files = value
        .get_mut("files")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| BundleReject::Invalid("manifest files are missing".into()))?;
    for file in files {
        if file.get("path").and_then(|p| p.as_str()) == Some(path) {
            let obj = file
                .as_object_mut()
                .ok_or_else(|| BundleReject::Invalid("manifest file is not an object".into()))?;
            obj.insert(
                "sha256".to_string(),
                serde_json::Value::String(sha256.to_string()),
            );
            obj.insert("bytes".to_string(), serde_json::Value::Number(bytes.into()));
            return Ok(());
        }
    }
    Err(BundleReject::Invalid("manifest file is missing".into()))
}

fn rebuild_zip(
    manifest: &CheckedManifest,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, BundleReject> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let mut order = vec!["manifest.json".to_string()];
    order.extend(manifest.files.iter().map(|f| f.path.clone()));
    for path in order {
        let data = entries
            .get(&path)
            .ok_or_else(|| BundleReject::Invalid("listed file is missing".into()))?;
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer
            .start_file(path, opts)
            .map_err(|_| BundleReject::Invalid("could not store the bundle".into()))?;
        writer
            .write_all(data)
            .map_err(|_| BundleReject::Invalid("could not store the bundle".into()))?;
    }
    let cursor = writer
        .finish()
        .map_err(|_| BundleReject::Invalid("could not store the bundle".into()))?;
    Ok(cursor.into_inner())
}

fn read_capped(reader: &mut impl Read, cap: u64) -> Result<Vec<u8>, BundleReject> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|_| BundleReject::Invalid("zip entry is unreadable".into()))?;
        if n == 0 {
            break;
        }
        if out.len() as u64 + n as u64 > cap {
            return Err(BundleReject::TooLarge);
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

fn safe_entry_name(path: &str) -> bool {
    if path.is_empty() || path.len() > 128 || path.ends_with('/') {
        return false;
    }
    if path.starts_with('/') || path.starts_with('.') || path.contains('\\') || path.contains('\0')
    {
        return false;
    }
    if path.contains(':') || path.contains("//") {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return false;
    }
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.') {
            return false;
        }
        if seg.eq_ignore_ascii_case("__macosx") {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
        {
            return false;
        }
    }
    true
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let dig = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in dig {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
