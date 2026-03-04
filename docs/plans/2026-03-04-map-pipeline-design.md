# OW Map Rendering Pipeline Design

**Date:** 2026-03-04
**Status:** Approved

## Goal

Automatically generate 2D tactical map tiles (split by floor) from 3D Overwatch map models. Output feeds directly into the existing Dioxus app's `TileManager` and `MapMetadata` system.

## Pipeline Overview

```
OW Game Files
    | (manual, once per map)
    v
DataTool extract -> .owmap/.owmdl/.owmat
    | (manual, once per map)
    v
+-- Blender 5 Python Script (automated) ---------------+
|  1. Import via io_scene_owm (or load saved .blend)    |
|  2. Automated cleanup (skybox, OOB, lights, particles)|
|  3. Export .glb (geometry + materials)                 |
|  4. Workbench render: orthographic PNG per floor       |
|  5. Export entities.json (health packs, spawns)        |
+-------------------------------------------------------+
    |
    v
+-- Rust CLI: cargo xtask process-map -----------------+
|  1. Load .glb -> floor detection (Z-histogram)        |
|  2. Output floors config (auto-detected Y-ranges)     |
|  3. [User reviews/overrides floor config]             |
|  4. Tile pyramid from Workbench PNGs                  |
|  5. Generate metadata.json (MapMetadata)              |
|  6. Output: assets/maps/{id}/floors/{f}/{z}/{x}/{y}.webp |
+-------------------------------------------------------+
```

## Blender Script: `scripts/export_map.py`

Run as: `blender -b map.blend -P scripts/export_map.py -- --output ./out --config kings_row.toml`

### Automated Cleanup

Before rendering, the script removes non-gameplay geometry:

- **Lights** — delete entire "Lights" collection
- **Cameras** — delete all camera objects
- **Skybox** — objects whose dimensions exceed `skybox_size_threshold` (default 500m)
- **OOB geometry** — objects beyond `max_distance_from_center` (default 200m)
- **Particles/effects** — objects with particle systems
- **Tiny debris** — objects smaller than `min_object_size` (default 0.01m)

Thresholds are configurable per-map in the TOML config.

### Rendering

For each floor defined in the config:
1. Hide all objects outside that floor's Y-range
2. Position orthographic camera looking straight down, sized to map bounds + padding
3. Set render engine to Workbench (flat/matcap lighting, textured solid mode)
4. Render to `{output}/floors/{floor_id}.png` (RGBA, transparent background)

Performance: 2-8 seconds per floor, 15-50 seconds per entire map.

### Entity Export

Parse Empty objects with known GUID patterns from io_scene_owm to extract:
- Health pack positions and sizes
- Spawn room locations
- Objective positions

Output as `entities.json` sidecar.

### Dependencies

- Blender 5.0+
- io_scene_owm addon (v3.3.0+, available on Blender Extensions)
- No pip packages

## Rust CLI: `crates/map-pipeline/`

New workspace crate: `scuffed-map-pipeline`.

### Subcommands

```sh
# Detect floor levels from glTF geometry
cargo xtask detect-floors --glb kings_row.glb --output maps/kings_row.toml

# Generate tile pyramid from rendered floor PNGs
cargo xtask generate-tiles --config maps/kings_row.toml --images ./out/floors/ --output assets/maps/kings_row/

# Both steps combined
cargo xtask process-map --glb kings_row.glb --images ./out/floors/ --output assets/maps/kings_row/
```

### Floor Detection Algorithm

1. Load `.glb` via `gltf` crate (recursive node traversal with Mat4 transform accumulation)
2. Identify walkable surfaces: `face_normal.dot(UP) > cos(50 degrees)`
3. Collect Y-centroids of walkable faces, weighted by triangle area
4. Build histogram (0.25m bins)
5. Gaussian smooth (sigma = 0.4m)
6. `find_peaks` with prominence filter -> peak positions = floor levels
7. Valleys between peaks = floor Y-range boundaries
8. Output detected floors to TOML config
9. Print diagnostic histogram to terminal

### Tile Generation

