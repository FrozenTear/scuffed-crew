# Map Registry & Frontend Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a map registry in SurrealDB, API endpoints for listing/loading maps, a map picker UI with game mode tabs, and pipeline-to-DB registration.

**Architecture:** Maps stored in SurrealDB `map` table with `MapMetadata` as a FLEXIBLE JSON object. Two public API endpoints (list + detail), admin CRUD. Dioxus map picker with game mode tabs and thumbnail grid. Pipeline CLI gains `--register` flag to upsert maps directly.

**Tech Stack:** SurrealDB v3 (existing), Axum 0.8 (existing routes pattern), Dioxus 0.7 (existing component pattern), `scuffed-api-client` (existing HTTP client)

**Design doc:** `docs/plans/2026-03-04-map-integration-design.md`

**Key patterns to follow:**
- DB queries: `crates/db/src/queries/strategies.rs` — `DbStrategy`/`with_timeout`/`RecordId::new` pattern
- Routes: `crates/server/src/routes/strategy.rs` — `AuthUser` extractor, `(StatusCode, Json<Value>)` errors
- API client: `crates/api-client/src/lib.rs` — `client.get()`, `client.post_json()` pattern
- Frontend: `crates/app/src/pages/strategy/editor.rs` — `use_resource`, async tasks, signal-based state

---

## Task 1: Add map types to `crates/types/`

**Files:**
- Modify: `crates/types/src/strategy.rs`

We need a `MapSummary` type for the list endpoint and a `SubMapSummary` for Control map sub-map info. `MapMetadata` already exists and is used for the detail endpoint.

**Step 1: Add the types**

Add after the existing `MapMetadata` impl block (after line ~450) in `crates/types/src/strategy.rs`:

```rust
/// Lightweight map info for listing/picker UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapSummary {
    pub id: String,
    pub name: String,
    pub game_mode: GameMode,
    pub thumbnail_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_maps: Option<Vec<SubMapSummary>>,
}

/// Sub-map reference for Control maps (e.g., Ilios: Lighthouse/Well/Ruins).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubMapSummary {
    pub id: String,
    pub name: String,
}
```

**Step 2: Verify it compiles**

Run: `cargo check -p scuffed-types`
Expected: compiles with no errors

**Step 3: Commit**

```bash
git add crates/types/src/strategy.rs
git commit -m "feat(types): add MapSummary and SubMapSummary types"
```

---

## Task 2: Add `map` table to DB migrations

**Files:**
- Modify: `crates/db/src/migrations.rs`

**Step 1: Add map table migration**

Add the following SQL block at the end of the `run_migrations` query string, before the closing `"#`:

```sql
        -- ================================================
        -- Maps (game map registry for strategy planner)
        -- ================================================
        DEFINE TABLE map SCHEMAFULL;
        DEFINE FIELD name ON map TYPE string;
        DEFINE FIELD game_mode ON map TYPE string
            ASSERT $value IN ['escort', 'hybrid', 'control', 'push', 'flashpoint', 'clash', 'payload_race', 'assault'];
        DEFINE FIELD thumbnail_path ON map TYPE string;
        DEFINE FIELD asset_path ON map TYPE string;
        DEFINE FIELD metadata ON map TYPE object FLEXIBLE;
        DEFINE FIELD sub_maps ON map TYPE option<array<object>> FLEXIBLE;
        DEFINE FIELD created_at ON map TYPE datetime DEFAULT time::now();
        DEFINE FIELD updated_at ON map TYPE datetime DEFAULT time::now();

        DEFINE INDEX idx_map_game_mode ON map COLUMNS game_mode;
```

**Step 2: Verify it compiles**

Run: `cargo check -p scuffed-db`
Expected: compiles

**Step 3: Commit**

```bash
git add crates/db/src/migrations.rs
git commit -m "feat(db): add map table migration for game map registry"
```

---

## Task 3: Add map DB queries

**Files:**
- Create: `crates/db/src/queries/maps.rs`
- Modify: `crates/db/src/queries/mod.rs`

**Step 1: Create the queries module**

Create `crates/db/src/queries/maps.rs`:

