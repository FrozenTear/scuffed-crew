//! Comparison key for `member.display_name`.
//!
//! The stored display name is what people see. The key is what uniqueness uses,
//! so look-alike names cannot impersonate a member. The pipeline is:
//!
//! 1. Reject Unicode control characters (general category Cc) and the line and
//!    paragraph separators U+2028 and U+2029. A name that contains one is
//!    invalid. Controls are not stripped.
//! 2. Unicode NFKC.
//! 3. Remove zero-width and other invisible format characters (the set below).
//!    This happens before whitespace collapsing so a zero-width character cannot
//!    keep two spaces apart or split a name into a different key.
//! 4. Trim and collapse the remaining Unicode whitespace to a single ASCII space.
//! 5. Unicode full casefold (`icu_casemap`, locale-independent).
//! 6. Map the small homoglyph table below onto Latin letters.
//!
//! An empty key after that pipeline is invalid. Callers store the original
//! display name unchanged and persist this key in `display_name_key`.

use std::collections::HashMap;

use serde::Deserialize;
use surrealdb::engine::any::Any;
use surrealdb::Surreal;
use surrealdb_types::{RecordId, SurrealValue};
use unicode_normalization::UnicodeNormalization;

use crate::{record_id_key_to_string, DbError, DbResult};

/// SurrealDB index that enforces one row per [`display_name_key`].
pub(crate) const DISPLAY_NAME_KEY_INDEX: &str = "member_display_name_key_idx";

/// Why a display name cannot be keyed. Both map to HTTP 400
/// `display_name_invalid`. Migration refuses to rewrite the stored name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayNameError {
    /// The raw name, or the NFKC form, contains a control or line separator.
    Control,
    /// Nothing remained after removing invisible characters and trimming.
    Empty,
}

impl DisplayNameError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "contains a control character",
            Self::Empty => "empty after normalization",
        }
    }
}

impl std::fmt::Display for DisplayNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// True when `raw` is the reserved tombstone display name.
///
/// Comparison uses the same key as uniqueness: trim, case fold, and the
/// whitespace collapse already applied by [`display_name_key`].
pub fn is_reserved_display_name(raw: &str) -> bool {
    let Ok(key) = display_name_key(raw) else {
        return false;
    };
    let Ok(reserved) = display_name_key(crate::DELETED_MEMBER_NAME) else {
        return false;
    };
    key == reserved
}

/// Build the uniqueness key for `raw`. See the module docs for the rules.
pub fn display_name_key(raw: &str) -> Result<String, DisplayNameError> {
    if raw.chars().any(is_rejected_control) {
        return Err(DisplayNameError::Control);
    }
    let nfkc: String = raw.nfkc().collect();
    if nfkc.chars().any(is_rejected_control) {
        return Err(DisplayNameError::Control);
    }
    let stripped: String = nfkc.chars().filter(|c| !is_invisible(*c)).collect();
    let collapsed = collapse_whitespace(&stripped);
    if collapsed.is_empty() {
        return Err(DisplayNameError::Empty);
    }
    let folded = casefold(&collapsed);
    let mapped: String = folded.chars().map(map_homoglyph).collect();
    if mapped.is_empty() {
        return Err(DisplayNameError::Empty);
    }
    Ok(mapped)
}

fn casefold(s: &str) -> String {
    icu_casemap::CaseMapper::new().fold_string(s).into_owned()
}

fn is_rejected_control(c: char) -> bool {
    // `char::is_control` is general category Cc (C0, DEL, C1). U+2028 and
    // U+2029 are line and paragraph separators (Zl, Zp), not Cc.
    c.is_control() || c == '\u{2028}' || c == '\u{2029}'
}

/// Invisible / format characters removed from the key.
///
/// Soft hyphen, zero-width space, ZWNJ, ZWJ, word joiner, BOM, bidi marks and
/// overrides, variation selectors, and the other default-ignorable format
/// characters that are commonly pasted into a name to impersonate someone.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200C}'
            | '\u{200D}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'
            | '\u{202B}'
            | '\u{202C}'
            | '\u{202D}'
            | '\u{202E}'
            | '\u{2060}'
            | '\u{2061}'
            | '\u{2062}'
            | '\u{2063}'
            | '\u{2064}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
            | '\u{206A}'
            | '\u{206B}'
            | '\u{206C}'
            | '\u{206D}'
            | '\u{206E}'
            | '\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'
            | '\u{FFFA}'
            | '\u{FFFB}'
    ) || ('\u{FE00}'..='\u{FE0F}').contains(&c)
        || ('\u{E0100}'..='\u{E01EF}').contains(&c)
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    let mut started = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if started {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
        started = true;
    }
    out
}

