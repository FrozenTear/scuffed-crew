//! In-place edits of `config.toml`: [`Config::text_for_save`] and the
//! first-run guide's [`Config::apply_setup_patch`]. A hand-edited file keeps
//! its comments, order and unknown keys when only an in-place key changes.

use super::file::{atomic_write_600, unreadable_config};
use super::parse_error::{TomlFailure, describe};
use super::{CaptureRoute, Config, OcrSetting, ReaderSetting, SyncConfig};

/// `existing` did not parse. The text names the line and toml's reason,
/// never the line itself, so the sync token cannot reach Settings or a log.
fn not_parsed(existing: &str, err: &impl TomlFailure) -> Box<dyn std::error::Error + Send + Sync> {
    unreadable_config(format!(
        "config.toml could not be parsed, so it was not replaced: {}",
        describe(existing, err)
    ))
}

/// The in-place edit produced `text` that does not parse. Same redaction
/// as [`not_parsed`].
fn not_patched(text: &str, err: &impl TomlFailure) -> Box<dyn std::error::Error + Send + Sync> {
    unreadable_config(format!(
        "config.toml could not be updated in place, so it was not replaced: {}",
        describe(text, err)
    ))
}

impl Config {
    /// Text `save` would write.
    ///
    /// An `existing` document that parses as `next` is returned unchanged
    /// (byte for byte), comments included. When the only change is
    /// `shadow_recognizer`, `reader`, `ocr`, `update_channel`, or
    /// `capture_route` (or `reader` together with `shadow_recognizer`), that
    /// key is edited in place so comments, order, and every other key stay as
    /// they were. A
    /// document that does not parse is an error, not a pretty-printed
    /// replacement. A missing file (no `existing` text) is a fresh
    /// pretty-printed document.
    pub fn text_for_save(
        existing: Option<&str>,
        next: &Self,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let Some(existing) = existing else {
            return Ok(toml::to_string_pretty(next)?);
        };
        let loaded = toml::from_str::<Self>(existing).map_err(|err| not_parsed(existing, &err))?;
        if loaded == *next {
            return Ok(existing.to_string());
        }
        if only_shadow_recognizer_differs(&loaded, next) {
            return patch_shadow_recognizer(existing, next.shadow_recognizer);
        }
        if only_update_channel_differs(&loaded, next) {
            return patch_update_channel(existing, &next.update_channel);
        }
        if only_reader_differs(&loaded, next) {
            return patch_reader(existing, next.reader);
        }
        if only_reader_and_shadow_differ(&loaded, next) {
            let with_shadow = patch_shadow_recognizer(existing, next.shadow_recognizer)?;
            return patch_reader(&with_shadow, next.reader);
        }
        if only_ocr_differs(&loaded, next) {
            return patch_ocr(existing, next.ocr);
        }
        if only_capture_route_differs(&loaded, next) {
            return patch_capture_route(existing, next.capture_route);
        }
        Ok(toml::to_string_pretty(next)?)
    }
}

fn only_capture_route_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.capture_route == next.capture_route {
        return false;
    }
    let mut same = loaded.clone();
    same.capture_route = next.capture_route;
    same == *next
}

fn patch_capture_route(
    existing: &str,
    route: CaptureRoute,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let word = route.as_str();
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get("capture_route").is_some() {
        return splice_root_string(existing, &doc, "capture_route", word);
    }
    insert_root_string(existing, "capture_route", word)
}

fn only_shadow_recognizer_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.shadow_recognizer == next.shadow_recognizer {
        return false;
    }
    let mut same = loaded.clone();
    same.shadow_recognizer = next.shadow_recognizer;
    same == *next
}

fn only_update_channel_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.update_channel == next.update_channel {
        return false;
    }
    let mut same = loaded.clone();
    same.update_channel.clone_from(&next.update_channel);
    same == *next
}

fn channel_word(raw: &str) -> &'static str {
    if raw.trim().eq_ignore_ascii_case("prerelease") {
        "prerelease"
    } else {
        "stable"
    }
}