```rust
use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::RecordId;
use surrealdb_types::SurrealValue;

use crate::{record_id_key_to_string, with_timeout, Database, DbError, DbResult};
use scuffed_types::{
    CoordinateTransform, FloorLevel, GameMode, MapMetadata, MapSummary, SubMapSummary,
    TilePyramidInfo, WorldBounds,
};

type SurrealDatetime = surrealdb::types::Datetime;

// ─── Internal DB types (private) ───

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbMap {
    #[surreal(default)]
    id: Option<RecordId>,
    name: String,
    game_mode: String,
    thumbnail_path: String,
    asset_path: String,
    metadata: serde_json::Value,
    #[surreal(default)]
    sub_maps: Option<serde_json::Value>,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbMapSummary {
    #[surreal(default)]
    id: Option<RecordId>,
    name: String,
    game_mode: String,
    thumbnail_path: String,
    #[surreal(default)]
    sub_maps: Option<serde_json::Value>,
}

// ─── Conversion helpers ───

fn extract_id(id: Option<RecordId>) -> String {
    id.map(|r| record_id_key_to_string(r.key))
        .unwrap_or_default()
}

fn parse_game_mode(s: &str) -> GameMode {
    match s {
        "escort" => GameMode::Escort,
        "hybrid" => GameMode::Hybrid,
        "control" => GameMode::Control,
        "push" => GameMode::Push,
        "flashpoint" => GameMode::Flashpoint,
        "clash" => GameMode::Clash,
        "payload_race" => GameMode::PayloadRace,
        "assault" => GameMode::Assault,
        _ => GameMode::Control,
    }
}

fn game_mode_to_string(gm: GameMode) -> String {
    match gm {
        GameMode::Escort => "escort",
        GameMode::Hybrid => "hybrid",
        GameMode::Control => "control",
        GameMode::Push => "push",
        GameMode::Flashpoint => "flashpoint",
        GameMode::Clash => "clash",
        GameMode::PayloadRace => "payload_race",
        GameMode::Assault => "assault",
    }
    .to_string()
}

fn parse_sub_maps(val: Option<serde_json::Value>) -> Option<Vec<SubMapSummary>> {
    val.and_then(|v| serde_json::from_value(v).ok())
}

fn db_summary_to_summary(db: DbMapSummary) -> MapSummary {
    MapSummary {
        id: extract_id(db.id),
        name: db.name,
        game_mode: parse_game_mode(&db.game_mode),
        thumbnail_path: db.thumbnail_path,
        sub_maps: parse_sub_maps(db.sub_maps),
    }
}

// ─── Database methods ───

impl Database {
    /// List all maps, optionally filtered by game mode.
    pub async fn list_maps(&self, game_mode: Option<&str>) -> DbResult<Vec<MapSummary>> {
        with_timeout(async {
            let query = if let Some(gm) = game_mode {
                format!(
                    "SELECT id, name, game_mode, thumbnail_path, sub_maps \
                     FROM map WHERE game_mode = '{}' ORDER BY name",
                    gm.replace('\'', "''")
                )
            } else {
                "SELECT id, name, game_mode, thumbnail_path, sub_maps \
                 FROM map ORDER BY name"
                    .to_string()
            };

            let mut result = self.client.query(&query).await?;
            let rows: Vec<DbMapSummary> = result.take(0)?;
            Ok(rows.into_iter().map(db_summary_to_summary).collect())
        })
        .await
    }

    /// Get full map metadata by ID.
    pub async fn get_map(&self, id: &str) -> DbResult<(MapSummary, MapMetadata)> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM $rid")
                .bind(("rid", RecordId::new("map", id)))
                .await?;
            let row: Option<DbMap> = result.take(0)?;
            let db = row.ok_or_else(|| DbError::NotFound(format!("Map '{id}' not found")))?;

            let metadata: MapMetadata = serde_json::from_value(db.metadata)
                .map_err(|e| DbError::Config(format!("Failed to deserialize map metadata: {e}")))?;

            let summary = MapSummary {
                id: extract_id(db.id),
                name: db.name,
                game_mode: parse_game_mode(&db.game_mode),
                thumbnail_path: db.thumbnail_path,
                sub_maps: parse_sub_maps(db.sub_maps),
            };

            Ok((summary, metadata))
        })
        .await
    }

    /// Get sub-map metadata for a Control map.
    pub async fn get_sub_map_metadata(
        &self,
        map_id: &str,
        sub_map_id: &str,
    ) -> DbResult<MapMetadata> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT sub_maps FROM $rid")
                .bind(("rid", RecordId::new("map", map_id)))
                .await?;

            #[derive(Debug, Deserialize, SurrealValue)]
            struct SubMapsRow {
                #[surreal(default)]
                sub_maps: Option<serde_json::Value>,
            }

            let row: Option<SubMapsRow> = result.take(0)?;
            let row = row.ok_or_else(|| DbError::NotFound(format!("Map '{map_id}' not found")))?;

            let sub_maps: Vec<serde_json::Value> = row
                .sub_maps
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();

            let sub_map = sub_maps
                .into_iter()
                .find(|sm| sm.get("id").and_then(|v| v.as_str()) == Some(sub_map_id))
                .ok_or_else(|| {
                    DbError::NotFound(format!("Sub-map '{sub_map_id}' not found in '{map_id}'"))
                })?;

            let metadata = sub_map
                .get("metadata")
                .cloned()
                .ok_or_else(|| {
                    DbError::Config(format!("Sub-map '{sub_map_id}' has no metadata"))
                })?;

            serde_json::from_value(metadata)
                .map_err(|e| DbError::Config(format!("Failed to deserialize sub-map metadata: {e}")))
        })
        .await
    }

    /// Create or update (upsert) a map record. Used by pipeline --register and admin UI.
    pub async fn upsert_map(
        &self,
        id: &str,
        name: &str,
        game_mode: GameMode,
        thumbnail_path: &str,
        asset_path: &str,
        metadata: &MapMetadata,
        sub_maps: Option<&[SubMapEntry]>,
    ) -> DbResult<MapSummary> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let metadata_json = serde_json::to_value(metadata)
                .map_err(|e| DbError::Config(format!("Failed to serialize metadata: {e}")))?;
            let sub_maps_json = sub_maps
                .map(|sm| serde_json::to_value(sm))
                .transpose()
                .map_err(|e| DbError::Config(format!("Failed to serialize sub_maps: {e}")))?;

            self.client
                .query(
                    "UPDATE $rid SET \
                     name = $name, \
                     game_mode = $gm, \
                     thumbnail_path = $thumb, \
                     asset_path = $asset, \
                     metadata = $meta, \
                     sub_maps = $subs, \
                     updated_at = time::now() \
                     UPSERT",
                )
                .bind(("rid", RecordId::new("map", id)))
                .bind(("name", name.to_string()))
                .bind(("gm", game_mode_to_string(game_mode)))
                .bind(("thumb", thumbnail_path.to_string()))
                .bind(("asset", asset_path.to_string()))
                .bind(("meta", metadata_json))
                .bind(("subs", sub_maps_json))
                .await?
                .check()?;

            // Read back the created/updated record
            let (summary, _) = self.get_map(id).await?;
            Ok(summary)
        })
        .await
    }

    /// Delete a map record.
    pub async fn delete_map(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("DELETE $rid")
                .bind(("rid", RecordId::new("map", id)))
                .await?
                .check()?;
            Ok(())
        })
        .await
    }
}

/// Sub-map entry for upsert (includes metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubMapEntry {
    pub id: String,
    pub name: String,
    pub metadata: MapMetadata,
}
```