/// Latin look-alikes, applied after casefold so only lowercase code points
/// are listed. Uppercase Cyrillic and Greek fold to these first.
///
/// The table is intentionally small. It covers the confusable letters called
/// out for this check, plus the Greek letters of the same shapes.
///
/// | Code point | Name            | Maps to |
/// |------------|-----------------|---------|
/// | U+0430     | Cyrillic a      | a       |
/// | U+0435     | Cyrillic ie     | e       |
/// | U+043E     | Cyrillic o      | o       |
/// | U+0440     | Cyrillic er     | p       |
/// | U+0441     | Cyrillic es     | c       |
/// | U+0456     | Cyrillic i      | i       |
/// | U+03BF     | Greek omicron   | o       |
/// | U+03B9     | Greek iota      | i       |
/// | U+03C1     | Greek rho       | p       |
fn map_homoglyph(c: char) -> char {
    match c {
        '\u{0430}' => 'a',
        '\u{0435}' => 'e',
        '\u{043E}' => 'o',
        '\u{0440}' => 'p',
        '\u{0441}' => 'c',
        '\u{0456}' => 'i',
        '\u{03BF}' => 'o',
        '\u{03B9}' => 'i',
        '\u{03C1}' => 'p',
        other => other,
    }
}

/// True when `err` is the unique-index rejection for [`DISPLAY_NAME_KEY_INDEX`].
///
/// SurrealDB v3 reports that as `IndexExists` / "already contains", and the
/// message names the index. Matching only "already contains" would also catch
/// `member_user_idx` and `member_nostr_pubkey_idx`.
pub(crate) fn is_display_name_key_conflict(err: &surrealdb::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains(DISPLAY_NAME_KEY_INDEX)
        && (msg.contains("already") || msg.contains("unique") || msg.contains("indexexist"))
}

