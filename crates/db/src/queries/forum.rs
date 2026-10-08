#[cfg(test)]
use std::cell::Cell;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::engine::any::Any;
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb::Surreal;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{
    ForumBoard, ForumBoardNode, ForumCategory, ForumCategoryNode, ForumReply, ForumThread,
};
use crate::{with_timeout, Database, DbError, DbResult};

// ─── DB rows ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbForumCategory {
    #[surreal(default)]
    id: Option<RecordId>,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i64,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbForumBoard {
    #[surreal(default)]
    id: Option<RecordId>,
    category_id: String,
    parent_board_id: Option<String>,
    name: String,
    slug: String,
    description: Option<String>,
    sort_order: i64,
    is_locked: bool,
    min_role: Option<String>,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbForumThread {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    title: String,
    category: String,
    #[serde(default)]
    board_id: Option<String>,
    author_member_id: String,
    content: String,
    pinned: bool,
    locked: bool,
    nostr_event_id: Option<String>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
    is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbForumReply {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    thread_id: String,
    author_member_id: String,
    content: String,
    created_at: SurrealDatetime,
    is_active: bool,
}

fn rid_key(id: Option<RecordId>) -> String {
    id.map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string())
}

fn db_to_category(db: DbForumCategory) -> ForumCategory {
    ForumCategory {
        id: rid_key(db.id),
        name: db.name,
        slug: db.slug,
        description: db.description,
        sort_order: db.sort_order as i32,
        is_active: db.is_active,
    }
}

fn db_to_board(db: DbForumBoard) -> ForumBoard {
    ForumBoard {
        id: rid_key(db.id),
        category_id: db.category_id,
        parent_board_id: db.parent_board_id,
        name: db.name,
        slug: db.slug,
        description: db.description,
        sort_order: db.sort_order as i32,
        is_locked: db.is_locked,
        min_role: db.min_role,
        is_active: db.is_active,
    }
}

fn db_to_thread(db: DbForumThread) -> ForumThread {
    ForumThread {
        id: rid_key(db.id),
        title: db.title,
        category: db.category,
        board_id: db.board_id,
        author_member_id: db.author_member_id,
        content: db.content,
        pinned: db.pinned,
        locked: db.locked,
        nostr_event_id: db.nostr_event_id,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
        is_active: db.is_active,
    }
}

fn db_to_reply(db: DbForumReply) -> ForumReply {
    ForumReply {
        id: rid_key(db.id),
        thread_id: db.thread_id,
        author_member_id: db.author_member_id,
        content: db.content,
        created_at: db.created_at.into(),
        is_active: db.is_active,
    }
}

// ─── Seed / migrate (called from run_migrations) ────────────────────────────

/// Ensure default category/board tree exists and map legacy `category` strings → `board_id`.
pub async fn ensure_forum_hierarchy(client: &Surreal<Any>) -> DbResult<()> {
    // Count categories
    let mut count_res = client
        .query("SELECT count() AS total FROM forum_category WHERE is_active = true GROUP ALL")
        .await?;
    #[derive(Deserialize, SurrealValue)]
    struct C {
        total: u64,
    }
    let n: u64 = count_res
        .take::<Option<C>>(0)?
        .map(|c| c.total)
        .unwrap_or(0);

    if n == 0 {
        seed_default_tree(client).await?;
    }

    migrate_legacy_thread_categories(client).await?;
    Ok(())
}

async fn seed_default_tree(client: &Surreal<Any>) -> DbResult<()> {
    // Org
    let org = create_category_raw(client, "Org", "org", Some("Organization discussion"), 0).await?;
    create_board_raw(
        client,
        &org,
        None,
        "General",
        "general",
        Some("General discussion"),
        0,
    )
    .await?;
    create_board_raw(
        client,
        &org,
        None,
        "Announcements",
        "announcements",
        Some("Official posts"),
        1,
    )
    .await?;

    // Games
    let games =
        create_category_raw(client, "Games", "games", Some("Game-related discussion"), 1).await?;
    let ow = create_board_raw(
        client,
        &games,
        None,
        "Overwatch",
        "overwatch",
        Some("Overwatch discussion"),
        0,
    )
    .await?;
    create_board_raw(
        client,
        &games,
        Some(&ow),
        "Strategy",
        "ow-strategy",
        Some("Comps, VODs, strats"),
        0,
    )
    .await?;
    create_board_raw(
        client,
        &games,
        None,
        "LFG",
        "lfg",
        Some("Looking for group"),
        1,
    )
    .await?;

    // Off-topic
    let off =
        create_category_raw(client, "Off-topic", "offtopic", Some("Anything else"), 2).await?;
    create_board_raw(
        client,
        &off,
        None,
        "General",
        "offtopic-general",
        Some("Off-topic chatter"),
        0,
    )
    .await?;

    tracing::info!("Seeded default forum category/board tree");
    Ok(())
}

async fn create_category_raw(
    client: &Surreal<Any>,
    name: &str,
    slug: &str,
    description: Option<&str>,
    sort_order: i64,
) -> DbResult<String> {
    let row = DbForumCategory {
        id: None,
        name: name.to_string(),
        slug: slug.to_string(),
        description: description.map(|s| s.to_string()),
        sort_order,
        is_active: true,
    };
    let created: Option<DbForumCategory> = client.create("forum_category").content(row).await?;
    Ok(rid_key(created.and_then(|c| c.id)))
}

async fn create_board_raw(
    client: &Surreal<Any>,
    category_id: &str,
    parent_board_id: Option<&str>,
    name: &str,
    slug: &str,
    description: Option<&str>,
    sort_order: i64,
) -> DbResult<String> {
    let row = DbForumBoard {
        id: None,
        category_id: category_id.to_string(),
        parent_board_id: parent_board_id.map(|s| s.to_string()),
        name: name.to_string(),
        slug: slug.to_string(),
        description: description.map(|s| s.to_string()),
        sort_order,
        is_locked: false,
        min_role: None,
        is_active: true,
    };
    let created: Option<DbForumBoard> = client.create("forum_board").content(row).await?;
    Ok(rid_key(created.and_then(|c| c.id)))
}

/// Threads fetched per response while backfilling a missing `board_id`.
/// Steady-state boot does not read `forum_thread` at all.
const FORUM_LEGACY_THREAD_BATCH: u32 = 500;

#[cfg(test)]
thread_local! {
    static FORUM_THREAD_ROW_SELECTS: Cell<u64> = const { Cell::new(0) };
    static FORUM_THREAD_BATCH_OVERRIDE: Cell<Option<u32>> = const { Cell::new(None) };
}

fn note_forum_thread_row_select() {
    #[cfg(test)]
    FORUM_THREAD_ROW_SELECTS.with(|n| n.set(n.get().saturating_add(1)));
}

fn forum_legacy_batch_size() -> u32 {
    #[cfg(test)]
    {
        if let Some(n) = FORUM_THREAD_BATCH_OVERRIDE.with(|c| c.get()) {
            return n.max(1);
        }
    }
    FORUM_LEGACY_THREAD_BATCH
}

#[cfg(test)]
struct ForumBatchGuard;

#[cfg(test)]
impl Drop for ForumBatchGuard {
    fn drop(&mut self) {
        FORUM_THREAD_BATCH_OVERRIDE.with(|c| c.set(None));
    }
}

#[cfg(test)]
#[must_use]
fn set_forum_legacy_batch_size(n: u32) -> ForumBatchGuard {
    FORUM_THREAD_BATCH_OVERRIDE.with(|c| c.set(Some(n)));
    ForumBatchGuard
}

#[cfg(test)]
fn reset_forum_thread_row_selects() {
    FORUM_THREAD_ROW_SELECTS.with(|n| n.set(0));
}

#[cfg(test)]
fn forum_thread_row_selects() -> u64 {
    FORUM_THREAD_ROW_SELECTS.with(|n| n.get())
}

#[derive(Debug, Deserialize, SurrealValue)]
struct BoardSlug {
    #[surreal(default)]
    id: Option<RecordId>,
    slug: String,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct LegacyThreadPage {
    #[surreal(default)]
    id: Option<RecordId>,
    #[serde(default)]
    category: String,
    #[serde(default)]
    board_id: Option<String>,
}

fn legacy_board_id(
    cat: &str,
    slug_to_id: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let slug = match cat {
        "general" => "general",
        "game" => "overwatch",
        "strategy" => "ow-strategy",
        "offtopic" => "offtopic-general",
        other if !other.is_empty() && slug_to_id.contains_key(other) => other,
        _ => "general",
    };
    slug_to_id.get(slug).cloned()
}

async fn count_threads_missing_board(client: &Surreal<Any>) -> DbResult<u64> {
    let mut count_res = client
        .query(
            "SELECT count() AS total FROM forum_thread \
             WHERE board_id IS NONE OR board_id = '' GROUP ALL",
        )
        .await?;
    #[derive(Deserialize, SurrealValue)]
    struct C {
        total: u64,
    }
    Ok(count_res
        .take::<Option<C>>(0)?
        .map(|c| c.total)
        .unwrap_or(0))
}

async fn migrate_legacy_thread_categories(client: &Surreal<Any>) -> DbResult<()> {
    // Missing `board_id` is the only work. A count of zero (the steady state)
    // returns before any thread row is loaded. The old fallback was
    // `SELECT * FROM forum_thread WHERE is_active = true` on every boot,
    // because `board_id = NONE` does not match NONE in SurrealDB.
    if count_threads_missing_board(client).await? == 0 {
        return Ok(());
    }

    let mut res = client
        .query("SELECT id, slug FROM forum_board WHERE is_active = true")
        .await?;
    let boards: Vec<BoardSlug> = res.take(0)?;
    let slug_to_id: std::collections::HashMap<String, String> = boards
        .into_iter()
        .map(|b| (b.slug, rid_key(b.id)))
        .collect();

    let batch = forum_legacy_batch_size();
    let mut cursor: Option<RecordId> = None;
    let mut migrated = 0u32;
    loop {
        note_forum_thread_row_select();
        let mut page_res = if let Some(cursor_id) = cursor.clone() {
            client
                .query(
                    "SELECT id, category, board_id FROM forum_thread \
                     WHERE (board_id IS NONE OR board_id = '') AND id > $cursor \
                     ORDER BY id LIMIT $lim",
                )
                .bind(("cursor", cursor_id))
                .bind(("lim", batch))
                .await?
                .check()?
        } else {
            client
                .query(
                    "SELECT id, category, board_id FROM forum_thread \
                     WHERE board_id IS NONE OR board_id = '' \
                     ORDER BY id LIMIT $lim",
                )
                .bind(("lim", batch))
                .await?
                .check()?
        };
        let threads: Vec<LegacyThreadPage> = page_res.take(0)?;
        if threads.is_empty() {
            break;
        }
        let page_len = threads.len() as u32;
        let last_id = threads.last().and_then(|t| t.id.clone());
        for thread in threads {
            if thread
                .board_id
                .as_ref()
                .is_some_and(|board| !board.is_empty())
            {
                continue;
            }
            let Some(bid) = legacy_board_id(&thread.category, &slug_to_id) else {
                continue;
            };
            let id = rid_key(thread.id);
            if id == "unknown" {
                continue;
            }
            client
                .query("UPDATE $rid SET board_id = $bid")
                .bind(("rid", RecordId::new("forum_thread", id.as_str())))
                .bind(("bid", bid))
                .await?;
            migrated += 1;
        }
        let Some(last_id) = last_id else {
            break;
        };
        if cursor.as_ref() == Some(&last_id) {
            break;
        }
        cursor = Some(last_id);
        if page_len < batch {
            break;
        }
    }
    if migrated > 0 {
        tracing::info!("Migrated {migrated} forum threads to board_id");
    }
    Ok(())
}

// ─── Database API ───────────────────────────────────────────────────────────

impl Database {
    pub async fn list_forum_tree(&self) -> DbResult<Vec<ForumCategoryNode>> {
        with_timeout(async {
            let mut cres = self
                .client
                .query(
                    "SELECT * FROM forum_category WHERE is_active = true ORDER BY sort_order ASC",
                )
                .await?;
            let cats: Vec<DbForumCategory> = cres.take(0)?;

            let mut bres = self
                .client
                .query("SELECT * FROM forum_board WHERE is_active = true ORDER BY sort_order ASC")
                .await?;
            let boards: Vec<DbForumBoard> = bres.take(0)?;
            let boards: Vec<ForumBoard> = boards.into_iter().map(db_to_board).collect();

            let mut out = Vec::new();
            for cat in cats {
                let cat = db_to_category(cat);
                let top: Vec<ForumBoard> = boards
                    .iter()
                    .filter(|b| b.category_id == cat.id && b.parent_board_id.is_none())
                    .cloned()
                    .collect();
                let mut board_nodes = Vec::new();
                for b in top {
                    let subs: Vec<ForumBoard> = boards
                        .iter()
                        .filter(|s| s.parent_board_id.as_deref() == Some(b.id.as_str()))
                        .cloned()
                        .collect();
                    let thread_count = self.count_threads_on_board(&b.id).await.unwrap_or(0);
                    board_nodes.push(ForumBoardNode {
                        board: b,
                        sub_boards: subs,
                        thread_count,
                    });
                }
                out.push(ForumCategoryNode {
                    category: cat,
                    boards: board_nodes,
                });
            }
            Ok(out)
        })
        .await
    }

    async fn count_threads_on_board(&self, board_id: &str) -> DbResult<u64> {
        let mut result = self
            .client
            .query(
                "SELECT count() AS total FROM forum_thread \
                 WHERE is_active = true AND board_id = $bid GROUP ALL",
            )
            .bind(("bid", board_id.to_string()))
            .await?;
        let row: Option<CountRow> = result.take(0)?;
        Ok(row.map(|r| r.total).unwrap_or(0))
    }

    pub async fn get_forum_board_by_slug(&self, slug: &str) -> DbResult<Option<ForumBoard>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM forum_board WHERE is_active = true AND slug = $slug LIMIT 1")
                .bind(("slug", slug.to_string()))
                .await?;
            let rows: Vec<DbForumBoard> = result.take(0)?;
            Ok(rows.into_iter().next().map(db_to_board))
        })
        .await
    }

    pub async fn get_forum_board(&self, id: &str) -> DbResult<Option<ForumBoard>> {
        with_timeout(async {
            let db: Option<DbForumBoard> = self.client.select(("forum_board", id)).await?;
            Ok(db.map(db_to_board))
        })
        .await
    }

    pub async fn get_forum_category(&self, id: &str) -> DbResult<Option<ForumCategory>> {
        with_timeout(async {
            let db: Option<DbForumCategory> = self.client.select(("forum_category", id)).await?;
            Ok(db.map(db_to_category))
        })
        .await
    }

    pub async fn create_forum_category(
        &self,
        name: &str,
        slug: &str,
        description: Option<&str>,
        sort_order: i32,
    ) -> DbResult<ForumCategory> {
        with_timeout(async {
            let row = DbForumCategory {
                id: None,
                name: name.to_string(),
                slug: slug.to_string(),
                description: description.map(|s| s.to_string()),
                sort_order: sort_order as i64,
                is_active: true,
            };
            let created: Option<DbForumCategory> =
                self.client.create("forum_category").content(row).await?;
            Ok(db_to_category(created.ok_or_else(|| {
                DbError::NotFound("Failed to create forum category".into())
            })?))
        })
        .await
    }

    pub async fn update_forum_category(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        sort_order: Option<i32>,
        is_active: Option<bool>,
    ) -> DbResult<ForumCategory> {
        with_timeout(async {
            let mut db: DbForumCategory = self
                .client
                .select(("forum_category", id))
                .await?
                .ok_or_else(|| DbError::NotFound(format!("category {id}")))?;
            if let Some(n) = name {
                db.name = n.to_string();
            }
            if let Some(d) = description {
                db.description = d.map(|s| s.to_string());
            }
            if let Some(s) = sort_order {
                db.sort_order = s as i64;
            }
            if let Some(a) = is_active {
                db.is_active = a;
            }
            let updated: Option<DbForumCategory> = self
                .client
                .update(("forum_category", id))
                .content(db)
                .await?;
            Ok(db_to_category(updated.ok_or_else(|| {
                DbError::NotFound("update category failed".into())
            })?))
        })
        .await
    }

    pub async fn create_forum_board(
        &self,
        category_id: &str,
        parent_board_id: Option<&str>,
        name: &str,
        slug: &str,
        description: Option<&str>,
        sort_order: i32,
    ) -> DbResult<ForumBoard> {
        with_timeout(async {
            if let Some(pid) = parent_board_id {
                let parent: Option<DbForumBoard> = self.client.select(("forum_board", pid)).await?;
                let parent = parent.ok_or_else(|| DbError::NotFound("parent board".into()))?;
                if parent.parent_board_id.is_some() {
                    return Err(DbError::Config(
                        "sub-boards cannot have children (max depth 1)".into(),
                    ));
                }
            }
            let row = DbForumBoard {
                id: None,
                category_id: category_id.to_string(),
                parent_board_id: parent_board_id.map(|s| s.to_string()),
                name: name.to_string(),
                slug: slug.to_string(),
                description: description.map(|s| s.to_string()),
                sort_order: sort_order as i64,
                is_locked: false,
                min_role: None,
                is_active: true,
            };
            let created: Option<DbForumBoard> =
                self.client.create("forum_board").content(row).await?;
            Ok(db_to_board(created.ok_or_else(|| {
                DbError::NotFound("Failed to create forum board".into())
            })?))
        })
        .await
    }

    pub async fn update_forum_board(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        sort_order: Option<i32>,
        is_locked: Option<bool>,
        is_active: Option<bool>,
    ) -> DbResult<ForumBoard> {
        with_timeout(async {
            let mut db: DbForumBoard = self
                .client
                .select(("forum_board", id))
                .await?
                .ok_or_else(|| DbError::NotFound(format!("board {id}")))?;
            if let Some(n) = name {
                db.name = n.to_string();
            }
            if let Some(d) = description {
                db.description = d.map(|s| s.to_string());
            }
            if let Some(s) = sort_order {
                db.sort_order = s as i64;
            }
            if let Some(l) = is_locked {
                db.is_locked = l;
            }
            if let Some(a) = is_active {
                db.is_active = a;
            }
            let updated: Option<DbForumBoard> =
                self.client.update(("forum_board", id)).content(db).await?;
            Ok(db_to_board(updated.ok_or_else(|| {
                DbError::NotFound("update board failed".into())
            })?))
        })
        .await
    }

    /// Create a new forum thread on a board.
    pub async fn create_forum_thread(
        &self,
        title: &str,
        board_id: &str,
        author_member_id: &str,
        content: &str,
    ) -> DbResult<ForumThread> {
        with_timeout(async {
            let board: Option<DbForumBoard> = self.client.select(("forum_board", board_id)).await?;
            let board =
                board.ok_or_else(|| DbError::NotFound(format!("board {board_id} not found")))?;
            if board.is_locked {
                return Err(DbError::Config("board is locked".into()));
            }
            let now = SurrealDatetime::from(Utc::now());
            let db_thread = DbForumThread {
                id: None,
                title: title.to_string(),
                category: board.slug.clone(),
                board_id: Some(board_id.to_string()),
                author_member_id: author_member_id.to_string(),
                content: content.to_string(),
                pinned: false,
                locked: false,
                nostr_event_id: None,
                created_at: now,
                updated_at: now,
                is_active: true,
            };
            let created: Option<DbForumThread> = self
                .client
                .create("forum_thread")
                .content(db_thread)
                .await?;
            Ok(db_to_thread(created.ok_or_else(|| {
                DbError::NotFound("Failed to create forum thread".into())
            })?))
        })
        .await
    }

    /// List active forum threads by board_id, or legacy category string.
    pub async fn list_forum_threads(
        &self,
        board_id: Option<&str>,
        category: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<ForumThread>> {
        with_timeout(async {
            let mut result = if let Some(bid) = board_id {
                self.client
                    .query(
                        "SELECT * FROM forum_thread WHERE is_active = true AND board_id = $bid \
                         ORDER BY pinned DESC, updated_at DESC LIMIT $lim START $off",
                    )
                    .bind(("bid", bid.to_string()))
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?
            } else if let Some(cat) = category {
                self.client
                    .query(
                        "SELECT * FROM forum_thread WHERE is_active = true AND category = $cat \
                         ORDER BY pinned DESC, updated_at DESC LIMIT $lim START $off",
                    )
                    .bind(("cat", cat.to_string()))
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?
            } else {
                self.client
                    .query(
                        "SELECT * FROM forum_thread WHERE is_active = true \
                         ORDER BY pinned DESC, updated_at DESC LIMIT $lim START $off",
                    )
                    .bind(("lim", limit))
                    .bind(("off", offset))
                    .await?
            };
            let threads: Vec<DbForumThread> = result.take(0)?;
            Ok(threads.into_iter().map(db_to_thread).collect())
        })
        .await
    }

    pub async fn get_forum_thread(&self, id: &str) -> DbResult<ForumThread> {
        with_timeout(async {
            let db: Option<DbForumThread> = self.client.select(("forum_thread", id)).await?;
            Ok(db_to_thread(db.ok_or_else(|| {
                DbError::NotFound(format!("Forum thread {id} not found"))
            })?))
        })
        .await
    }

    pub async fn create_forum_reply(
        &self,
        thread_id: &str,
        author_member_id: &str,
        content: &str,
    ) -> DbResult<ForumReply> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db_reply = DbForumReply {
                id: None,
                thread_id: thread_id.to_string(),
                author_member_id: author_member_id.to_string(),
                content: content.to_string(),
                created_at: now,
                is_active: true,
            };
            let created: Option<DbForumReply> =
                self.client.create("forum_reply").content(db_reply).await?;
            // bump thread updated_at
            let _ = self
                .client
                .query("UPDATE $rid SET updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", thread_id)))
                .await;
            Ok(db_to_reply(created.ok_or_else(|| {
                DbError::NotFound("Failed to create forum reply".into())
            })?))
        })
        .await
    }

    pub async fn list_forum_replies(
        &self,
        thread_id: &str,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<ForumReply>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT * FROM forum_reply WHERE is_active = true AND thread_id = $tid \
                     ORDER BY created_at ASC LIMIT $lim START $off",
                )
                .bind(("tid", thread_id.to_string()))
                .bind(("lim", limit))
                .bind(("off", offset))
                .await?;
            let replies: Vec<DbForumReply> = result.take(0)?;
            Ok(replies.into_iter().map(db_to_reply).collect())
        })
        .await
    }

    pub async fn pin_forum_thread(&self, id: &str, pinned: bool) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET pinned = $pinned, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .bind(("pinned", pinned))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn lock_forum_thread(&self, id: &str, locked: bool) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET locked = $locked, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .bind(("locked", locked))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn update_thread_nostr_event_id(
        &self,
        id: &str,
        nostr_event_id: &str,
    ) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET nostr_event_id = $eid, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .bind(("eid", nostr_event_id.to_string()))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn deactivate_forum_thread(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false, updated_at = time::now()")
                .bind(("rid", RecordId::new("forum_thread", id)))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn deactivate_forum_reply(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("UPDATE $rid SET is_active = false")
                .bind(("rid", RecordId::new("forum_reply", id)))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn count_forum_replies(&self, thread_id: &str) -> DbResult<u64> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT count() as total FROM forum_reply \
                     WHERE is_active = true AND thread_id = $tid GROUP ALL",
                )
                .bind(("tid", thread_id.to_string()))
                .await?;
            let row: Option<CountRow> = result.take(0)?;
            Ok(row.map(|r| r.total).unwrap_or(0))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::ensure_forum_hierarchy;
    use crate::migrations::run_migrations;
    use crate::Database;

    async fn test_db() -> Database {
        let db = Database::connect_memory().await.unwrap();
        run_migrations(&db.client).await.unwrap();
        db
    }

    async fn board_key(db: &Database, slug: &str) -> String {
        use serde::Deserialize;
        use surrealdb_types::SurrealValue;
        #[derive(Deserialize, SurrealValue)]
        struct Row {
            id: surrealdb_types::RecordId,
        }
        let mut res = db
            .client
            .query("SELECT id FROM forum_board WHERE slug = $slug LIMIT 1")
            .bind(("slug", slug.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
        let rows: Vec<Row> = res.take(0).unwrap();
        super::rid_key(rows.into_iter().next().map(|row| row.id))
    }

    #[tokio::test]
    async fn forum_boot_skips_thread_scan_when_boards_are_assigned() {
        let db = test_db().await;
        let general = board_key(&db, "general").await;
        db.create_forum_thread("hello", &general, "m1", "body")
            .await
            .unwrap();

        super::reset_forum_thread_row_selects();
        ensure_forum_hierarchy(&db.client).await.unwrap();
        assert_eq!(
            super::forum_thread_row_selects(),
            0,
            "threads that already have a board_id must not be loaded at boot"
        );
    }

    #[tokio::test]
    async fn legacy_threads_gain_board_ids_in_pages() {
        let db = test_db().await;
        let _batch = super::set_forum_legacy_batch_size(2);
        let cases = [
            ("general", "general", true),
            ("game", "overwatch", true),
            ("strategy", "ow-strategy", true),
            ("offtopic", "offtopic-general", true),
            ("lfg", "lfg", false),
            ("", "general", true),
        ];
        for (i, (cat, _slug, active)) in cases.iter().enumerate() {
            db.client
                .query(
                    "CREATE forum_thread SET title = $title, category = $cat, \
                     author_member_id = 'm1', content = 'x', board_id = NONE, \
                     is_active = $active",
                )
                .bind(("title", format!("legacy-{i}")))
                .bind(("cat", (*cat).to_string()))
                .bind(("active", *active))
                .await
                .unwrap()
                .check()
                .unwrap();
        }
        // Empty string is a second spelling of "missing".
        db.client
            .query(
                "CREATE forum_thread SET title = 'blank-board', category = 'strategy', \
                 author_member_id = 'm1', content = 'x', board_id = '', is_active = true",
            )
            .await
            .unwrap()
            .check()
            .unwrap();

        super::reset_forum_thread_row_selects();
        ensure_forum_hierarchy(&db.client).await.unwrap();
        let selects = super::forum_thread_row_selects();
        assert!(
            selects >= 3,
            "legacy threads must be loaded in pages, got {selects} selects"
        );

        use serde::Deserialize;
        use surrealdb_types::SurrealValue;
        #[derive(Deserialize, SurrealValue)]
        struct ThreadBoard {
            title: String,
            category: String,
            board_id: Option<String>,
        }
        let mut res = db
            .client
            .query("SELECT title, category, board_id FROM forum_thread")
            .await
            .unwrap()
            .check()
            .unwrap();
        let threads: Vec<ThreadBoard> = res.take(0).unwrap();
        assert_eq!(threads.len(), cases.len() + 1);
        for thread in threads {
            let expected_slug = if thread.title == "blank-board" {
                "ow-strategy"
            } else {
                let (_cat, slug, _) = cases
                    .iter()
                    .find(|(cat, _, _)| thread.category == *cat && thread.title != "blank-board")
                    .copied()
                    .unwrap_or_else(|| panic!("unexpected thread {}", thread.title));
                // Disambiguate the two rows that map through "general".
                if thread.category.is_empty() {
                    "general"
                } else {
                    slug
                }
            };
            let expected = board_key(&db, expected_slug).await;
            assert_eq!(
                thread.board_id.as_deref(),
                Some(expected.as_str()),
                "title {} category {:?}",
                thread.title,
                thread.category
            );
        }

        super::reset_forum_thread_row_selects();
        ensure_forum_hierarchy(&db.client).await.unwrap();
        assert_eq!(
            super::forum_thread_row_selects(),
            0,
            "once every thread has a board_id, boot must not read forum_thread"
        );
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    total: u64,
}