1. Load per-floor PNGs (rendered by Blender Workbench)
2. For each floor image:
   - Calculate tile pyramid levels (256x256 WebP tiles)
   - Generate tiles at each zoom level
   - `fast_image_resize` for SIMD-accelerated downscaling
   - `rayon` parallelism across tiles
3. Generate `metadata.json` matching existing `MapMetadata` struct
4. Generate `thumbnail.webp`

### Dependencies

| Crate | Purpose |
|-------|---------|
| `gltf` (`utils`, `names` features) | glTF mesh loading for floor detection |
| `glam` | 3D math (normals, transforms) |
| `find_peaks` | Floor level peak detection |
| `image` | PNG loading, WebP encoding |
| `fast_image_resize` | Tile downscaling |
| `rayon` | Parallel tile generation |
| `rustc-hash` | Fast hash maps for adjacency |
| `clap` | CLI argument parsing |
| `serde` + `serde_json` + `toml` | Config and metadata I/O |

## Per-Map Configuration

Each map gets a TOML config at `maps/{map_id}.toml`:

```toml
[map]
name = "King's Row"
id = "kings_row"
game_mode = "hybrid"
blend_file = "maps/blender/kings_row.blend"

[cleanup]
max_distance_from_center = 200.0
min_object_size = 0.01
remove_lights = true
remove_cameras = true
remove_particles = true
skybox_size_threshold = 500.0

[detection]
walkable_slope_max_degrees = 50
histogram_bin_width = 0.25
gaussian_sigma = 0.4
min_floor_gap = 2.0
peak_min_prominence = 10.0

[render]
pixels_per_meter = 32
camera_padding = 5.0

[tiles]
tile_size = 256
max_zoom = 4

# Auto-detected or manually overridden floor ranges
[[floors]]
id = "underground"
name = "Underground"
y_min = -8.0
y_max = -2.5

[[floors]]
id = "ground"
name = "Ground"
y_min = -2.5
y_max = 4.0
is_default = true

[[floors]]
id = "high_ground"
name = "High Ground"
y_min = 4.0
y_max = 12.0
```

## Output Structure

```
assets/maps/kings_row/
  metadata.json          # MapMetadata (coordinate transforms, floor info, tile pyramid)
  thumbnail.webp         # Map thumbnail
  entities.json          # Health packs, spawns, objectives
  floors/
    ground/
      0/0/0.webp         # zoom 0 (overview)
      1/0/0.webp         # zoom 1
      1/0/1.webp
      1/1/0.webp
      1/1/1.webp
      2/...              # zoom 2+ (full res tiles)
    high_ground/
      ...
    underground/
      ...
```

## Integration

The pipeline outputs match what the Dioxus app already consumes with zero frontend changes:

- `metadata.json` deserializes into `MapMetadata` (`crates/types/src/strategy.rs`)
- Tile paths match `TileManager` URL pattern: `floors/{floor}/{z}/{x}/{y}.webp`
- `FloorLevel` populated from TOML config
- `CoordinateTransform` calculated from `pixels_per_meter` and world bounds

## Workflow

### First time per map

1. Extract OW files with DataTool
2. Open Blender 5, import via io_scene_owm (drag and drop)
3. Run cleanup script, quick visual check, save .blend
4. `cargo xtask detect-floors` -> review floor ranges in TOML
5. `blender -b ... -P scripts/export_map.py` -> floor PNGs
6. `cargo xtask generate-tiles` -> tile pyramid + metadata

### After OW patches

Re-extract, re-import, re-export, re-tile. Floor config usually stays the same.

## Risks

1. **Non-manifold geometry** — OW maps are game assets with holes, overlapping faces, T-junctions. Floor detection must tolerate dirty input.
2. **Floor detection accuracy** — Automatic detection will fail on some maps (gradual elevation, complex multi-level areas). Manual override config is essential.
3. **Entity identification** — io_scene_owm entity names are GUIDs. Cross-referencing with Workshop coordinate data may be needed.
4. **Legal** — Blizzard ToS prohibits reverse engineering, but DataTool/OWLib have existed since 2017 without takedown. Only derived 2D representations are published.

## Research

Full research findings from 13 agents across 3 waves are documented in the project memory at `wave-research-findings.md`.
