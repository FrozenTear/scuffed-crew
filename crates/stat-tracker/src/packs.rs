//! Reader packs from `GET /api/tracker/packs`.
//!
//! The list is a JSON array of `name`, `version`, `sha256`, and `size`.
//! The published pack is [`HEROES_PACK_NAME`] (version [`HEROES_PACK_VERSION`],
//! [`HEROES_PACK_SIZE`] bytes, sha256 [`HEROES_PACK_SHA256`]). The daemon
//! token is the same bearer used for stat uploads. The file is a plain ustar
//! archive with a top-level `manifest.json`. Template files land in
//! [`crate::shadow::heroes::HeroTemplates::dir_in`] as `<hero-key>.png` and
//! `special/<class>*.png`, the layout [`HeroTemplates::load_dir`] reads.
//! Size and sha256 are checked against the list before anything is unpacked.
//! A failed download leaves that directory as it was. The token is never
//! written to a log.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::config::validate_sync_server_url;
use crate::shadow::heroes::HeroTemplates;

/// `Retry-After` waits stay inside this range. The setup guide uses this
/// same clamp before it adds a wait to a clock.
pub const RETRY_AFTER_MIN_SECS: u64 = 1;
pub const RETRY_AFTER_MAX_SECS: u64 = 3600;
/// Used when a 429 names no wait.
pub const RATE_LIMIT_FALLBACK_SECS: u64 = 10;
/// Hard cap while streaming a pack. The list's `size` is not this cap.
pub const PACK_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// First entry of the published pack list (Vision).
pub const HEROES_PACK_NAME: &str = "heroes-v1.tar";
pub const HEROES_PACK_VERSION: &str = "1";
pub const HEROES_PACK_SHA256: &str =
    "3e2731330bc37a42ffd671ff98c73fa013c6463d577dbc83035747cc17167978";
pub const HEROES_PACK_SIZE: u64 = 1_280_000;

pub const PACK_SAVED: &str = "Reader pack saved.";
pub const PACK_CURRENT: &str = "The reader pack is already installed.";
pub const PACK_UNAVAILABLE: &str =
    "No reader pack is published on the site. The tracker will keep what is already installed.";
pub const PACK_UNSUPPORTED: &str = "This server does not offer reader packs yet.";
pub const PACK_TOO_BIG: &str = "The reader pack is too large to save.";
pub const PACK_MISMATCH: &str =
    "The reader pack did not match the site. The installed pack was left in place.";
pub const PACK_UNSAFE: &str =
    "The reader pack was not safe to unpack. The installed pack was left in place.";
pub const PACK_FAILED: &str =
    "Could not download the reader pack. The installed pack was left in place.";
pub const PACK_AUTH: &str =
    "The site rejected the tracker token. The installed pack was left in place.";
pub const PACK_SIGN_IN: &str = "Sign in first. Then the tracker can download the reader pack.";

/// A huge `Retry-After` must not be added to a clock.
pub fn clamp_retry_after(seconds: u64) -> u64 {
    seconds.clamp(RETRY_AFTER_MIN_SECS, RETRY_AFTER_MAX_SECS)
}

/// Header seconds when present, otherwise [`RATE_LIMIT_FALLBACK_SECS`], then clamped.
pub fn rate_limit_seconds(retry_after: Option<u64>) -> u64 {
    clamp_retry_after(retry_after.unwrap_or(RATE_LIMIT_FALLBACK_SECS))
}

