//! The GUI's `live_snapshot.json`, written small and in bounded memory.
//!
//! The file used to hold every capture row (19,694 rows, 10.4 MB on a real
//! install), built from one `get_all_matches` load: about 98 MB of live heap
//! and 2.25 GB allocated per export. The GUI only ever shows one row per game
//! (`games_from_snapshot` in the UI crate keeps the newest row that passes
//! the tracked-mode filter) and the report bundle looks rows and sessions up
//! by session id, so the file now keeps, per game, the newest row and, when
//! that row is untracked, the newest tracked one. Rows with an empty session
//! id are each their own game and are all kept. Every session row and the
//! pending deletes are kept. Same JSON schema, so any GUI build reads it.

use std::io::Write as _;
use std::path::Path;

use surrealdb_types::{Datetime as SurrealDatetime, RecordId};

use super::{LocalStore, PersonalMatch, Snapshot, snapshot_path};

/// Rows read per query. Memory holds one page plus the kept rows.
pub(super) const EXPORT_PAGE: usize = 500;

/// One export at a time: the sync task and the event loop both write.
static EXPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The row `games_from_snapshot` would show for a game: the map and mode
/// filters the GUI applies before it picks the first row of a session.
pub(super) fn row_is_tracked(m: &PersonalMatch) -> bool {
    let map = m.display_map_name();
    let mode = crate::parse::uploaded_game_mode(map, &m.game_mode);
    crate::parse::stats_row_is_tracked(map, &m.game_mode)
        && crate::parse::stats_row_is_tracked(map, &mode)
}

/// Which rows of a newest-first stream the compact snapshot keeps.
#[derive(Default)]
pub(super) struct Keep {
    /// Session id to "a tracked row is kept".
    seen: std::collections::HashMap<String, bool>,
    pub(super) rows: Vec<PersonalMatch>,
}

impl Keep {
    /// Feed rows newest first.
    pub(super) fn push(&mut self, m: PersonalMatch) {
        if m.session_id.is_empty() {
            self.rows.push(m);
            return;
        }
        match self.seen.get(&m.session_id).copied() {
            None => {
                self.seen.insert(m.session_id.clone(), row_is_tracked(&m));
                self.rows.push(m);
            }
            Some(false) if row_is_tracked(&m) => {
                self.seen.insert(m.session_id.clone(), true);
                self.rows.push(m);
            }
            Some(_) => {}
        }
    }
}

/// Every `personal_match` row, newest first, a page at a time, as stored
/// (not hydrated). Keyset on played_at (idx_played_at gives the order).
/// After a full page, the rest of its last timestamp is read by equality and
/// the walk goes on strictly below it, so ties never split or repeat.
pub(super) struct RowPages {
    page: usize,
    below: Option<SurrealDatetime>,
    done: bool,
}

impl RowPages {
    pub(super) fn new(page: usize) -> Self {
        Self {
            page: page.max(1),
            below: None,
            done: false,
        }
    }

    pub(super) async fn next(
        &mut self,
        store: &LocalStore,
    ) -> Result<Option<Vec<PersonalMatch>>, Box<dyn std::error::Error + Send + Sync>> {
        if self.done {
            return Ok(None);
        }
        let page = self.page;
        // The page size is a fixed number, not input; a literal lets the
        // engine push the LIMIT into the index scan.
        let mut rows: Vec<PersonalMatch> = match self.below {
            None => store
                .db
                .query(format!(
                    "SELECT * FROM personal_match ORDER BY played_at DESC LIMIT {page}"
                ))
                .await?
                .take(0)?,
            Some(at) => store
                .db
                .query(format!(
                    "SELECT * FROM personal_match WHERE played_at < $at \
                     ORDER BY played_at DESC LIMIT {page}"
                ))
                .bind(("at", at))
                .await?
                .take(0)?,
        };
        let last = rows.last().map(|m| m.played_at);
        match last.filter(|_| rows.len() == page) {
            None => self.done = true,
            Some(last) => {
                let seen: Vec<RecordId> = rows
                    .iter()
                    .filter(|m| m.played_at == last)
                    .filter_map(|m| m.id.clone())
                    .collect();
                let tied: Vec<PersonalMatch> = store
                    .db
                    .query("SELECT * FROM personal_match WHERE played_at = $at")
                    .bind(("at", last))
                    .await?
                    .take(0)?;
                rows.extend(
                    tied.into_iter()
                        .filter(|m| !m.id.as_ref().is_some_and(|id| seen.contains(id))),
                );
                self.below = Some(last);
            }
        }
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some(rows))
    }
}

impl LocalStore {
    /// The compact snapshot (see the module doc), read in pages.
    pub async fn compact_snapshot(
        &self,
    ) -> Result<Snapshot, Box<dyn std::error::Error + Send + Sync>> {
        self.compact_snapshot_paged(EXPORT_PAGE).await
    }

    pub(super) async fn compact_snapshot_paged(
        &self,
        page: usize,
    ) -> Result<Snapshot, Box<dyn std::error::Error + Send + Sync>> {
        let mut keep = Keep::default();
        let mut pages = RowPages::new(page);
        while let Some(mut rows) = pages.next(self).await? {
            self.hydrate_rows(&mut rows).await?;
            for m in rows {
                keep.push(m);
            }
        }
        Ok(Snapshot {
            matches: keep.rows,
            sessions: self.get_all_sessions().await?,
            pending_deletes: self.get_pending_tombstones().await?,
        })
    }

    /// Write the compact snapshot to `live_snapshot.json` (tmp + rename).
    /// SurrealKV is single-process, so while the daemon holds the store the
    /// GUI reads this file. Unlike `matches.jsonl` it reflects back-filled
    /// outcomes and sync-state changes.
    pub async fn export_snapshot(
        &self,
        data_dir: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _one = EXPORT_LOCK.lock().await;
        let snapshot = self.compact_snapshot().await?;
        let tmp = data_dir.join("live_snapshot.json.tmp");
        let file = std::fs::File::create(&tmp)?;
        let mut out = std::io::BufWriter::new(file);
        serde_json::to_writer(&mut out, &snapshot)?;
        out.flush()?;
        drop(out);
        std::fs::rename(&tmp, snapshot_path(data_dir))?;
        Ok(())
    }
}