/// Change `update_channel` and nothing else.
///
/// A root string is spliced in place, so comments, order, spacing, and
/// every other key stay. A missing root key is inserted on the root table.
fn patch_update_channel(
    existing: &str,
    raw: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let word = channel_word(raw);
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get("update_channel").is_some() {
        return splice_root_string(existing, &doc, "update_channel", word);
    }
    insert_root_string(existing, "update_channel", word)
}

fn splice_root_string(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    key: &str,
    word: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let quoted = format!("\"{word}\"");
    if let Some(item) = doc.get(key)
        && let Some(value) = item.as_value()
        && value.as_str().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        let replacement = if current.starts_with('"') || current.starts_with('\'') {
            quoted
        } else {
            word.to_string()
        };
        let mut out = String::with_capacity(existing.len() + replacement.len());
        out.push_str(&existing[..span.start]);
        out.push_str(&replacement);
        out.push_str(&existing[span.end..]);
        return Ok(out);
    }
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

fn insert_root_string(
    existing: &str,
    key: &str,
    word: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| not_parsed(existing, &err))?;
    if doc.get(key).is_some() {
        return Err(unreadable_config(
            "config.toml already has that key at the root, so it was not replaced".to_string(),
        ));
    }
    doc.as_table_mut().insert(key, toml_edit::value(word));
    Ok(doc.to_string())
}

fn only_reader_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.reader == next.reader {
        return false;
    }
    let mut same = loaded.clone();
    same.reader = next.reader;
    same == *next
}

fn only_ocr_differs(loaded: &Config, next: &Config) -> bool {
    if loaded.ocr == next.ocr {
        return false;
    }
    let mut same = loaded.clone();
    same.ocr = next.ocr;
    same == *next
}

/// Change `ocr` and nothing else.
///
/// A root string is spliced in place. A missing root key is inserted on the
/// root table, before the first `[table]` header.
fn patch_ocr(
    existing: &str,
    ocr: OcrSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get("ocr").is_some() {
        return splice_root_string(existing, &doc, "ocr", ocr.as_str());
    }
    insert_root_string(existing, "ocr", ocr.as_str())
}

fn only_reader_and_shadow_differ(loaded: &Config, next: &Config) -> bool {
    if loaded.reader == next.reader || loaded.shadow_recognizer == next.shadow_recognizer {
        return false;
    }
    let mut same = loaded.clone();
    same.reader = next.reader;
    same.shadow_recognizer = next.shadow_recognizer;
    same == *next
}

/// Change `shadow_recognizer` and nothing else.
///
/// A root boolean is spliced in place, so comments, order, spacing, and
/// every other key stay byte for byte. A missing root key is inserted on
/// the root table, before the first `[table]` header (or at the end when
/// the file has no tables). A copy of the key inside a table is left
/// alone, and the root key is never inserted twice.
fn patch_shadow_recognizer(
    existing: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get("shadow_recognizer").is_some() {
        return splice_root_shadow_bool(existing, &doc, on);
    }
    insert_root_shadow_recognizer(existing, on)
}

fn splice_root_shadow_bool(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let wanted = if on { "true" } else { "false" };
    if let Some(item) = doc.get("shadow_recognizer")
        && let Some(value) = item.as_value()
        && value.as_bool().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        if current == "true" || current == "false" {
            let mut out = String::with_capacity(existing.len() + wanted.len());
            out.push_str(&existing[..span.start]);
            out.push_str(wanted);
            out.push_str(&existing[span.end..]);
            return Ok(out);
        }
    }
    // The root key is already there. Inserting another would be a duplicate.
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

/// Insert `shadow_recognizer` on the document root.
///
/// `toml_edit` emits root values before standard tables, so the new key
/// stays at the top level instead of falling into the last `[table]`.
fn insert_root_shadow_recognizer(
    existing: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| not_parsed(existing, &err))?;
    if doc.get("shadow_recognizer").is_some() {
        return Err(unreadable_config(
            "config.toml already has shadow_recognizer at the root, so it was not replaced"
                .to_string(),
        ));
    }
    doc.as_table_mut()
        .insert("shadow_recognizer", toml_edit::value(on));
    Ok(doc.to_string())
}