**Step 2: Register the module**

Add `pub mod maps;` to `crates/db/src/queries/mod.rs`.

**Step 3: Verify it compiles**

Run: `cargo check -p scuffed-db`
Expected: compiles

**Step 4: Commit**

```bash
git add crates/db/src/queries/maps.rs crates/db/src/queries/mod.rs
git commit -m "feat(db): add map CRUD queries with upsert support"
```

---

## Task 4: Add map API routes

**Files:**
- Create: `crates/server/src/routes/maps.rs`
- Modify: `crates/server/src/routes/mod.rs`
- Modify: `crates/server/src/main.rs`

**Step 1: Create the routes file**

Create `crates/server/src/routes/maps.rs`. Follow the exact pattern from `strategy.rs`:

```rust
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use scuffed_auth::server::extractor::AuthUser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::AppState;
use scuffed_types::{GameMode, MapMetadata, MapSummary};

pub fn map_routes(state: AppState) -> Router {
    Router::new()
        .route("/api/maps", get(list_maps))
        .route("/api/maps/{id}", get(get_map))
        .route("/api/maps/{id}/sub/{sub_id}", get(get_sub_map))
        // Admin routes
        .route(
            "/api/admin/maps",
            axum::routing::post(create_map),
        )
        .route(
            "/api/admin/maps/{id}",
            axum::routing::put(update_map).delete(delete_map),
        )
        .with_state(state)
}

// ─── Query parameters ───

#[derive(Debug, Deserialize)]
struct ListMapsQuery {
    game_mode: Option<String>,
}

// ─── Request types ───

#[derive(Debug, Deserialize)]
struct CreateMapRequest {
    id: String,
    name: String,
    game_mode: String,
    thumbnail_path: String,
    asset_path: String,
    metadata: MapMetadata,
    #[serde(default)]
    sub_maps: Option<Vec<scuffed_db::queries::maps::SubMapEntry>>,
}

#[derive(Debug, Deserialize)]
struct UpdateMapRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    metadata: Option<MapMetadata>,
    #[serde(default)]
    sub_maps: Option<Vec<scuffed_db::queries::maps::SubMapEntry>>,
}

// ─── Handlers ───

async fn list_maps(
    State(state): State<AppState>,
    Query(params): Query<ListMapsQuery>,
) -> Result<Json<Vec<MapSummary>>, (StatusCode, Json<Value>)> {
    let maps = state
        .db
        .list_maps(params.game_mode.as_deref())
        .await
        .map_err(|e| {
            tracing::error!("Failed to list maps: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to list maps"})),
            )
        })?;

    Ok(Json(maps))
}

async fn get_map(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MapMetadata>, (StatusCode, Json<Value>)> {
    let (_summary, metadata) = state.db.get_map(&id).await.map_err(|e| match &e {
        scuffed_db::DbError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Map '{id}' not found")})),
        ),
        _ => {
            tracing::error!("Failed to get map: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to get map"})),
            )
        }
    })?;

    Ok(Json(metadata))
}

async fn get_sub_map(
    State(state): State<AppState>,
    Path((id, sub_id)): Path<(String, String)>,
) -> Result<Json<MapMetadata>, (StatusCode, Json<Value>)> {
    let metadata = state
        .db
        .get_sub_map_metadata(&id, &sub_id)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Sub-map '{sub_id}' not found in '{id}'")})),
            ),
            _ => {
                tracing::error!("Failed to get sub-map: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "Failed to get sub-map"})),
                )
            }
        })?;

    Ok(Json(metadata))
}

async fn create_map(
    State(state): State<AppState>,
    _user: AuthUser<AppState>,
    Json(body): Json<CreateMapRequest>,
) -> Result<(StatusCode, Json<MapSummary>), (StatusCode, Json<Value>)> {
    // TODO: Replace AuthUser with AdminUser once admin extractor is wired to this server
    let game_mode = parse_game_mode_str(&body.game_mode)?;

    let summary = state
        .db
        .upsert_map(
            &body.id,
            &body.name,
            game_mode,
            &body.thumbnail_path,
            &body.asset_path,
            &body.metadata,
            body.sub_maps.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create map: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to create map"})),
            )
        })?;

    Ok((StatusCode::CREATED, Json(summary)))
}

async fn update_map(
    State(state): State<AppState>,
    _user: AuthUser<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateMapRequest>,
) -> Result<Json<MapSummary>, (StatusCode, Json<Value>)> {
    // Get existing map first
    let (existing_summary, existing_metadata) = state.db.get_map(&id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Map '{id}' not found")})),
        )
    })?;

    let name = body.name.as_deref().unwrap_or(&existing_summary.name);
    let metadata = body.metadata.as_ref().unwrap_or(&existing_metadata);

    let summary = state
        .db
        .upsert_map(
            &id,
            name,
            existing_summary.game_mode,
            &existing_summary.thumbnail_path,
            &format!("maps/{id}"),
            metadata,
            body.sub_maps.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to update map: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to update map"})),
            )
        })?;

    Ok(Json(summary))
}

async fn delete_map(
    State(state): State<AppState>,
    _user: AuthUser<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    state.db.delete_map(&id).await.map_err(|e| {
        tracing::error!("Failed to delete map: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to delete map"})),
        )
    })?;

    Ok(StatusCode::NO_CONTENT)
}

fn parse_game_mode_str(s: &str) -> Result<GameMode, (StatusCode, Json<Value>)> {
    match s {
        "escort" => Ok(GameMode::Escort),
        "hybrid" => Ok(GameMode::Hybrid),
        "control" => Ok(GameMode::Control),
        "push" => Ok(GameMode::Push),
        "flashpoint" => Ok(GameMode::Flashpoint),
        "clash" => Ok(GameMode::Clash),
        "payload_race" => Ok(GameMode::PayloadRace),
        "assault" => Ok(GameMode::Assault),
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid game mode: '{other}'")})),
        )),
    }
}
```

