# Map Renderer Design — Bevy Headless

## Problem

Blender's Workbench renderer fails with "Failed to allocate GPU buffers" on Overwatch map scenes (8000+ mesh objects, 3.7M triangles). We need a Rust-native renderer that can handle these scenes reliably.

## Solution

New `crates/map-renderer` crate using Bevy 0.18 headless rendering. Loads a GLB file, renders orthographic top-down views per floor (filtered by Y range), outputs RGBA PNGs with transparent background.

## Architecture

```
Input:  map.glb + map.toml (floor definitions)
                |
    +-----------v-----------+
    |   crates/map-renderer |
    |                       |
    |  1. Load GLB (Bevy)   |
    |  2. Per floor:        |
    |     - Visibility by   |
    |       Y range         |
    |     - Ortho camera    |
    |       looking down -Y |
    |     - Render to image |
    |     - Save PNG        |
    +-----------+-----------+
                |
Output: out/{floor_id}.png (RGBA, transparent BG)
```

## Key Decisions

- **Bevy 0.18**: Battle-tested renderer, wgpu 27-28 (current), handles glTF natively
- **Separate crate**: Keeps map-pipeline lightweight (~5s compile) for non-GPU tasks
- **Vertex colors**: GLB has no textures — only COLOR_0 (AO bake data, ~649 unique colors). Render directly with flat lighting.
- **Config reuse**: Import `FloorConfig` and `RenderConfig` from `map-pipeline::config`

## Rendering Details

- **Engine**: Bevy headless (no window, offscreen render target)
- **Camera**: Orthographic, positioned above scene center, looking -Y
- **Lighting**: Flat/unlit — vertex colors provide all shading via AO bake
- **Floor filtering**: Toggle mesh visibility based on centroid Y vs floor y_min/y_max
- **Resolution**: `pixels_per_meter` from config, computed from scene XZ bounding box
- **Background**: Transparent (RGBA PNG)
- **One render pass per floor**: Adjust visibility, render, save, repeat

## CLI Interface

```
scuffed-map-renderer \
  --glb out/kings_row.glb \
  --config maps/kings_row.toml \
  --output ./out/
```

Options:
- `--glb` — Path to GLB file (required)
- `--config` — Path to map TOML config with floor definitions (required)
- `--output` — Output directory for floor PNGs (required)
- `--floor` — Render only this floor ID (optional, renders all if omitted)

## Full Pipeline Workflow

1. `blender -b map.blend -P export_map.py -- --export-glb --skip-render --output ./out/` (GLB + entities)
2. `scuffed-map detect-floors --glb out/map.glb --output maps/map.toml` (floor detection)
3. `scuffed-map-renderer --glb out/map.glb --config maps/map.toml --output out/` (NEW)
4. `scuffed-map generate-tiles --floor-images out/ --config maps/map.toml --output out/tiles/` (tile pyramid)

## Dependencies

- `bevy` 0.18 (render, asset, image features — minimal plugin set)
- `map-pipeline` (config types only)
- `clap` (CLI)
- `anyhow` (errors)
- `tracing` + `tracing-subscriber` (logging)

## Crate Structure

```
crates/map-renderer/
  Cargo.toml
  src/
    main.rs       # CLI entry point
    lib.rs        # pub fn render_floors()
    plugin.rs     # Bevy plugin: load scene, setup camera, render loop
```
