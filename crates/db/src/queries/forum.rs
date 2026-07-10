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
    let off = create_category_raw(client, "Off-topic", "offtopic", Some("Anything else"), 2).await?;
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

async fn migrate_legacy_thread_categories(client: &Surreal<Any>) -> DbResult<()> {
    // Load boards by slug for mapping
    let mut res = client
        .query("SELECT * FROM forum_board WHERE is_active = true")
        .await?;
    let boards: Vec<DbForumBoard> = res.take(0)?;
    let slug_to_id: std::collections::HashMap<String, String> = boards
        .into_iter()
        .map(|b| {
            let id = rid_key(b.id.clone());
            (b.slug, id)
        })
        .collect();

    let map_cat = |cat: &str| -> Option<String> {
        let slug = match cat {
            "general" => "general",
            "game" => "overwatch",
            "strategy" => "ow-strategy",
            "offtopic" => "offtopic-general",
            other if !other.is_empty() => {
                // try exact slug match
                if slug_to_id.contains_key(other) {
                    other
                } else {
                    "general"
                }
            }
            _ => "general",
        };
        slug_to_id.get(slug).cloned()
    };

    let mut tres = client
        .query("SELECT * FROM forum_thread WHERE board_id = NONE OR board_id = NONE")
        .await?;
    // Surreal may use NONE; also catch empty
    let mut threads: Vec<DbForumThread> = tres.take(0).unwrap_or_default();

    // Also grab threads where board_id is missing entirely via broader select
    if threads.is_empty() {
        let mut all = client
            .query("SELECT * FROM forum_thread WHERE is_active = true")
            .await?;
        let all_t: Vec<DbForumThread> = all.take(0).unwrap_or_default();
        threads = all_t
            .into_iter()
            .filter(|t| t.board_id.as_ref().map(|s| s.is_empty()).unwrap_or(true))
            .collect();
    }

    let mut migrated = 0u32;
    for t in threads {
        if t.board_id.as_ref().map(|s| !s.is_empty()).unwrap_or(false) {
            continue;
        }
        let Some(bid) = map_cat(&t.category) else {
            continue;
        };
        let id = rid_key(t.id);
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
                .query(
                    "SELECT * FROM forum_board WHERE is_active = true AND slug = $slug LIMIT 1",
                )
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
                let parent: Option<DbForumBoard> =
                    self.client.select(("forum_board", pid)).await?;
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
            let board: Option<DbForumBoard> =
                self.client.select(("forum_board", board_id)).await?;
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

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    total: u64,
}