**Step 2: Register the module and routes**

In `crates/server/src/routes/mod.rs`, add:
```rust
pub mod maps;
```

In `crates/server/src/main.rs`, where routes are merged (around line 158-160), add:
```rust
.merge(routes::maps::map_routes(state.clone()))
```

**Step 3: Verify it compiles**

Run: `cargo check -p scuffed-server`
Expected: compiles (may need to adjust imports based on exact AppState/AuthUser paths)

**Step 4: Commit**

```bash
git add crates/server/src/routes/maps.rs crates/server/src/routes/mod.rs crates/server/src/main.rs
git commit -m "feat(server): add map API routes (list, detail, admin CRUD)"
```

---

## Task 5: Add asset serving

**Files:**
- Modify: `crates/server/src/main.rs`

**Step 1: Add ASSETS_DIR env var and ServeDir**

In the server's main function, after the existing route setup, add:

```rust
// Serve map assets from configurable directory
let assets_dir = std::env::var("ASSETS_DIR").unwrap_or_else(|_| "assets".to_string());
if std::path::Path::new(&assets_dir).exists() {
    tracing::info!("Serving assets from: {assets_dir}");
}
// Add to router:
.nest_service("/assets", tower_http::services::ServeDir::new(&assets_dir))
```

This should be added *before* the SPA fallback service so `/assets/` requests don't fall through to `index.html`.

