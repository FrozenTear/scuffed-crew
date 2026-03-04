# Map Registry & Frontend Integration Design

**Date:** 2026-03-04
**Status:** Approved
**Depends on:** `2026-03-04-map-pipeline-design.md` (map asset generation)

## Goal

Bridge the gap between the map rendering pipeline (generates tile assets) and the frontend (consumes them). Provide a map registry in SurrealDB, API endpoints for listing/loading maps, a map picker UI, and pipeline-to-DB registration.

## Current State

- **Pipeline** generates: `assets/maps/{id}/metadata.json`, `thumbnail.webp`, `floors/{floor}/{z}/{x}/{y}.webp`
- **Frontend** has `TileManager`, `MapMetadata`, `FloorLevel`, `CoordinateTransform` types — all ready to consume pipeline output
- **Missing:** No map registry, no metadata loading, no map picker UI, no asset serving endpoint, GameMode hardcoded as "control"

## Database Schema

New `map` table in SurrealDB:

```sql
DEFINE TABLE map SCHEMAFULL;
DEFINE FIELD name ON map TYPE string;
DEFINE FIELD game_mode ON map TYPE string;
DEFINE FIELD thumbnail_path ON map TYPE string;
DEFINE FIELD asset_path ON map TYPE string;
DEFINE FIELD metadata ON map TYPE object FLEXIBLE;
DEFINE FIELD sub_maps ON map TYPE option<array<object>> FLEXIBLE;
DEFINE FIELD created_at ON map TYPE datetime DEFAULT time::now();
DEFINE FIELD updated_at ON map TYPE datetime DEFAULT time::now();
DEFINE INDEX idx_map_game_mode ON map COLUMNS game_mode;
```

The `metadata` field stores the entire `MapMetadata` struct as a JSON object (transform, bounds, floors, tile pyramid, health packs, connections). This avoids needing separate tables for floors/health packs — the data is always loaded as a unit.

`sub_maps` is an optional array for Control maps. Each sub-map entry contains `{id, name, metadata}` where metadata is a full `MapMetadata` object for that sub-map.

## API Endpoints

### Public (any authenticated user)

**List maps:**
```
GET /api/maps
GET /api/maps?game_mode=escort
```

Response:
```json
[
  {
    "id": "kings_row",
    "name": "King's Row",
    "game_mode": "hybrid",
    "thumbnail_path": "/assets/maps/kings_row/thumbnail.webp",
    "sub_maps": null
  },
  {
    "id": "ilios",
    "name": "Ilios",
    "game_mode": "control",
    "thumbnail_path": "/assets/maps/ilios/thumbnail.webp",
    "sub_maps": [
      {"id": "lighthouse", "name": "Lighthouse"},
      {"id": "well", "name": "Well"},
      {"id": "ruins", "name": "Ruins"}
    ]
  }
]
```

**Get map metadata (for editor):**
```
GET /api/maps/{id}
GET /api/maps/{id}/sub/{sub_id}
```

Returns full `MapMetadata` struct.

### Admin (AdminUser extractor)

```
POST   /api/admin/maps           -- Create/register a map
PUT    /api/admin/maps/{id}      -- Update map metadata
DELETE /api/admin/maps/{id}      -- Remove a map
```

## Frontend Integration

### API Client (`crates/api-client/`)

New methods:
- `get_maps(game_mode: Option<GameMode>) -> Vec<MapSummary>`
- `get_map_metadata(id: &str) -> MapMetadata`
- `get_sub_map_metadata(id: &str, sub_id: &str) -> MapMetadata`

### Map Picker Component

New component: `crates/app/src/components/strategy/map_picker.rs`

- Modal overlay with game mode tabs: Escort | Hybrid | Control | Push | Flashpoint | Clash
- Each tab shows a grid of map thumbnail cards
- `use_resource` to fetch map list (cached)
- Clicking a map selects it and closes the modal
- For Control maps: selecting the parent map sets `current_map`. Sub-map is chosen later in the editor toolbar.

### Editor Changes (`crates/app/src/pages/strategy/editor.rs`)

- When a map is selected via the picker, fetch `MapMetadata` via API
- Set `canvas_state.map_metadata` with the result
- Fix hardcoded `game_mode: "control"` to derive from selected map
- For Control maps: add a sub-map selector dropdown in the editor toolbar. Switching sub-maps re-fetches metadata and resets the tile manager.

### No Changes Needed

- **TileManager** (`tile_manager.rs`) — URL pattern already matches pipeline output
- **CanvasRenderer** (`renderer.rs`) — no map-specific code
- **MapMetadata types** (`strategy.rs`) — already designed for this use case

## Pipeline DB Registration

The Rust CLI gains optional `--register` flag:

```sh
cargo xtask process-map \
  --glb kings_row.glb \
  --images ./out/floors/ \
  --output assets/maps/kings_row/ \
  --register \
  --db-url ws://localhost:8000 \
  --map-name "King's Row" \
  --game-mode hybrid
```

When `--register` is passed:
1. Build `MapMetadata` as normal
2. Connect to SurrealDB
3. Upsert `map` record with id, name, game_mode, metadata, thumbnail_path, asset_path
4. Print confirmation

Without `--register`, the CLI generates files only (no DB dependency).

For Control sub-maps:
```sh
cargo xtask process-map --glb ilios_lighthouse.glb ... --register \
  --map-name "Ilios" --game-mode control \
  --sub-map lighthouse --sub-map-name "Lighthouse"
```

This upserts the parent map record and adds/updates the sub-map entry in the `sub_maps` array.

## Asset Serving

Add `ASSETS_DIR` env var (default: `assets/`). Server mounts it:

```rust
.nest_service("/assets", ServeDir::new(assets_dir))
```

Separate from the SPA `dist/` directory. Map tiles live on disk at `{ASSETS_DIR}/maps/{id}/floors/...`.

In production, a reverse proxy (nginx) can serve `/assets/` directly for better performance.

## Data Flow Summary

```
Pipeline generates tiles + metadata.json
    |
    | --register flag
    v
SurrealDB map table <-- Admin CRUD
    |
    | GET /api/maps
    v
Map Picker UI (tabs + thumbnail grid)
    |
    | user selects map
    | GET /api/maps/{id}
    v
Editor loads MapMetadata
    |
    | TileManager uses map_id + floor_id
    | GET /assets/maps/{id}/floors/{floor}/{z}/{x}/{y}.webp
    v
Canvas renders tiles
```

## Files to Create/Modify

### New Files
- `crates/db/src/queries/maps.rs` — map CRUD queries
- `crates/server/src/routes/maps.rs` — API endpoints
- `crates/api-client/src/maps.rs` — client methods
- `crates/app/src/components/strategy/map_picker.rs` — picker component

### Modified Files
- `crates/db/src/migrations.rs` — add map table schema
- `crates/db/src/queries/mod.rs` — register maps module
- `crates/server/src/routes/mod.rs` — register map routes
- `crates/server/src/main.rs` — add ASSETS_DIR env + ServeDir
- `crates/api-client/src/lib.rs` — register maps module
- `crates/app/src/pages/strategy/editor.rs` — wire map picker, fix game_mode, add sub-map selector
- `crates/app/src/components/strategy/mod.rs` — register map_picker module
- `crates/map-pipeline/Cargo.toml` — add optional surrealdb dependency
- `crates/map-pipeline/src/main.rs` — add --register flag + DB upsert logic