pub fn rate_limit_message(seconds: u64) -> String {
    let seconds = clamp_retry_after(seconds);
    format!("Too many tries, wait {seconds} seconds and try again")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackSync {
    Installed,
    Current,
    /// HTTP 503 `packs_disabled`. Callers keep the installed files and stay quiet.
    Unavailable,
    /// HTTP 404 on the list. This server does not have the route yet.
    Unsupported,
    Failed(String),
}

impl PackSync {
    pub fn guide_result(self) -> Result<String, String> {
        match self {
            Self::Installed => Ok(PACK_SAVED.to_string()),
            Self::Current => Ok(PACK_CURRENT.to_string()),
            Self::Unavailable => Ok(PACK_UNAVAILABLE.to_string()),
            Self::Unsupported => Ok(PACK_UNSUPPORTED.to_string()),
            Self::Failed(message) => Err(message),
        }
    }
}

/// Daemon startup. Logs a plain warning on failure and never the token.
pub async fn sync_on_startup(server_url: &str, token: &str, data_dir: &Path) {
    match sync_reader_packs(server_url, token, data_dir, PACK_MAX_BYTES).await {
        PackSync::Installed => tracing::info!("reader pack installed"),
        PackSync::Current | PackSync::Unavailable => {}
        PackSync::Unsupported => {
            tracing::info!("reader packs are not supported on this server yet");
        }
        PackSync::Failed(message) => {
            tracing::warn!(message, "reader pack left in place");
        }
    }
}

pub async fn sync_reader_packs(
    server_url: &str,
    token: &str,
    data_dir: &Path,
    max_bytes: u64,
) -> PackSync {
    if token.trim().is_empty() || server_url.trim().is_empty() {
        return PackSync::Failed(PACK_SIGN_IN.to_string());
    }
    if validate_sync_server_url(server_url).is_err() {
        return PackSync::Failed(PACK_FAILED.to_string());
    }
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return PackSync::Failed(PACK_FAILED.to_string()),
    };
    let base = server_url.trim().trim_end_matches('/');
    let listed = match get_bytes(
        &client,
        &format!("{base}/api/tracker/packs"),
        token,
        64 * 1024,
    )
    .await
    {
        Get::Ok(bytes) => bytes,
        Get::Unavailable => return PackSync::Unavailable,
        Get::Unsupported => return PackSync::Unsupported,
        Get::Auth => return PackSync::Failed(PACK_AUTH.to_string()),
        Get::Limited(seconds) => return PackSync::Failed(rate_limit_message(seconds)),
        Get::Failed(message) => return PackSync::Failed(message),
    };
    let entries: Vec<PackListEntry> = match serde_json::from_slice(&listed) {
        Ok(entries) => entries,
        Err(_) => return PackSync::Failed(PACK_MISMATCH.to_string()),
    };
    let entry = match select_heroes_pack(&entries) {
        Ok(entry) => entry,
        Err(outcome) => return outcome,
    };
    let installed = read_versions(data_dir);
    if installed
        .get(HEROES_PACK_NAME)
        .is_some_and(|version| version == &entry.version)
    {
        return PackSync::Current;
    }
    let parent = HeroTemplates::dir_in(data_dir)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| data_dir.to_path_buf());
    if let Err(message) = std::fs::create_dir_all(&parent) {
        tracing::warn!(error = %message, "reader pack directory was not created");
        return PackSync::Failed(PACK_FAILED.to_string());
    }
    let staging = parent.join(format!(
        ".heroes-next-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    if std::fs::create_dir(&staging).is_err() {
        return PackSync::Failed(PACK_FAILED.to_string());
    }
    let bytes = match get_bytes(
        &client,
        &format!("{base}/api/tracker/packs/{HEROES_PACK_NAME}"),
        token,
        max_bytes,
    )
    .await
    {
        Get::Ok(bytes) => bytes,
        Get::Unavailable => {
            let _ = std::fs::remove_dir_all(&staging);
            return PackSync::Unavailable;
        }
        Get::Unsupported => {
            let _ = std::fs::remove_dir_all(&staging);
            return PackSync::Failed(PACK_FAILED.to_string());
        }
        Get::Auth => {
            let _ = std::fs::remove_dir_all(&staging);
            return PackSync::Failed(PACK_AUTH.to_string());
        }
        Get::Limited(seconds) => {
            let _ = std::fs::remove_dir_all(&staging);
            return PackSync::Failed(rate_limit_message(seconds));
        }
        Get::Failed(message) => {
            let _ = std::fs::remove_dir_all(&staging);
            return PackSync::Failed(message);
        }
    };
    // List size and sha256 are checked before any file is written.
    if bytes.len() as u64 != entry.size || sha256_hex(&bytes) != entry.sha256 {
        let _ = std::fs::remove_dir_all(&staging);
        return PackSync::Failed(PACK_MISMATCH.to_string());
    }
    if let Err(message) = unpack_pack(&bytes, &entry.name, &entry.version, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return PackSync::Failed(message.to_string());
    }
    let mut versions = BTreeMap::new();
    versions.insert(HEROES_PACK_NAME.to_string(), entry.version.clone());
    let live = HeroTemplates::dir_in(data_dir);
    if let Err(message) = swap_dir(&staging, &live) {
        let _ = std::fs::remove_dir_all(&staging);
        tracing::warn!(error = %message, "reader pack was not swapped into place");
        return PackSync::Failed(PACK_FAILED.to_string());
    }
    if let Err(message) = write_versions(data_dir, &versions) {
        tracing::warn!(error = %message, "reader pack version was not stored");
    }
    PackSync::Installed
}