**Step 2: Verify it compiles**

Run: `cargo check -p scuffed-server`
Expected: compiles

**Step 3: Commit**

```bash
git add crates/server/src/main.rs
git commit -m "feat(server): serve map assets from configurable ASSETS_DIR"
```

---

## Task 6: Add API client methods

**Files:**
- Modify: `crates/api-client/src/lib.rs`

**Step 1: Add map methods to the API client**

Add these methods to the `ApiClient` impl block:

```rust
/// List available maps, optionally filtered by game mode.
pub async fn get_maps(&self, game_mode: Option<&str>) -> Result<Vec<MapSummary>, ClientError> {
    let path = match game_mode {
        Some(gm) => format!("/api/maps?game_mode={gm}"),
        None => "/api/maps".to_string(),
    };
    self.get(&path).await
}

/// Get full metadata for a map (for the editor).
pub async fn get_map_metadata(&self, id: &str) -> Result<MapMetadata, ClientError> {
    self.get(&format!("/api/maps/{id}")).await
}

/// Get metadata for a Control map sub-map.
pub async fn get_sub_map_metadata(
    &self,
    map_id: &str,
    sub_map_id: &str,
) -> Result<MapMetadata, ClientError> {
    self.get(&format!("/api/maps/{map_id}/sub/{sub_map_id}")).await
}
```

Add required imports at the top:
```rust
use scuffed_types::{MapSummary, MapMetadata};
```

**Step 2: Verify it compiles**

Run: `cargo check -p scuffed-api-client`
Expected: compiles

**Step 3: Commit**

```bash
git add crates/api-client/src/lib.rs
git commit -m "feat(api-client): add map listing and metadata methods"
```

---

## Task 7: Map picker component

**Files:**
- Create: `crates/app/src/components/strategy/map_picker.rs`
- Modify: `crates/app/src/components/strategy/mod.rs`

**Step 1: Create the map picker component**

Create `crates/app/src/components/strategy/map_picker.rs`:

```rust
use dioxus::prelude::*;
use scuffed_types::{GameMode, MapSummary};

use crate::hooks::use_api_client;

const GAME_MODE_TABS: &[(GameMode, &str)] = &[
    (GameMode::Escort, "Escort"),
    (GameMode::Hybrid, "Hybrid"),
    (GameMode::Control, "Control"),
    (GameMode::Push, "Push"),
    (GameMode::Flashpoint, "Flashpoint"),
    (GameMode::Clash, "Clash"),
];

#[component]
pub fn MapPicker(
    on_select: EventHandler<MapSummary>,
    on_close: EventHandler<()>,
) -> Element {
    let client = use_api_client();
    let mut active_tab = use_signal(|| GameMode::Escort);

    let maps = use_resource(move || {
        let client = client.clone();
        async move {
            client.get_maps(None).await.unwrap_or_default()
        }
    });

    let filtered_maps: Vec<&MapSummary> = maps
        .read()
        .as_ref()
        .map(|all| {
            all.iter()
                .filter(|m| m.game_mode == *active_tab.read())
                .collect()
        })
        .unwrap_or_default();

    rsx! {
        div {
            class: "map-picker-overlay",
            onclick: move |_| on_close.call(()),

            div {
                class: "map-picker-dialog",
                onclick: move |evt| evt.stop_propagation(),

                // Header
                div { class: "map-picker-header",
                    h3 { "Select Map" }
                    button {
                        class: "map-picker-close",
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }

                // Game mode tabs
                div { class: "map-picker-tabs",
                    for &(mode, label) in GAME_MODE_TABS {
                        button {
                            class: if *active_tab.read() == mode { "tab active" } else { "tab" },
                            onclick: move |_| active_tab.set(mode),
                            "{label}"
                        }
                    }
                }

                // Map grid
                div { class: "map-picker-grid",
                    if maps.read().is_none() {
                        p { class: "loading", "Loading maps..." }
                    } else if filtered_maps.is_empty() {
                        p { class: "empty", "No maps available for this mode." }
                    } else {
                        for map in filtered_maps {
                            {
                                let map_clone = map.clone();
                                rsx! {
                                    button {
                                        class: "map-card",
                                        onclick: move |_| on_select.call(map_clone.clone()),
                                        img {
                                            class: "map-thumbnail",
                                            src: "{map.thumbnail_path}",
                                            alt: "{map.name}",
                                        }
                                        span { class: "map-name", "{map.name}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

**Step 2: Register the component**

In `crates/app/src/components/strategy/mod.rs`, add:
```rust
pub mod map_picker;
pub use map_picker::MapPicker;
```

**Step 3: Verify it compiles**

Run: `cargo check -p scuffed-app`
Expected: compiles (may need to adjust hook import paths)

**Step 4: Commit**

```bash
git add crates/app/src/components/strategy/map_picker.rs crates/app/src/components/strategy/mod.rs
git commit -m "feat(app): add map picker component with game mode tabs and thumbnail grid"
```

---

## Task 8: Wire map picker into editor

**Files:**
- Modify: `crates/app/src/pages/strategy/editor.rs`

This task has 4 changes:

**Step 1: Add map metadata state and fetch logic**

Near the top of the editor component (around the existing signal declarations), add:

```rust
let client = use_api_client();
let mut selected_map = use_signal::<Option<MapSummary>>(|| None);
```

When `selected_map` changes, fetch metadata:

```rust
let _meta_loader = use_resource(move || {
    let client = client.clone();
    async move {
        let map = selected_map.read();
        if let Some(map_summary) = map.as_ref() {
            match client.get_map_metadata(&map_summary.id).await {
                Ok(meta) => {
                    canvas_state.write().map_metadata = Some(meta);
                    canvas_state.write().current_map = Some(map_summary.id.clone());
                }
                Err(e) => tracing::error!("Failed to load map metadata: {e:?}"),
            }
        }
    }
});
```

**Step 2: Replace the map picker placeholder**

Replace the existing placeholder block (lines ~1041-1067) with:

```rust
if *show_map_picker.read() {
    MapPicker {
        on_select: move |map: MapSummary| {
            selected_map.set(Some(map));
            show_map_picker.set(false);
        },
        on_close: move |_| show_map_picker.set(false),
    }
}
```

Add the import at the top:
```rust
use crate::components::strategy::MapPicker;
```

**Step 3: Fix hardcoded game_mode**

Find line ~659 where `game_mode: "control".to_string()` is hardcoded. Replace with:

```rust
game_mode: selected_map
    .read()
    .as_ref()
    .map(|m| {
        match m.game_mode {
            GameMode::Escort => "escort",
            GameMode::Hybrid => "hybrid",
            GameMode::Control => "control",
            GameMode::Push => "push",
            GameMode::Flashpoint => "flashpoint",
            GameMode::Clash => "clash",
            GameMode::PayloadRace => "payload_race",
            GameMode::Assault => "assault",
        }
        .to_string()
    })
    .unwrap_or_else(|| "control".to_string()),