pub(crate) fn map_display_name_key_conflict(err: surrealdb::Error) -> DbError {
    if is_display_name_key_conflict(&err) {
        DbError::DisplayNameTaken
    } else {
        DbError::Surreal(err)
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct NameRow {
    id: RecordId,
    display_name: String,
    #[serde(default)]
    #[surreal(default)]
    display_name_key: Option<String>,
}

/// Fill `display_name_key` on every member and install the UNIQUE index.
///
/// Runs on every startup after the field exists. Existing display names are
/// never rewritten. If two rows share a key, or a stored name cannot be keyed,
/// this returns an error and writes nothing, so boot fails instead of renaming
/// someone. The UNIQUE index is created only after that check passes, which is
/// why it is not in the schema script: defining it first would treat every
/// missing key as NONE and then skip the clash check.
pub(crate) async fn backfill_display_name_keys(client: &Surreal<Any>) -> DbResult<()> {
    let mut response = client
        .query(
            "SELECT id, display_name, display_name_key FROM member \
             WHERE is_tombstone != true",
        )
        .await?
        .check()?;
    let rows: Vec<NameRow> = response.take(0)?;

    let mut pending: Vec<(RecordId, String)> = Vec::new();
    let mut by_key: HashMap<String, Vec<String>> = HashMap::new();
    let mut problems: Vec<String> = Vec::new();
    let scanned = rows.len();

    for row in rows {
        let id_str = record_id_key_to_string(row.id.key.clone());
        match display_name_key(&row.display_name) {
            Ok(key) => {
                // Rewrite only a missing or stale key. A second boot that
                // already matches does not touch the row.
                if row.display_name_key.as_deref() != Some(key.as_str()) {
                    pending.push((row.id, key.clone()));
                }
                by_key.entry(key).or_default().push(id_str);
            }
            Err(reason) => problems.push(format!(
                "member {id_str} display_name cannot be keyed ({reason}); refusing to rename"
            )),
        }
    }

    let mut clash_keys: Vec<&String> = by_key
        .iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(key, _)| key)
        .collect();
    clash_keys.sort();
    for key in clash_keys {
        let mut ids = by_key[key].clone();
        ids.sort();
        problems.push(format!(
            "display_name_key '{key}' is shared by members {}; refusing to rename",
            ids.join(", ")
        ));
    }

    if !problems.is_empty() {
        problems.sort();
        let joined = problems.join("; ");
        tracing::error!(problems = %joined, "display_name_key backfill refused");
        return Err(DbError::Conflict(joined));
    }

    for (id, key) in &pending {
        client
            .query("UPDATE $rid SET display_name_key = $key")
            .bind(("rid", id.clone()))
            .bind(("key", key.clone()))
            .await?
            .check()?;
    }

    client
        .query(format!(
            "DEFINE INDEX IF NOT EXISTS {DISPLAY_NAME_KEY_INDEX} ON member COLUMNS display_name_key UNIQUE"
        ))
        .await?
        .check()?;

    tracing::info!(
        scanned,
        updated = pending.len(),
        "display_name_key backfill complete"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::run_migrations;
    use crate::types::OrgRole;
    use crate::Database;

    fn key(raw: &str) -> String {
        display_name_key(raw).unwrap_or_else(|e| panic!("{raw:?} should key, got {e}"))
    }

    #[test]
    fn casefold_whitespace_zero_width_and_homoglyphs_share_a_key() {
        let frozen = key("Frozen");
        assert_eq!(key("FROZEN"), frozen);
        assert_eq!(key("fRoZeN"), frozen);
        assert_eq!(key("  Frozen  "), frozen);
        assert_eq!(key("Frozen\u{200B}"), frozen);
        assert_eq!(key("Fro\u{200B}zen"), frozen);
        assert_eq!(key("\u{FEFF}Frozen"), frozen);
        assert_eq!(key("Fro\u{00AD}zen"), frozen);
        // Cyrillic o (U+043E) and Greek omicron (U+03BF).
        assert_eq!(key("Fr\u{043E}zen"), frozen);
        assert_eq!(key("Fr\u{03BF}zen"), frozen);
        // Cyrillic uppercase A folds, then the homoglyph table maps it to Latin.
        assert_eq!(key("\u{0410}lice"), key("Alice"));

        assert_eq!(key("Frozen  Tear"), key(" frozen   tear "));
        assert_eq!(key("Frozen\u{2003}Tear"), key("Frozen Tear"));

        // Full casefold, not just lowercase: eszett and final sigma.
        assert_eq!(key("Straße"), key("STRASSE"));
        assert_eq!(key("\u{03C2}"), key("\u{03C3}"));
        assert_eq!(key("\u{03A3}"), key("\u{03C3}"));

        // NFKC: fullwidth letters and the fi ligature.
        assert_eq!(key("\u{FF26}rozen"), frozen);
        assert_eq!(key("\u{FB01}le"), key("file"));
    }

    #[test]
    fn homoglyph_table_maps_each_documented_letter() {
        let pairs = [
            ('\u{0430}', 'a'),
            ('\u{0435}', 'e'),
            ('\u{043E}', 'o'),
            ('\u{0440}', 'p'),
            ('\u{0441}', 'c'),
            ('\u{0456}', 'i'),
            ('\u{03BF}', 'o'),
            ('\u{03B9}', 'i'),
            ('\u{03C1}', 'p'),
        ];
        for (src, latin) in pairs {
            assert_eq!(key(&src.to_string()), latin.to_string(), "{src:?}");
        }
    }

    #[test]
    fn controls_and_empty_names_are_rejected() {
        assert_eq!(
            display_name_key("bad\nname"),
            Err(DisplayNameError::Control)
        );
        assert_eq!(
            display_name_key("bad\u{0000}name"),
            Err(DisplayNameError::Control)
        );
        assert_eq!(
            display_name_key("bad\u{2028}name"),
            Err(DisplayNameError::Control)
        );
        assert_eq!(display_name_key("   "), Err(DisplayNameError::Empty));
        assert_eq!(
            display_name_key("\u{200B}\u{FEFF}"),
            Err(DisplayNameError::Empty)
        );
        assert_eq!(display_name_key(""), Err(DisplayNameError::Empty));
    }

    async fn legacy_member(db: &Database, id: &str, user_id: &str, display_name: &str) {
        db.client
            .query(format!(
                "CREATE member:{id} SET user_id = $user_id, org_role = 'member', display_name = $name, is_active = true"
            ))
            .bind(("user_id", user_id.to_string()))
            .bind(("name", display_name.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    #[derive(Debug, Deserialize, SurrealValue)]
    struct StoredName {
        user_id: String,
        display_name: String,
        #[serde(default)]
        #[surreal(default)]
        display_name_key: Option<String>,
    }

    async fn stored_names(db: &Database) -> Vec<StoredName> {
        let mut response = db
            .client
            .query("SELECT user_id, display_name, display_name_key FROM member ORDER BY user_id")
            .await
            .unwrap()
            .check()
            .unwrap();
        response.take(0).unwrap()
    }

    #[tokio::test]
    async fn migration_backfill_writes_keys_and_blocks_a_later_clash() {
        let db = Database::connect_memory().await.unwrap();
        // First boot on an empty table installs the field and the index.
        run_migrations(&db.client).await.expect("empty boot");
        // Legacy rows: display_name is set, display_name_key is still NONE.
        // The next boot is what a deploy with existing members runs.
        legacy_member(&db, "one", "u1", "Frozen Tear").await;
        legacy_member(&db, "two", "u2", "Other").await;

        run_migrations(&db.client).await.expect("backfill");
        run_migrations(&db.client).await.expect("second boot");

        let rows = stored_names(&db).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].display_name, "Frozen Tear");
        assert_eq!(rows[0].display_name_key.as_deref(), Some("frozen tear"));
        assert_eq!(rows[1].display_name, "Other");
        assert_eq!(rows[1].display_name_key.as_deref(), Some("other"));

        let err = db
            .create_member("u3", "  FROZEN   TEAR ", OrgRole::Member)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DbError::DisplayNameTaken),
            "index should reject the clash, got {err}"
        );
        assert_eq!(stored_names(&db).await.len(), 2, "clash must not insert");
    }

    #[tokio::test]
    async fn migration_backfill_refuses_to_rename_on_clash() {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.expect("empty boot");
        legacy_member(&db, "one", "u1", "Frozen").await;
        legacy_member(&db, "two", "u2", "frozen").await;

        let err = run_migrations(&db.client).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("refusing to rename"), "{msg}");
        assert!(msg.contains("one"), "{msg}");
        assert!(msg.contains("two"), "{msg}");

        let rows = stored_names(&db).await;
        assert_eq!(rows[0].display_name, "Frozen");
        assert_eq!(rows[1].display_name, "frozen");
        assert!(
            rows.iter().all(|row| row.display_name_key.is_none()),
            "a clash must not write keys: {rows:?}"
        );
    }

    #[tokio::test]
    async fn migration_backfill_refuses_to_rename_an_invalid_name() {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.expect("empty boot");
        legacy_member(&db, "one", "u1", "Bad\nName").await;

        let err = run_migrations(&db.client).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("refusing to rename"), "{msg}");
        assert!(msg.contains("control"), "{msg}");

        let rows = stored_names(&db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_name, "Bad\nName");
        assert!(rows[0].display_name_key.is_none());
    }

    #[tokio::test]
    async fn concurrent_insert_hits_the_unique_index() {
        let db = std::sync::Arc::new(Database::connect_memory().await.unwrap());
        run_migrations(&db.client).await.unwrap();

        let first = {
            let db = db.clone();
            tokio::spawn(async move { db.create_member("u1", "Frozen", OrgRole::Member).await })
        };
        let second = {
            let db = db.clone();
            tokio::spawn(async move { db.create_member("u2", "FROZEN", OrgRole::Member).await })
        };
        let (a, b) = tokio::join!(first, second);
        let results = [a.unwrap(), b.unwrap()];
        let oks = results.iter().filter(|r| r.is_ok()).count();
        let taken = results
            .iter()
            .filter(|r| matches!(r, Err(DbError::DisplayNameTaken)))
            .count();
        assert_eq!(oks, 1, "one insert wins, results: {results:?}");
        assert_eq!(taken, 1, "the loser hits the index, results: {results:?}");

        let rows = stored_names(&db).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_name_key.as_deref(), Some("frozen"));
    }

    #[tokio::test]
    async fn self_rename_to_a_different_case_keeps_the_row() {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        let member = db
            .create_member("u1", "Frozen", OrgRole::Member)
            .await
            .unwrap();
        let updated = db
            .update_member(
                &member.id,
                Some("fRoZeN"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(updated.display_name, "fRoZeN");
        let rows = stored_names(&db).await;
        assert_eq!(rows[0].display_name_key.as_deref(), Some("frozen"));
    }
}