/// Change `reader` and nothing else.
///
/// A root string is spliced in place. A missing root key is inserted on the
/// root table, before the first `[table]` header, for either value: a
/// missing key means `"new"`, so turning the switch off has to write
/// `"ocr-v1"`.
fn patch_reader(
    existing: &str,
    reader: ReaderSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get("reader").is_some() {
        return splice_root_reader(existing, &doc, reader);
    }
    insert_root_reader(existing, reader)
}

fn splice_root_reader(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    reader: ReaderSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let wanted = format!("\"{}\"", reader.as_str());
    if let Some(item) = doc.get("reader")
        && let Some(value) = item.as_value()
        && value.as_str().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        if current.starts_with('"') && current.ends_with('"') {
            let mut out = String::with_capacity(existing.len() + wanted.len());
            out.push_str(&existing[..span.start]);
            out.push_str(&wanted);
            out.push_str(&existing[span.end..]);
            return Ok(out);
        }
    }
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

fn insert_root_reader(
    existing: &str,
    reader: ReaderSetting,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| not_parsed(existing, &err))?;
    if doc.get("reader").is_some() {
        return Err(unreadable_config(
            "config.toml already has reader at the root, so it was not replaced".to_string(),
        ));
    }
    doc.as_table_mut()
        .insert("reader", toml_edit::value(reader.as_str()));
    Ok(doc.to_string())
}

/// Keys the first-run guide may write. A device code is not a field here
/// and must never be added: it stays in memory for the sign-in poll.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetupDiskPatch {
    pub setup_completed: Option<bool>,
    pub sync: Option<SyncConfig>,
}

impl Config {
    /// Edit `setup_completed` and the sync token in place, the same way
    /// [`Self::text_for_save`] edits `shadow_recognizer`.
    ///
    /// Comments, key order, and every other key stay. A file that does not
    /// parse is left untouched. A missing file becomes a fresh document.
    /// The patch has no device-code field, so this write cannot store one.
    pub fn apply_setup_patch(
        path: &std::path::Path,
        patch: &SetupDiskPatch,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let existing = if path.exists() {
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };
        let toml = Self::text_for_setup_patch(existing.as_deref(), patch)?;
        if existing.as_deref() != Some(toml.as_str()) {
            atomic_write_600(path, toml.as_bytes())?;
        } else if path.exists() {
            Self::restrict_permissions(path);
        }
        Self::parse_file_contents(&toml)
    }

    /// Text [`Self::apply_setup_patch`] would write.
    pub fn text_for_setup_patch(
        existing: Option<&str>,
        patch: &SetupDiskPatch,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let Some(existing) = existing else {
            let mut fresh = Self::default();
            if let Some(done) = patch.setup_completed {
                fresh.setup_completed = done;
            }
            if let Some(sync) = &patch.sync {
                fresh.sync = Some(sync.clone());
            }
            return Ok(toml::to_string_pretty(&fresh)?);
        };
        let _loaded = toml::from_str::<Self>(existing).map_err(|err| not_parsed(existing, &err))?;
        let mut text = existing.to_string();
        if let Some(done) = patch.setup_completed {
            text = patch_root_bool(&text, "setup_completed", done)?;
        }
        if let Some(sync) = &patch.sync {
            text = patch_sync_keys(&text, sync)?;
        }
        let _check = toml::from_str::<Self>(&text).map_err(|err| not_patched(&text, &err))?;
        Ok(text)
    }
}

/// Splice a root boolean the way `shadow_recognizer` is spliced, or insert
/// it on the root when the key is missing.
fn patch_root_bool(
    existing: &str,
    key: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get(key).is_some() {
        return splice_root_bool(existing, &doc, key, on);
    }
    insert_root_bool(existing, key, on)
}

fn splice_root_bool(
    existing: &str,
    doc: &toml_edit::Document<&str>,
    key: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let wanted = if on { "true" } else { "false" };
    if let Some(item) = doc.get(key)
        && let Some(value) = item.as_value()
        && value.as_bool().is_some()
        && let Some(span) = value.span()
        && existing.is_char_boundary(span.start)
        && existing.is_char_boundary(span.end)
    {
        let current = &existing[span.start..span.end];
        if current == "true" || current == "false" {
            let mut out = String::with_capacity(existing.len() + wanted.len());
            out.push_str(&existing[..span.start]);
            out.push_str(wanted);
            out.push_str(&existing[span.end..]);
            return Ok(out);
        }
    }
    Err(unreadable_config(
        "config.toml could not be updated in place, so it was not replaced".to_string(),
    ))
}