```

**Step 4: Add sub-map selector for Control maps**

In the editor toolbar area (near the map name display), add a sub-map dropdown:

```rust
// Show sub-map selector if this is a Control map with sub-maps
if let Some(ref map) = *selected_map.read() {
    if let Some(ref sub_maps) = map.sub_maps {
        select {
            class: "sub-map-selector",
            onchange: move |evt: Event<FormData>| {
                let sub_id = evt.value().clone();
                let map_id = selected_map.read().as_ref().map(|m| m.id.clone());
                if let Some(map_id) = map_id {
                    let client = client.clone();
                    spawn(async move {
                        match client.get_sub_map_metadata(&map_id, &sub_id).await {
                            Ok(meta) => {
                                canvas_state.write().map_metadata = Some(meta);
                                canvas_state.write().selected_sub_map = Some(sub_id);
                            }
                            Err(e) => tracing::error!("Failed to load sub-map: {e:?}"),
                        }
                    });
                }
            },
            for sub in sub_maps {
                option { value: "{sub.id}", "{sub.name}" }
            }
        }
    }
}
```

**Step 5: Verify it compiles**

Run: `cargo check -p scuffed-app`
Expected: compiles

**Step 6: Commit**

```bash
git add crates/app/src/pages/strategy/editor.rs
git commit -m "feat(app): wire map picker into editor, fix hardcoded game_mode, add sub-map selector"
```

---

## Task 9: Pipeline `--register` flag

**Files:**
- Modify: `crates/map-pipeline/Cargo.toml`
- Modify: `crates/map-pipeline/src/main.rs`

**Step 1: Add optional surrealdb dependency**

In `crates/map-pipeline/Cargo.toml`, add:

```toml
[features]
default = []
register = ["dep:surrealdb", "dep:tokio"]

[dependencies]
# ... existing deps ...
surrealdb = { version = "3", features = ["protocol-ws"], optional = true }
tokio = { version = "1", features = ["full"], optional = true }
```

**Step 2: Add register CLI flags**

Add to the `ProcessMap` subcommand in `main.rs`:

```rust
/// Register the map in SurrealDB
#[arg(long)]
register: bool,

/// SurrealDB URL (required if --register)
#[arg(long, default_value = "ws://localhost:8000")]
db_url: String,

/// SurrealDB namespace
#[arg(long, default_value = "scuffed_crew")]
db_ns: String,

/// SurrealDB database
#[arg(long, default_value = "main")]
db_db: String,

/// Map name (for registration)
#[arg(long)]
map_name: Option<String>,

/// Game mode
#[arg(long)]
game_mode: Option<String>,

/// Sub-map ID (for Control maps)
#[arg(long)]
sub_map: Option<String>,

/// Sub-map display name
#[arg(long)]
sub_map_name: Option<String>,
```

**Step 3: Add registration logic (behind feature gate)**

Add a `register_map` function gated behind `#[cfg(feature = "register")]`:

```rust
#[cfg(feature = "register")]
async fn register_in_db(
    db_url: &str,
    db_ns: &str,
    db_db: &str,
    map_id: &str,
    map_name: &str,
    game_mode: &str,
    metadata: &scuffed_types::MapMetadata,
    sub_map: Option<(&str, &str)>,
) -> anyhow::Result<()> {
    use surrealdb::engine::remote::ws::Ws;
    use surrealdb::Surreal;

    tracing::info!("Connecting to SurrealDB at {db_url}");
    let db = Surreal::new::<Ws>(db_url).await?;
    db.use_ns(db_ns).use_db(db_db).await?;

    let metadata_json = serde_json::to_value(metadata)?;
    let thumbnail_path = format!("/assets/maps/{map_id}/thumbnail.webp");
    let asset_path = format!("maps/{map_id}");

    if let Some((sub_id, sub_name)) = sub_map {
        // Upsert as sub-map of parent
        tracing::info!("Registering sub-map '{sub_id}' of map '{map_id}'");

        // Get existing sub_maps or start fresh
        let existing: Option<serde_json::Value> = db
            .query("SELECT sub_maps FROM $rid")
            .bind(("rid", surrealdb::RecordId::new("map", map_id)))
            .await?
            .take("sub_maps")?;

        let mut sub_maps: Vec<serde_json::Value> = existing
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        // Remove existing sub-map entry if present
        sub_maps.retain(|s| s.get("id").and_then(|v| v.as_str()) != Some(sub_id));

        // Add updated entry
        sub_maps.push(serde_json::json!({
            "id": sub_id,
            "name": sub_name,
            "metadata": metadata_json,
        }));

        let sub_maps_json = serde_json::to_value(&sub_maps)?;

        db.query(
            "UPDATE $rid SET \
             name = $name, \
             game_mode = $gm, \
             thumbnail_path = $thumb, \
             asset_path = $asset, \
             sub_maps = $subs, \
             updated_at = time::now() \
             UPSERT",
        )
        .bind(("rid", surrealdb::RecordId::new("map", map_id)))
        .bind(("name", map_name.to_string()))
        .bind(("gm", game_mode.to_string()))
        .bind(("thumb", thumbnail_path))
        .bind(("asset", asset_path))
        .bind(("subs", sub_maps_json))
        .await?
        .check()?;
    } else {
        // Upsert as standalone map
        tracing::info!("Registering map '{map_id}'");

        db.query(
            "UPDATE $rid SET \
             name = $name, \
             game_mode = $gm, \
             thumbnail_path = $thumb, \
             asset_path = $asset, \
             metadata = $meta, \
             updated_at = time::now() \
             UPSERT",
        )
        .bind(("rid", surrealdb::RecordId::new("map", map_id)))
        .bind(("name", map_name.to_string()))
        .bind(("gm", game_mode.to_string()))
        .bind(("thumb", thumbnail_path))
        .bind(("asset", asset_path))
        .bind(("meta", metadata_json))
        .await?
        .check()?;
    }

    tracing::info!("Map registered successfully");
    Ok(())
}

#[cfg(not(feature = "register"))]
fn register_in_db(
    _db_url: &str,
    _db_ns: &str,
    _db_db: &str,
    _map_id: &str,
    _map_name: &str,
    _game_mode: &str,
    _metadata: &scuffed_types::MapMetadata,
    _sub_map: Option<(&str, &str)>,
) -> anyhow::Result<()> {
    anyhow::bail!("Built without 'register' feature. Rebuild with: cargo build -p scuffed-map-pipeline --features register");
}
```