#[derive(Debug, serde::Deserialize)]
struct PackListEntry {
    name: String,
    version: String,
    sha256: String,
    size: u64,
}

/// The first list entry is the hero template pack. Anything else is refused
/// before a download.
fn select_heroes_pack(entries: &[PackListEntry]) -> Result<&PackListEntry, PackSync> {
    let Some(entry) = entries.first() else {
        return Err(PackSync::Current);
    };
    if entry.name != HEROES_PACK_NAME || !is_sha256(&entry.sha256) || !is_version(&entry.version) {
        return Err(PackSync::Failed(PACK_MISMATCH.to_string()));
    }
    // Version 1 is the published Vision pack. A different size or sha256 is
    // not that pack, so it is not unpacked.
    if entry.version == HEROES_PACK_VERSION
        && (entry.sha256 != HEROES_PACK_SHA256 || entry.size != HEROES_PACK_SIZE)
    {
        return Err(PackSync::Failed(PACK_MISMATCH.to_string()));
    }
    Ok(entry)
}

enum Get {
    Ok(Vec<u8>),
    Unavailable,
    Unsupported,
    Auth,
    Limited(u64),
    Failed(String),
}

async fn get_bytes(client: &reqwest::Client, url: &str, token: &str, max_bytes: u64) -> Get {
    let response = match client.get(url).bearer_auth(token).send().await {
        Ok(response) => response,
        Err(_) => return Get::Failed(PACK_FAILED.to_string()),
    };
    let status = response.status().as_u16();
    if status == 401 {
        return Get::Auth;
    }
    if status == 404 {
        return Get::Unsupported;
    }
    if status == 503 {
        return Get::Unavailable;
    }
    if status == 429 {
        let header = retry_after_header(response.headers());
        let body = response.text().await.unwrap_or_default();
        let from_json = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("retry_after").and_then(|item| item.as_u64()));
        return Get::Limited(rate_limit_seconds(header.or(from_json)));
    }
    if !response.status().is_success() {
        return Get::Failed(PACK_FAILED.to_string());
    }
    match read_body_capped(response, max_bytes).await {
        Ok(bytes) => Get::Ok(bytes),
        Err(message) => Get::Failed(message.to_string()),
    }
}

fn retry_after_header(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse().ok())
}