fn insert_root_bool(
    existing: &str,
    key: &str,
    on: bool,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| not_parsed(existing, &err))?;
    if doc.get(key).is_some() {
        return Err(unreadable_config(
            "config.toml already has that key at the root, so it was not replaced".to_string(),
        ));
    }
    doc.as_table_mut().insert(key, toml_edit::value(on));
    Ok(doc.to_string())
}

fn patch_sync_keys(
    existing: &str,
    sync: &SyncConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    if doc.get("sync").and_then(|item| item.as_table()).is_none() {
        return insert_sync_table(existing, sync);
    }
    let text = upsert_table_string(existing, "sync", "server_url", &sync.server_url)?;
    upsert_table_string(&text, "sync", "token", &sync.token)
}

fn insert_sync_table(
    existing: &str,
    sync: &SyncConfig,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|err| not_parsed(existing, &err))?;
    if doc.get("sync").is_some() {
        return Err(unreadable_config(
            "config.toml already has a sync entry, so it was not replaced".to_string(),
        ));
    }
    let mut table = toml_edit::Table::new();
    table["server_url"] = toml_edit::value(sync.server_url.as_str());
    table["token"] = toml_edit::value(sync.token.as_str());
    doc["sync"] = toml_edit::Item::Table(table);
    Ok(doc.to_string())
}

fn upsert_table_string(
    existing: &str,
    table: &str,
    key: &str,
    raw: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let doc = toml_edit::Document::parse(existing).map_err(|err| not_parsed(existing, &err))?;
    let Some(tbl) = doc.get(table).and_then(|item| item.as_table()) else {
        return Err(unreadable_config(
            "config.toml could not be updated in place, so it was not replaced".to_string(),
        ));
    };
    let token = toml_edit::Value::from(raw).to_string();
    if let Some(item) = tbl.get(key) {
        let Some(value) = item.as_value().filter(|value| value.as_str().is_some()) else {
            return Err(unreadable_config(
                "config.toml could not be updated in place, so it was not replaced".to_string(),
            ));
        };
        let Some(span) = value.span() else {
            return Err(unreadable_config(
                "config.toml could not be updated in place, so it was not replaced".to_string(),
            ));
        };
        if !existing.is_char_boundary(span.start) || !existing.is_char_boundary(span.end) {
            return Err(unreadable_config(
                "config.toml could not be updated in place, so it was not replaced".to_string(),
            ));
        }
        let mut out = String::with_capacity(existing.len() + token.len());
        out.push_str(&existing[..span.start]);
        out.push_str(&token);
        out.push_str(&existing[span.end..]);
        return Ok(out);
    }
    insert_key_after_table_header(existing, table, key, &token)
}

fn insert_key_after_table_header(
    existing: &str,
    table: &str,
    key: &str,
    token: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let header = format!("[{table}]");
    let Some(idx) = existing.find(&header) else {
        return Err(unreadable_config(
            "config.toml could not be updated in place, so it was not replaced".to_string(),
        ));
    };
    let line_start = existing[..idx].rfind('\n').map(|n| n + 1).unwrap_or(0);
    if !existing[line_start..idx].trim().is_empty() {
        return Err(unreadable_config(
            "config.toml could not be updated in place, so it was not replaced".to_string(),
        ));
    }
    let after = idx + header.len();
    let line_end = existing[after..]
        .find('\n')
        .map(|n| after + n + 1)
        .unwrap_or(existing.len());
    let mut out = String::with_capacity(existing.len() + key.len() + token.len() + 8);
    out.push_str(&existing[..line_end]);
    if line_end == existing.len() && !existing.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(key);
    out.push_str(" = ");
    out.push_str(token);
    out.push('\n');
    out.push_str(&existing[line_end..]);
    Ok(out)
}