Then in `cmd_process_map`, after tile generation succeeds, add:

```rust
if register {
    let map_name = map_name.unwrap_or_else(|| name.to_string());
    let game_mode = game_mode.unwrap_or_else(|| "control".to_string());
    let sub = sub_map.as_deref().zip(sub_map_name.as_deref());

    #[cfg(feature = "register")]
    {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(register_in_db(
            &db_url, &db_ns, &db_db,
            &id, &map_name, &game_mode,
            &meta, sub,
        ))?;
    }

    #[cfg(not(feature = "register"))]
    anyhow::bail!("Built without 'register' feature");
}
```

**Step 4: Verify it compiles**

Run: `cargo check -p scuffed-map-pipeline`
Expected: compiles (without register feature)

Run: `cargo check -p scuffed-map-pipeline --features register`
Expected: compiles (with register feature)

**Step 5: Commit**

```bash
git add crates/map-pipeline/Cargo.toml crates/map-pipeline/src/main.rs
git commit -m "feat(map-pipeline): add --register flag for DB upsert (behind feature gate)"
```

---

## Task 10: Verify full compilation and run tests

**Step 1: Check all crates compile**

Run: `cargo check --workspace`
Expected: all crates compile

**Step 2: Run existing tests**

Run: `cargo test --workspace`
Expected: all existing tests pass (new code has no tests since it follows existing untested DB/route patterns)

**Step 3: Run map-pipeline tests specifically**

Run: `cargo test -p scuffed-map-pipeline`
Expected: all pipeline tests pass (from earlier implementation plan)

**Step 4: Run clippy**

Run: `cargo clippy --workspace`
Expected: no new warnings

**Step 5: Commit any fixes**

```bash
git add -A
git commit -m "fix: address clippy warnings from map integration"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Map types | `crates/types/src/strategy.rs` — MapSummary, SubMapSummary |
| 2 | DB migration | `crates/db/src/migrations.rs` — map table |
| 3 | DB queries | `crates/db/src/queries/maps.rs` — CRUD + upsert |
| 4 | API routes | `crates/server/src/routes/maps.rs` — list, detail, admin CRUD |
| 5 | Asset serving | `crates/server/src/main.rs` — ASSETS_DIR + ServeDir |
| 6 | API client | `crates/api-client/src/lib.rs` — get_maps, get_map_metadata |
| 7 | Map picker | `crates/app/src/components/strategy/map_picker.rs` — tabs + grid |
| 8 | Editor wiring | `crates/app/src/pages/strategy/editor.rs` — picker, game_mode, sub-maps |
| 9 | Pipeline register | `crates/map-pipeline/` — --register flag with feature gate |
| 10 | Verification | Full workspace compile + test + clippy |

**Execution order matters:** Tasks 1-3 are backend foundation (types → migration → queries). Tasks 4-6 are the API layer (routes → serving → client). Tasks 7-8 are frontend. Task 9 is pipeline extension. Task 10 is verification.