async fn read_body_capped(
    mut response: reqwest::Response,
    max_bytes: u64,
) -> Result<Vec<u8>, &'static str> {
    if response.content_length().is_some_and(|len| len > max_bytes) {
        return Err(PACK_TOO_BIG);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| PACK_FAILED)? {
        let next = (body.len() as u64).saturating_add(chunk.len() as u64);
        if next > max_bytes {
            return Err(PACK_TOO_BIG);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn unpack_pack(
    bytes: &[u8],
    expected_name: &str,
    expected_version: &str,
    staging: &Path,
) -> Result<(), &'static str> {
    let files = read_ustar(bytes)?;
    let manifest_bytes = files
        .iter()
        .find(|(path, _)| path == "manifest.json")
        .map(|(_, data)| data.as_slice())
        .ok_or(PACK_MISMATCH)?;
    let manifest: InnerManifest =
        serde_json::from_slice(manifest_bytes).map_err(|_| PACK_MISMATCH)?;
    if manifest.name != expected_name || manifest.version != expected_version {
        return Err(PACK_MISMATCH);
    }
    if manifest.files.is_empty() {
        return Err(PACK_MISMATCH);
    }
    let mut listed = BTreeSet::new();
    for file in &manifest.files {
        let path = safe_rel_path(&file.path).ok_or(PACK_UNSAFE)?;
        if path == "manifest.json" || !listed.insert(path) {
            return Err(PACK_UNSAFE);
        }
        if !is_sha256(&file.sha256) {
            return Err(PACK_MISMATCH);
        }
    }
    let mut seen = BTreeSet::new();
    for (path, data) in &files {
        if path == "manifest.json" {
            continue;
        }
        if !seen.insert(path.clone()) {
            return Err(PACK_UNSAFE);
        }
        let listed_file = manifest
            .files
            .iter()
            .find(|file| safe_rel_path(&file.path).as_deref() == Some(path.as_str()))
            .ok_or(PACK_MISMATCH)?;
        if data.len() as u64 != listed_file.size || sha256_hex(data) != listed_file.sha256 {
            return Err(PACK_MISMATCH);
        }
        let dest = staging.join(path);
        if !dest.starts_with(staging) {
            return Err(PACK_UNSAFE);
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|_| PACK_FAILED)?;
        }
        if dest.symlink_metadata().is_ok() {
            return Err(PACK_UNSAFE);
        }
        let mut file = std::fs::File::create(&dest).map_err(|_| PACK_FAILED)?;
        file.write_all(data).map_err(|_| PACK_FAILED)?;
        file.sync_all().map_err(|_| PACK_FAILED)?;
    }
    if seen != listed {
        return Err(PACK_MISMATCH);
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
struct InnerManifest {
    name: String,
    version: String,
    files: Vec<InnerFile>,
}

#[derive(Debug, serde::Deserialize)]
struct InnerFile {
    path: String,
    sha256: String,
    size: u64,
}

fn swap_dir(staging: &Path, live: &Path) -> Result<(), String> {
    let parent = live.parent().unwrap_or_else(|| Path::new("."));
    let backup = parent.join(format!(
        ".heroes-prev-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let had_live = live.exists();
    if had_live {
        std::fs::rename(live, &backup).map_err(|err| err.to_string())?;
    }
    if let Err(err) = std::fs::rename(staging, live) {
        if had_live {
            let _ = std::fs::rename(&backup, live);
        }
        return Err(err.to_string());
    }
    if had_live {
        let _ = std::fs::remove_dir_all(&backup);
    }
    Ok(())
}

fn versions_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates").join("pack-versions.json")
}

fn read_versions(data_dir: &Path) -> BTreeMap<String, String> {
    let Ok(bytes) = std::fs::read(versions_path(data_dir)) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn write_versions(data_dir: &Path, versions: &BTreeMap<String, String>) -> Result<(), String> {
    let path = versions_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(versions).map_err(|err| err.to_string())?;
    {
        let mut file = std::fs::File::create(&tmp).map_err(|err| err.to_string())?;
        file.write_all(&bytes).map_err(|err| err.to_string())?;
        file.sync_all().map_err(|err| err.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|err| err.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn safe_rel_path(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') {
        return None;
    }
    if raw.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains('\0') {
            return None;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn read_ustar(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
    let mut offset = 0usize;
    let mut files = Vec::new();
    while offset + 512 <= bytes.len() {
        let header = &bytes[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        if !checksum_ok(header) {
            return Err(PACK_UNSAFE);
        }
        let typeflag = header[156];
        if header[124] & 0x80 != 0 {
            return Err(PACK_UNSAFE);
        }
        let size = parse_octal(&header[124..136]).map_err(|_| PACK_UNSAFE)?;
        let name = entry_name(header).ok_or(PACK_UNSAFE)?;
        offset += 512;
        let data_end = offset.saturating_add(size as usize);
        if data_end > bytes.len() {
            return Err(PACK_UNSAFE);
        }
        let data = bytes[offset..data_end].to_vec();
        let padded = size.div_ceil(512) * 512;
        offset = offset.saturating_add(padded as usize);
        match typeflag {
            b'0' | b'\0' => {
                let path = safe_rel_path(&name).ok_or(PACK_UNSAFE)?;
                files.push((path, data));
            }
            b'5' => {
                let _ = safe_rel_path(&name).ok_or(PACK_UNSAFE)?;
            }
            _ => return Err(PACK_UNSAFE),
        }
    }
    if files.is_empty() {
        return Err(PACK_MISMATCH);
    }
    Ok(files)
}

fn entry_name(header: &[u8]) -> Option<String> {
    let name = c_string(&header[..100]);
    let prefix = c_string(&header[345..500]);
    let joined = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn parse_octal(field: &[u8]) -> Result<u64, ()> {
    let mut value = 0u64;
    let mut saw = false;
    for &byte in field {
        if byte == 0 || byte == b' ' {
            if saw {
                break;
            }
            continue;
        }
        if !byte.is_ascii_digit() || byte > b'7' {
            return Err(());
        }
        saw = true;
        value = value
            .checked_mul(8)
            .ok_or(())?
            .checked_add(u64::from(byte - b'0'))
            .ok_or(())?;
    }
    Ok(value)
}

fn checksum_ok(header: &[u8]) -> bool {
    let Ok(recorded) = parse_octal(&header[148..156]) else {
        return false;
    };
    let sum = header.iter().enumerate().fold(0u64, |sum, (index, byte)| {
        sum + u64::from(if (148..156).contains(&index) {
            b' '
        } else {
            *byte
        })
    });
    sum == recorded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn ustar_header(name: &str, size: u64, typeflag: u8) -> [u8; 512] {
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
        header[156] = typeflag;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u64 = header.iter().map(|byte| u64::from(*byte)).sum();
        let checksum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(checksum.as_bytes());
        header
    }

    fn ustar_file(name: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = ustar_header(name, data.len() as u64, b'0').to_vec();
        bytes.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        bytes.extend(std::iter::repeat_n(0u8, pad));
        bytes
    }

    fn pack_bytes(name: &str, version: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let listed: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, data)| {
                serde_json::json!({
                    "path": path,
                    "sha256": sha256_hex(data),
                    "size": data.len(),
                })
            })
            .collect();
        let manifest = serde_json::json!({
            "name": name,
            "version": version,
            "files": listed,
        });
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let mut bytes = ustar_file("manifest.json", &manifest_bytes);
        for (path, data) in files {
            bytes.extend(ustar_file(path, data));
        }
        bytes.extend(std::iter::repeat_n(0u8, 1024));
        bytes
    }

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct Fake {
        token: String,
        list_status: u16,
        list_body: Vec<u8>,
        list_retry: Option<u64>,
        file_status: u16,
        file_body: Vec<u8>,
        file_chunked: bool,
        file_hits: Arc<std::sync::atomic::AtomicUsize>,
        requested: Arc<Mutex<Vec<String>>>,
    }

    fn spawn_fake(fake: Arc<Fake>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for _ in 0..8 {
                let Ok((mut sock, _)) = listener.accept() else {
                    break;
                };
                sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .expect("timeout");
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                loop {
                    match std::io::Read::read(&mut sock, &mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let head = String::from_utf8_lossy(&buf);
                let authed = head.lines().any(|line| {
                    line.eq_ignore_ascii_case(&format!("authorization: bearer {}", fake.token))
                });
                let path = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("");
                fake.requested.lock().expect("paths").push(path.to_string());
                let response = if !authed {
                    http_bytes(401, br#"{"error":"Unauthorized"}"#, None, false)
                } else if path == "/api/tracker/packs" {
                    http_bytes(fake.list_status, &fake.list_body, fake.list_retry, false)
                } else if path.starts_with("/api/tracker/packs/") {
                    fake.file_hits
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    http_bytes(fake.file_status, &fake.file_body, None, fake.file_chunked)
                } else {
                    http_bytes(404, br#"{"error":"pack_not_found"}"#, None, false)
                };
                let _ = std::io::Write::write_all(&mut sock, &response);
            }
        });
        format!("http://{addr}")
    }

    fn http_bytes(status: u16, body: &[u8], retry_after: Option<u64>, chunked: bool) -> Vec<u8> {
        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            404 => "Not Found",
            429 => "Too Many Requests",
            503 => "Service Unavailable",
            _ => "Error",
        };
        if chunked {
            let mut out = format!(
                "HTTP/1.1 {status} {reason}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
            )
            .into_bytes();
            out.extend(format!("{:x}\r\n", body.len()).into_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\r\n0\r\n\r\n");
            return out;
        }
        let mut headers = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(seconds) = retry_after {
            headers.push_str(&format!("Retry-After: {seconds}\r\n"));
        }
        headers.push_str("\r\n");
        let mut out = headers.into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn list_json(name: &str, version: &str, bytes: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([{
            "name": name,
            "version": version,
            "sha256": sha256_hex(bytes),
            "size": bytes.len(),
        }]))
        .unwrap()
    }

    #[test]
    fn retry_after_clamp_matches_the_setup_guide_range() {
        assert_eq!(clamp_retry_after(0), 1);
        assert_eq!(clamp_retry_after(3601), 3600);
        assert_eq!(clamp_retry_after(u64::MAX), 3600);
        assert_eq!(rate_limit_seconds(Some(u64::MAX)), 3600);
        assert_eq!(
            rate_limit_message(u64::MAX),
            "Too many tries, wait 3600 seconds and try again"
        );
    }

    fn requested_paths() -> Arc<Mutex<Vec<String>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    #[test]
    fn vision_pack_manifest_matches_the_published_entry() {
        let raw = r#"[{"name":"heroes-v1.tar","version":"1","sha256":"3e2731330bc37a42ffd671ff98c73fa013c6463d577dbc83035747cc17167978","size":1280000}]"#;
        let entries: Vec<PackListEntry> = serde_json::from_str(raw).expect("list");
        let entry = select_heroes_pack(&entries).expect("first pack");
        assert_eq!(entry.name, HEROES_PACK_NAME);
        assert_eq!(entry.version, HEROES_PACK_VERSION);
        assert_eq!(entry.sha256, HEROES_PACK_SHA256);
        assert_eq!(entry.size, HEROES_PACK_SIZE);
        assert_eq!(HEROES_PACK_NAME, "heroes-v1.tar");
        assert_eq!(HEROES_PACK_VERSION, "1");
        assert_eq!(
            HEROES_PACK_SHA256,
            "3e2731330bc37a42ffd671ff98c73fa013c6463d577dbc83035747cc17167978"
        );
        assert_eq!(HEROES_PACK_SIZE, 1_280_000);
        let wrong = vec![PackListEntry {
            name: "heroes-v1.tar".into(),
            version: "1".into(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            size: 1280000,
        }];
        assert_eq!(
            select_heroes_pack(&wrong).unwrap_err(),
            PackSync::Failed(PACK_MISMATCH.into())
        );
    }

    #[test]
    fn ustar_rejects_absolute_paths_dotdot_and_links() {
        let escape = ustar_file("../escape.txt", b"nope");
        assert_eq!(read_ustar(&escape).unwrap_err(), PACK_UNSAFE);
        let absolute = ustar_file("/tmp/escape.txt", b"nope");
        assert_eq!(read_ustar(&absolute).unwrap_err(), PACK_UNSAFE);
        let mut link = ustar_header("link.txt", 0, b'2');
        link[157..161].copy_from_slice(b"dest");
        let sum: u64 = {
            let mut spaced = link;
            for byte in &mut spaced[148..156] {
                *byte = b' ';
            }
            spaced.iter().map(|byte| u64::from(*byte)).sum()
        };
        let checksum = format!("{sum:06o}\0 ");
        link[148..156].copy_from_slice(checksum.as_bytes());
        assert_eq!(read_ustar(&link).unwrap_err(), PACK_UNSAFE);
    }

    #[tokio::test]
    async fn chunked_download_without_content_length_stops_at_the_cap() {
        let body = b"abcdefghijklmnopqrst";
        let fake = Arc::new(Fake {
            token: "pack-token".into(),
            list_status: 200,
            list_body: Vec::new(),
            list_retry: None,
            file_status: 200,
            file_body: body.to_vec(),
            file_chunked: true,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{base}/api/tracker/packs/{HEROES_PACK_NAME}"))
            .bearer_auth("pack-token")
            .send()
            .await
            .expect("chunked response");
        assert!(response.content_length().is_none());
        let err = read_body_capped(response, 8)
            .await
            .expect_err("over the cap");
        assert_eq!(err, PACK_TOO_BIG);
    }

    #[tokio::test]
    async fn synthetic_tar_unpacks_into_the_reader_layout() {
        let token = "pack-token";
        // Not the published 1280000-byte archive. Version 1 is pinned to that
        // file, so this fixture uses another version label and its own hash.
        let hero: &[u8] = b"synthetic-hero";
        let special: &[u8] = b"synthetic-special";
        let bytes = pack_bytes(
            HEROES_PACK_NAME,
            "2",
            &[("ana.png", hero), ("special/placeholder_01.png", special)],
        );
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requested = requested_paths();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: list_json(HEROES_PACK_NAME, "2", &bytes),
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: false,
            file_hits: hits.clone(),
            requested: requested.clone(),
        });
        let base = spawn_fake(fake);
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        assert_eq!(heroes, dir.path().join("templates/heroes"));
        let first = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(first, PackSync::Installed);
        assert_eq!(std::fs::read(heroes.join("ana.png")).expect("hero"), hero);
        assert_eq!(
            std::fs::read(heroes.join("special/placeholder_01.png")).expect("special"),
            special
        );
        assert!(!heroes.join("manifest.json").exists());
        let stored: BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(versions_path(dir.path())).unwrap()).unwrap();
        assert_eq!(stored.get(HEROES_PACK_NAME).map(String::as_str), Some("2"));
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        let paths = requested.lock().expect("paths").clone();
        assert!(paths.iter().any(|path| path == "/api/tracker/packs"));
        assert!(
            paths
                .iter()
                .any(|path| path == "/api/tracker/packs/heroes-v1.tar")
        );

        let second = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(second, PackSync::Current);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn published_pack_is_checked_before_it_is_unpacked() {
        let token = "pack-token";
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("ana.png"), b"old").unwrap();
        let small = pack_bytes(
            HEROES_PACK_NAME,
            HEROES_PACK_VERSION,
            &[("ana.png", b"nope")],
        );
        let listed = format!(
            r#"[{{"name":"{HEROES_PACK_NAME}","version":"{HEROES_PACK_VERSION}","sha256":"{HEROES_PACK_SHA256}","size":{HEROES_PACK_SIZE}}}]"#
        );
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requested = requested_paths();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: listed.into_bytes(),
            list_retry: None,
            file_status: 200,
            file_body: small,
            file_chunked: false,
            file_hits: hits.clone(),
            requested: requested.clone(),
        });
        let base = spawn_fake(fake);
        let result = sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await;
        assert_eq!(result, PackSync::Failed(PACK_MISMATCH.into()));
        assert_eq!(std::fs::read(heroes.join("ana.png")).unwrap(), b"old");
        assert!(!heroes.join("special").exists());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        let paths = requested.lock().expect("paths").clone();
        assert!(
            paths
                .iter()
                .any(|path| path == "/api/tracker/packs/heroes-v1.tar")
        );
    }

    #[tokio::test]
    async fn a_bad_pack_keeps_the_old_files_and_does_not_log_the_token() {
        let token = "pack-token-secret";
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
        let bytes = pack_bytes(HEROES_PACK_NAME, "2", &[("ana.png", b"new")]);
        let listed = serde_json::to_vec(&serde_json::json!([{
            "name": HEROES_PACK_NAME,
            "version": "2",
            "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "size": bytes.len(),
        }]))
        .unwrap();
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 200,
            list_body: listed,
            list_retry: None,
            file_status: 200,
            file_body: bytes,
            file_chunked: false,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let log = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || SharedWriter(writer.clone()))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        sync_on_startup(base.as_str(), token, dir.path()).await;
        drop(_guard);
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
        assert!(!heroes.join("ana.png").exists());
        let logs = String::from_utf8(log.lock().unwrap().clone()).unwrap();
        assert!(logs.contains(PACK_MISMATCH), "{logs}");
        assert!(!logs.contains(token), "{logs}");
    }

    #[tokio::test]
    async fn disabled_and_missing_routes_do_not_replace_installed_files() {
        let token = "pack-token";
        let dir = tempfile::tempdir().expect("tempdir");
        let heroes = HeroTemplates::dir_in(dir.path());
        std::fs::create_dir_all(&heroes).unwrap();
        std::fs::write(heroes.join("keep.txt"), b"old").unwrap();
        let disabled = Arc::new(Fake {
            token: token.into(),
            list_status: 503,
            list_body: br#"{"error":"packs_disabled"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(disabled);
        assert_eq!(
            sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await,
            PackSync::Unavailable
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");

        let missing = Arc::new(Fake {
            token: token.into(),
            list_status: 404,
            list_body: br#"{"error":"not_found"}"#.to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(missing);
        assert_eq!(
            sync_reader_packs(base.as_str(), token, dir.path(), PACK_MAX_BYTES).await,
            PackSync::Unsupported
        );
        assert_eq!(std::fs::read(heroes.join("keep.txt")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn rate_limit_uses_the_clamped_retry_after() {
        let token = "pack-token";
        let fake = Arc::new(Fake {
            token: token.into(),
            list_status: 429,
            list_body: br#"{"error":"rate_limited","retry_after":1}"#.to_vec(),
            list_retry: Some(u64::MAX),
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let result = sync_reader_packs(base.as_str(), token, Path::new("/tmp"), 32).await;
        assert_eq!(
            result,
            PackSync::Failed("Too many tries, wait 3600 seconds and try again".into())
        );
    }

    #[tokio::test]
    async fn a_rejected_token_is_a_plain_401() {
        let fake = Arc::new(Fake {
            token: "expected".into(),
            list_status: 200,
            list_body: b"[]".to_vec(),
            list_retry: None,
            file_status: 200,
            file_body: Vec::new(),
            file_chunked: false,
            file_hits: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            requested: requested_paths(),
        });
        let base = spawn_fake(fake);
        let result = sync_reader_packs(base.as_str(), "wrong-token", Path::new("/tmp"), 32).await;
        assert_eq!(result, PackSync::Failed(PACK_AUTH.into()));
    }
}
