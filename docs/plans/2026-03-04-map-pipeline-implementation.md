# Map Rendering Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Rust CLI tool + Blender Python script that converts 3D Overwatch maps into 2D tiled tactical maps, split by floor, compatible with the existing Dioxus frontend.

**Architecture:** Blender Workbench renders textured orthographic views per floor (2-8 sec/floor). Rust handles floor detection from glTF geometry and tile pyramid generation from the rendered PNGs. Per-map TOML config drives both steps.

**Tech Stack:** Rust 2024 edition, `gltf`/`glam`/`find_peaks`/`image`/`fast_image_resize`/`rayon`/`clap`, Python 3 + Blender 5 `bpy` API.

**Design doc:** `docs/plans/2026-03-04-map-pipeline-design.md`

---

## Task 1: Scaffold the `map-pipeline` crate

**Files:**
- Create: `crates/map-pipeline/Cargo.toml`
- Create: `crates/map-pipeline/src/lib.rs`
- Create: `crates/map-pipeline/src/main.rs`
- Modify: `Cargo.toml` (workspace root, add member)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "scuffed-map-pipeline"
version = "0.1.0"
edition = "2024"

[dependencies]
scuffed-types = { path = "../types" }

# glTF loading + 3D math
gltf = { version = "1", features = ["utils", "names"] }
glam = "0.30"

# Floor detection
find_peaks = "0.4"

# Image processing + tile generation
image = { version = "0.25", default-features = false, features = ["png", "webp"] }
fast_image_resize = "5"
rayon = "1"

# Edge adjacency
rustc-hash = "2"

# CLI
clap = { version = "4", features = ["derive"] }

# Config + serialization
serde = { workspace = true }
serde_json = { workspace = true }
toml = "0.8"

# Error handling + logging
anyhow = "1"
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tempfile = "3"
```

**Step 2: Create minimal `src/lib.rs`**

```rust
pub mod config;
```

**Step 3: Create minimal `src/main.rs`**

```rust
fn main() {
    println!("scuffed-map-pipeline");
}
```

**Step 4: Add to workspace**

In root `Cargo.toml`, add `"crates/map-pipeline"` to the `members` array.

**Step 5: Verify it compiles**

Run: `cargo check -p scuffed-map-pipeline`
Expected: compiles with no errors (warnings about unused deps are fine)

**Step 6: Commit**

```bash
git add crates/map-pipeline/ Cargo.toml Cargo.lock
git commit -m "feat: scaffold map-pipeline crate"
```

---

## Task 2: Config types (TOML parsing)

**Files:**
- Create: `crates/map-pipeline/src/config.rs`
- Modify: `crates/map-pipeline/src/lib.rs`

**Step 1: Write tests for config deserialization**

Create `crates/map-pipeline/src/config.rs` with the types and a test module at the bottom:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapConfig {
    pub map: MapInfo,
    #[serde(default)]
    pub cleanup: CleanupConfig,
    #[serde(default)]
    pub detection: DetectionConfig,
    #[serde(default)]
    pub render: RenderConfig,
    #[serde(default)]
    pub tiles: TileConfig,
    #[serde(default)]
    pub floors: Vec<FloorConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapInfo {
    pub name: String,
    pub id: String,
    #[serde(default)]
    pub game_mode: String,
    #[serde(default)]
    pub blend_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    #[serde(default = "default_max_distance")]
    pub max_distance_from_center: f64,
    #[serde(default = "default_min_object_size")]
    pub min_object_size: f64,
    #[serde(default = "default_true")]
    pub remove_lights: bool,
    #[serde(default = "default_true")]
    pub remove_cameras: bool,
    #[serde(default = "default_true")]
    pub remove_particles: bool,
    #[serde(default = "default_skybox_threshold")]
    pub skybox_size_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionConfig {
    #[serde(default = "default_slope")]
    pub walkable_slope_max_degrees: f64,
    #[serde(default = "default_bin_width")]
    pub histogram_bin_width: f64,
    #[serde(default = "default_sigma")]
    pub gaussian_sigma: f64,
    #[serde(default = "default_floor_gap")]
    pub min_floor_gap: f64,
    #[serde(default = "default_prominence")]
    pub peak_min_prominence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderConfig {
    #[serde(default = "default_ppm")]
    pub pixels_per_meter: f64,
    #[serde(default = "default_padding")]
    pub camera_padding: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileConfig {
    #[serde(default = "default_tile_size")]
    pub tile_size: u32,
    #[serde(default = "default_max_zoom")]
    pub max_zoom: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloorConfig {
    pub id: String,
    pub name: String,
    pub y_min: f64,
    pub y_max: f64,
    #[serde(default)]
    pub is_default: bool,
}

// Default value functions
fn default_max_distance() -> f64 { 200.0 }
fn default_min_object_size() -> f64 { 0.01 }
fn default_true() -> bool { true }
fn default_skybox_threshold() -> f64 { 500.0 }
fn default_slope() -> f64 { 50.0 }
fn default_bin_width() -> f64 { 0.25 }
fn default_sigma() -> f64 { 0.4 }
fn default_floor_gap() -> f64 { 2.0 }
fn default_prominence() -> f64 { 10.0 }
fn default_ppm() -> f64 { 32.0 }
fn default_padding() -> f64 { 5.0 }
fn default_tile_size() -> u32 { 256 }
fn default_max_zoom() -> u32 { 4 }

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            max_distance_from_center: default_max_distance(),
            min_object_size: default_min_object_size(),
            remove_lights: true,
            remove_cameras: true,
            remove_particles: true,
            skybox_size_threshold: default_skybox_threshold(),
        }
    }
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            walkable_slope_max_degrees: default_slope(),
            histogram_bin_width: default_bin_width(),
            gaussian_sigma: default_sigma(),
            min_floor_gap: default_floor_gap(),
            peak_min_prominence: default_prominence(),
        }
    }
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            pixels_per_meter: default_ppm(),
            camera_padding: default_padding(),
        }
    }
}

impl Default for TileConfig {
    fn default() -> Self {
        Self {
            tile_size: default_tile_size(),
            max_zoom: default_max_zoom(),
        }
    }
}

impl MapConfig {
    pub fn from_toml(content: &str) -> anyhow::Result<Self> {
        Ok(toml::from_str(content)?)
    }

    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml = r#"
[map]
name = "King's Row"
id = "kings_row"
game_mode = "hybrid"
blend_file = "maps/blender/kings_row.blend"

[cleanup]
max_distance_from_center = 200.0
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
"#;
        let config = MapConfig::from_toml(toml).unwrap();
        assert_eq!(config.map.name, "King's Row");
        assert_eq!(config.map.id, "kings_row");
        assert_eq!(config.floors.len(), 2);
        assert!(config.floors[0].is_default);
        assert_eq!(config.tiles.tile_size, 256);
        assert_eq!(config.detection.walkable_slope_max_degrees, 50.0);
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[map]
name = "Test Map"
id = "test"
"#;
        let config = MapConfig::from_toml(toml).unwrap();
        assert_eq!(config.map.name, "Test Map");
        // All defaults should apply
        assert_eq!(config.detection.walkable_slope_max_degrees, 50.0);
        assert_eq!(config.tiles.tile_size, 256);
        assert_eq!(config.render.pixels_per_meter, 32.0);
        assert!(config.floors.is_empty());
    }

    #[test]
    fn roundtrip_config() {
        let config = MapConfig {
            map: MapInfo {
                name: "Test".into(),
                id: "test".into(),
                game_mode: "escort".into(),
                blend_file: String::new(),
            },
            cleanup: CleanupConfig::default(),
            detection: DetectionConfig::default(),
            render: RenderConfig::default(),
            tiles: TileConfig::default(),
            floors: vec![FloorConfig {
                id: "ground".into(),
                name: "Ground".into(),
                y_min: 0.0,
                y_max: 4.0,
                is_default: true,
            }],
        };
        let serialized = config.to_toml().unwrap();
        let deserialized = MapConfig::from_toml(&serialized).unwrap();
        assert_eq!(deserialized.map.id, "test");
        assert_eq!(deserialized.floors.len(), 1);
    }
}
```

**Step 2: Update `lib.rs`**

```rust
pub mod config;
```

**Step 3: Run tests to verify they pass**

Run: `cargo test -p scuffed-map-pipeline`
Expected: 3 tests pass

**Step 4: Commit**

```bash
git add crates/map-pipeline/src/config.rs crates/map-pipeline/src/lib.rs
git commit -m "feat(map-pipeline): add TOML config types with defaults"
```

---

## Task 3: Histogram and Gaussian smoothing utilities

**Files:**
- Create: `crates/map-pipeline/src/histogram.rs`
- Modify: `crates/map-pipeline/src/lib.rs`

These are pure math functions used by floor detection. Fully testable without any I/O.

**Step 1: Write failing tests**

Create `crates/map-pipeline/src/histogram.rs` with empty function signatures and tests:

```rust
/// Build a histogram from weighted samples.
/// Returns (bin_edges, bin_counts) where bin_counts[i] covers [bin_edges[i], bin_edges[i+1]).
pub fn build_histogram(
    samples: &[(f64, f64)], // (value, weight) pairs
    bin_width: f64,
) -> (Vec<f64>, Vec<f64>) {
    todo!()
}

/// Apply 1D Gaussian smoothing to a signal.
/// Uses a kernel of radius ceil(3*sigma/bin_width) bins.
pub fn gaussian_smooth(signal: &[f64], sigma: f64, bin_width: f64) -> Vec<f64> {
    todo!()
}

/// Find valleys (local minima) between peaks.
/// Given peak positions (as indices into the signal), find the minimum value
/// between each consecutive pair of peaks. Returns (index, value) pairs.
pub fn find_valleys(signal: &[f64], peak_indices: &[usize]) -> Vec<(usize, f64)> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_basic() {
        // 10 samples clustered at 0.0 and 3.0
        let samples: Vec<(f64, f64)> = vec![
            (0.0, 1.0), (0.1, 1.0), (0.2, 1.0), (0.1, 1.0), (0.15, 1.0),
            (3.0, 1.0), (3.1, 1.0), (3.2, 1.0), (3.05, 1.0), (3.15, 1.0),
        ];
        let (edges, counts) = build_histogram(&samples, 0.5);
        // Should have bins from 0.0 to ~3.5
        assert!(edges.len() >= 2);
        assert_eq!(counts.len(), edges.len() - 1);
        // First bin (0.0-0.5) should have weight 5.0
        assert_eq!(counts[0], 5.0);
        // Bin containing 3.0-3.5 should have weight 5.0
        let bin_3 = ((3.0 - edges[0]) / 0.5).floor() as usize;
        assert_eq!(counts[bin_3], 5.0);
    }

    #[test]
    fn histogram_empty() {
        let (edges, counts) = build_histogram(&[], 0.5);
        assert!(edges.is_empty());
        assert!(counts.is_empty());
    }

    #[test]
    fn histogram_weighted() {
        // One sample with weight 10 should produce a bin with count 10
        let samples = vec![(1.0, 10.0)];
        let (_, counts) = build_histogram(&samples, 0.5);
        let bin = counts.iter().find(|&&c| c > 0.0).unwrap();
        assert_eq!(*bin, 10.0);
    }

    #[test]
    fn gaussian_smooth_identity() {
        // With sigma=0, smoothing should be identity (or near-identity)
        let signal = vec![0.0, 0.0, 10.0, 0.0, 0.0];
        let smoothed = gaussian_smooth(&signal, 0.001, 1.0);
        assert_eq!(smoothed.len(), 5);
        // Peak should still be at index 2
        let max_idx = smoothed.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap().0;
        assert_eq!(max_idx, 2);
    }

    #[test]
    fn gaussian_smooth_spreads_peak() {
        // With larger sigma, the peak should spread
        let signal = vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0];
        let smoothed = gaussian_smooth(&signal, 1.0, 1.0);
        // Neighbors should now have nonzero values
        assert!(smoothed[2] > 0.0);
        assert!(smoothed[4] > 0.0);
        // Peak should still be at center
        let max_idx = smoothed.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap().0;
        assert_eq!(max_idx, 3);
    }

    #[test]
    fn gaussian_smooth_empty() {
        let smoothed = gaussian_smooth(&[], 1.0, 1.0);
        assert!(smoothed.is_empty());
    }

    #[test]
    fn find_valleys_between_two_peaks() {
        // Signal with two peaks at indices 2 and 7, valley around index 5
        let signal = vec![0.0, 5.0, 10.0, 5.0, 2.0, 1.0, 2.0, 8.0, 3.0];
        let valleys = find_valleys(&signal, &[2, 7]);
        assert_eq!(valleys.len(), 1);
        assert_eq!(valleys[0].0, 5); // valley at index 5 (value 1.0)
        assert_eq!(valleys[0].1, 1.0);
    }

    #[test]
    fn find_valleys_single_peak() {
        let signal = vec![0.0, 5.0, 10.0, 5.0, 0.0];
        let valleys = find_valleys(&signal, &[2]);
        assert!(valleys.is_empty()); // No valleys with only one peak
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p scuffed-map-pipeline histogram`
Expected: FAIL with `not yet implemented`

**Step 3: Implement the functions**

Replace the `todo!()` bodies:

```rust
pub fn build_histogram(
    samples: &[(f64, f64)],
    bin_width: f64,
) -> (Vec<f64>, Vec<f64>) {
    if samples.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let min_val = samples.iter().map(|(v, _)| *v).fold(f64::INFINITY, f64::min);
    let max_val = samples.iter().map(|(v, _)| *v).fold(f64::NEG_INFINITY, f64::max);

    let n_bins = ((max_val - min_val) / bin_width).ceil() as usize + 1;
    let mut counts = vec![0.0f64; n_bins];
    let mut edges = Vec::with_capacity(n_bins + 1);

    for i in 0..=n_bins {
        edges.push(min_val + i as f64 * bin_width);
    }

    for &(value, weight) in samples {
        let bin = ((value - min_val) / bin_width).floor() as usize;
        let bin = bin.min(n_bins - 1);
        counts[bin] += weight;
    }

    (edges, counts)
}

pub fn gaussian_smooth(signal: &[f64], sigma: f64, bin_width: f64) -> Vec<f64> {
    if signal.is_empty() {
        return Vec::new();
    }

    let sigma_bins = sigma / bin_width;
    let radius = (3.0 * sigma_bins).ceil() as usize;

    if radius == 0 {
        return signal.to_vec();
    }

    // Build kernel
    let kernel_size = 2 * radius + 1;
    let mut kernel = Vec::with_capacity(kernel_size);
    let mut kernel_sum = 0.0;
    for i in 0..kernel_size {
        let x = i as f64 - radius as f64;
        let val = (-0.5 * (x / sigma_bins).powi(2)).exp();
        kernel.push(val);
        kernel_sum += val;
    }
    // Normalize
    for k in &mut kernel {
        *k /= kernel_sum;
    }

    // Convolve
    let n = signal.len();
    let mut result = vec![0.0; n];
    for i in 0..n {
        let mut sum = 0.0;
        for (j, &k) in kernel.iter().enumerate() {
            let idx = i as isize + j as isize - radius as isize;
            let idx = idx.clamp(0, n as isize - 1) as usize;
            sum += signal[idx] * k;
        }
        result[i] = sum;
    }

    result
}

pub fn find_valleys(signal: &[f64], peak_indices: &[usize]) -> Vec<(usize, f64)> {
    if peak_indices.len() < 2 {
        return Vec::new();
    }

    let mut valleys = Vec::with_capacity(peak_indices.len() - 1);
    for window in peak_indices.windows(2) {
        let (start, end) = (window[0], window[1]);
        let mut min_idx = start;
        let mut min_val = f64::INFINITY;
        for i in start..=end {
            if signal[i] < min_val {
                min_val = signal[i];
                min_idx = i;
            }
        }
        valleys.push((min_idx, min_val));
    }

    valleys
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p scuffed-map-pipeline histogram`
Expected: all 7 tests pass

**Step 5: Update `lib.rs`**

```rust
pub mod config;
pub mod histogram;
```

**Step 6: Commit**

```bash
git add crates/map-pipeline/src/histogram.rs crates/map-pipeline/src/lib.rs
git commit -m "feat(map-pipeline): add histogram and Gaussian smoothing utilities"
```

---

## Task 4: glTF mesh loading

**Files:**
- Create: `crates/map-pipeline/src/mesh.rs`
- Modify: `crates/map-pipeline/src/lib.rs`

This module loads a glTF file and extracts all mesh triangles with world-space positions. No test fixtures needed for the types/math — the glTF loading itself is tested via integration test later.

**Step 1: Write the mesh types and face normal computation with tests**

Create `crates/map-pipeline/src/mesh.rs`:

```rust
use glam::{Mat4, Vec3};

/// A triangle in world space with precomputed normal.
#[derive(Debug, Clone)]
pub struct Triangle {
    pub v0: Vec3,
    pub v1: Vec3,
    pub v2: Vec3,
    pub normal: Vec3,
}

impl Triangle {
    pub fn new(v0: Vec3, v1: Vec3, v2: Vec3) -> Self {
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let normal = edge1.cross(edge2).normalize_or_zero();
        Self { v0, v1, v2, normal }
    }

    /// Area of the triangle.
    pub fn area(&self) -> f32 {
        let edge1 = self.v1 - self.v0;
        let edge2 = self.v2 - self.v0;
        edge1.cross(edge2).length() * 0.5
    }

    /// Y-coordinate of the centroid (for floor detection).
    pub fn centroid_y(&self) -> f32 {
        (self.v0.y + self.v1.y + self.v2.y) / 3.0
    }

    /// Whether this face is walkable (normal points mostly upward).
    /// Uses dot product with UP vector; threshold is cos(max_slope_degrees).
    pub fn is_walkable(&self, max_slope_degrees: f64) -> bool {
        let threshold = (max_slope_degrees.to_radians()).cos() as f32;
        self.normal.dot(Vec3::Y) > threshold
    }
}

/// Load all mesh triangles from a glTF file, transformed to world space.
pub fn load_glb(path: &std::path::Path) -> anyhow::Result<Vec<Triangle>> {
    let (document, buffers, _images) = gltf::import(path)?;

    let mut triangles = Vec::new();

    for scene in document.scenes() {
        for node in scene.nodes() {
            collect_node_triangles(&node, Mat4::IDENTITY, &buffers, &mut triangles);
        }
    }

    tracing::info!("Loaded {} triangles from {:?}", triangles.len(), path);
    Ok(triangles)
}

fn collect_node_triangles(
    node: &gltf::Node,
    parent_transform: Mat4,
    buffers: &[gltf::buffer::Data],
    triangles: &mut Vec<Triangle>,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_transform * local;

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

            let positions: Vec<Vec3> = match reader.read_positions() {
                Some(pos) => pos.map(|p| {
                    let world_pos = world.transform_point3(Vec3::from(p));
                    world_pos
                }).collect(),
                None => continue,
            };

            if let Some(indices) = reader.read_indices() {
                let indices: Vec<u32> = indices.into_u32().collect();
                for tri in indices.chunks_exact(3) {
                    let v0 = positions[tri[0] as usize];
                    let v1 = positions[tri[1] as usize];
                    let v2 = positions[tri[2] as usize];
                    triangles.push(Triangle::new(v0, v1, v2));
                }
            } else {
                // Non-indexed geometry
                for tri in positions.chunks_exact(3) {
                    triangles.push(Triangle::new(tri[0], tri[1], tri[2]));
                }
            }
        }
    }

    for child in node.children() {
        collect_node_triangles(&child, world, buffers, triangles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_flat_floor_is_walkable() {
        // Flat horizontal triangle (normal pointing straight up)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert!((tri.normal.y - 1.0).abs() < 0.01, "Normal should be (0,1,0), got {:?}", tri.normal);
        assert!(tri.is_walkable(50.0));
    }

    #[test]
    fn triangle_wall_not_walkable() {
        // Vertical wall (normal pointing along X)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert!(tri.normal.y.abs() < 0.01, "Wall normal Y should be ~0, got {}", tri.normal.y);
        assert!(!tri.is_walkable(50.0));
    }

    #[test]
    fn triangle_steep_slope_not_walkable() {
        // 60-degree slope (normal 30 degrees from horizontal)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.732, 0.0), // tan(60°) = 1.732
            Vec3::new(0.0, 0.0, 1.0),
        );
        assert!(!tri.is_walkable(50.0)); // 60° > 50° max
    }

    #[test]
    fn triangle_area() {
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 2.0),
        );
        assert!((tri.area() - 2.0).abs() < 0.01); // 2x2 right triangle = area 2
    }

    #[test]
    fn triangle_centroid_y() {
        let tri = Triangle::new(
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(0.0, 3.0, 1.0),
        );
        assert!((tri.centroid_y() - 2.0).abs() < 0.01); // (1+2+3)/3 = 2
    }

    #[test]
    fn degenerate_triangle_zero_area() {
        // Degenerate (collinear points)
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        );
        assert!(tri.area() < 0.001);
        assert!(!tri.is_walkable(50.0)); // Zero normal → not walkable
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p scuffed-map-pipeline mesh`
Expected: all 6 tests pass

**Step 3: Update `lib.rs`**

```rust
pub mod config;
pub mod histogram;
pub mod mesh;
```

**Step 4: Commit**

```bash
git add crates/map-pipeline/src/mesh.rs crates/map-pipeline/src/lib.rs
git commit -m "feat(map-pipeline): add glTF mesh loading with walkable surface detection"
```

---

## Task 5: Floor detection algorithm

**Files:**
- Create: `crates/map-pipeline/src/floor_detect.rs`
- Modify: `crates/map-pipeline/src/lib.rs`

This ties together mesh analysis, histogram, and peak detection to produce floor Y-ranges.

**Step 1: Write the floor detection module with tests**

Create `crates/map-pipeline/src/floor_detect.rs`:

```rust
use crate::config::{DetectionConfig, FloorConfig};
use crate::histogram::{build_histogram, find_valleys, gaussian_smooth};
use crate::mesh::Triangle;

/// Result of floor detection: a list of detected floor levels.
pub struct FloorDetectionResult {
    pub floors: Vec<FloorConfig>,
    /// The smoothed histogram (for diagnostic display).
    pub histogram_edges: Vec<f64>,
    pub histogram_values: Vec<f64>,
    /// Indices of detected peaks in the histogram.
    pub peak_indices: Vec<usize>,
}

/// Detect floor levels from mesh triangles.
pub fn detect_floors(
    triangles: &[Triangle],
    config: &DetectionConfig,
) -> anyhow::Result<FloorDetectionResult> {
    // 1. Filter to walkable faces and collect (Y-centroid, area) pairs
    let walkable_samples: Vec<(f64, f64)> = triangles
        .iter()
        .filter(|t| t.is_walkable(config.walkable_slope_max_degrees))
        .filter(|t| t.area() > 0.001) // Skip degenerate triangles
        .map(|t| (t.centroid_y() as f64, t.area() as f64))
        .collect();

    if walkable_samples.is_empty() {
        anyhow::bail!("No walkable surfaces found. Check walkable_slope_max_degrees ({} deg).",
            config.walkable_slope_max_degrees);
    }

    tracing::info!("Found {} walkable faces", walkable_samples.len());

    // 2. Build histogram
    let (edges, counts) = build_histogram(&walkable_samples, config.histogram_bin_width);

    // 3. Gaussian smooth
    let smoothed = gaussian_smooth(&counts, config.gaussian_sigma, config.histogram_bin_width);

    // 4. Peak detection
    let min_distance = (config.min_floor_gap / config.histogram_bin_width).ceil() as usize;
    let peaks = find_peaks_in_signal(&smoothed, config.peak_min_prominence, min_distance);

    if peaks.is_empty() {
        anyhow::bail!("No floor peaks detected. Try lowering peak_min_prominence (currently {}).",
            config.peak_min_prominence);
    }

    tracing::info!("Detected {} floor peaks", peaks.len());

    // 5. Find valleys between peaks → floor boundaries
    let valleys = find_valleys(&smoothed, &peaks);

    // 6. Convert to FloorConfig
    let y_min_global = edges.first().copied().unwrap_or(0.0);
    let y_max_global = edges.last().copied().unwrap_or(0.0);

    let mut floors = Vec::new();
    for (i, &peak_idx) in peaks.iter().enumerate() {
        let floor_y_min = if i == 0 {
            y_min_global
        } else {
            let valley_idx = valleys[i - 1].0;
            edges[valley_idx]
        };

        let floor_y_max = if i == peaks.len() - 1 {
            y_max_global
        } else {
            let valley_idx = valleys[i].0;
            edges[valley_idx]
        };

        let peak_y = edges[peak_idx] + config.histogram_bin_width / 2.0;
        let name = format!("Floor {}", i + 1);
        let id = format!("floor_{}", i + 1);

        tracing::info!("  {} (peak Y={:.1}m): [{:.1}m, {:.1}m]", name, peak_y, floor_y_min, floor_y_max);

        floors.push(FloorConfig {
            id,
            name,
            y_min: floor_y_min,
            y_max: floor_y_max,
            is_default: i == 0, // First floor is default (lowest)
        });
    }

    // Make the floor closest to y=0 the default (most likely "ground")
    if let Some(ground_idx) = floors.iter().enumerate()
        .min_by_key(|(_, f)| {
            let mid = (f.y_min + f.y_max) / 2.0;
            (mid.abs() * 1000.0) as i64
        })
        .map(|(i, _)| i)
    {
        for (i, floor) in floors.iter_mut().enumerate() {
            floor.is_default = i == ground_idx;
        }
    }

    Ok(FloorDetectionResult {
        floors,
        histogram_edges: edges,
        histogram_values: smoothed,
        peak_indices: peaks,
    })
}

/// Wrapper around `find_peaks` crate.
fn find_peaks_in_signal(signal: &[f64], min_prominence: f64, min_distance: usize) -> Vec<usize> {
    use find_peaks::PeakFinder;

    let mut finder = PeakFinder::new(signal);
    finder.with_min_prominence(min_prominence);
    finder.with_min_distance(min_distance);

    let peaks = finder.find_peaks();
    peaks.into_iter().map(|p| p.middle_position()).collect()
}

/// Print a simple ASCII histogram to the terminal for diagnostics.
pub fn print_histogram(result: &FloorDetectionResult) {
    let max_val = result.histogram_values.iter().cloned().fold(0.0f64, f64::max);
    if max_val == 0.0 { return; }

    let bar_width = 60;
    println!("\n  Y (m)  | Walkable surface area");
    println!("  -------+{}", "-".repeat(bar_width + 2));

    for (i, &val) in result.histogram_values.iter().enumerate() {
        let y = result.histogram_edges[i];
        let bar_len = ((val / max_val) * bar_width as f64).round() as usize;
        let bar: String = "#".repeat(bar_len);
        let marker = if result.peak_indices.contains(&i) { " <-- FLOOR" } else { "" };
        println!("  {:6.1} | {}{}", y, bar, marker);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn make_floor_patch(y: f32, count: usize, area: f32) -> Vec<Triangle> {
        // Create `count` flat horizontal triangles at height `y`
        // Each triangle is a unit square half (area ~= area param)
        let scale = (area * 2.0).sqrt();
        (0..count).map(|i| {
            let x = i as f32 * scale;
            Triangle::new(
                Vec3::new(x, y, 0.0),
                Vec3::new(x + scale, y, 0.0),
                Vec3::new(x, y, scale),
            )
        }).collect()
    }

    #[test]
    fn detect_two_floors() {
        let config = DetectionConfig {
            walkable_slope_max_degrees: 50.0,
            histogram_bin_width: 0.25,
            gaussian_sigma: 0.4,
            min_floor_gap: 2.0,
            peak_min_prominence: 1.0, // Lower for test data
        };

        // Ground floor at y=0, upper floor at y=4
        let mut triangles = make_floor_patch(0.0, 50, 1.0);
        triangles.extend(make_floor_patch(4.0, 50, 1.0));

        let result = detect_floors(&triangles, &config).unwrap();
        assert_eq!(result.floors.len(), 2);
        // First floor should contain y=0, second should contain y=4
        assert!(result.floors[0].y_min <= 0.0 && result.floors[0].y_max >= 0.0);
        assert!(result.floors[1].y_min <= 4.0 && result.floors[1].y_max >= 4.0);
    }

    #[test]
    fn detect_single_floor() {
        let config = DetectionConfig {
            walkable_slope_max_degrees: 50.0,
            histogram_bin_width: 0.25,
            gaussian_sigma: 0.4,
            min_floor_gap: 2.0,
            peak_min_prominence: 1.0,
        };

        let triangles = make_floor_patch(0.0, 100, 1.0);

        let result = detect_floors(&triangles, &config).unwrap();
        assert_eq!(result.floors.len(), 1);
        assert!(result.floors[0].is_default);
    }

    #[test]
    fn detect_no_walkable_faces() {
        let config = DetectionConfig::default();

        // All vertical walls — no walkable surfaces
        let triangles = vec![
            Triangle::new(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 5.0, 0.0),
                Vec3::new(0.0, 0.0, 5.0),
            ),
        ];

        let result = detect_floors(&triangles, &config);
        assert!(result.is_err());
    }

    #[test]
    fn ground_floor_is_default() {
        let config = DetectionConfig {
            walkable_slope_max_degrees: 50.0,
            histogram_bin_width: 0.25,
            gaussian_sigma: 0.4,
            min_floor_gap: 2.0,
            peak_min_prominence: 1.0,
        };

        // Three floors: underground (-5), ground (0), upper (5)
        let mut triangles = make_floor_patch(-5.0, 50, 1.0);
        triangles.extend(make_floor_patch(0.0, 50, 1.0));
        triangles.extend(make_floor_patch(5.0, 50, 1.0));

        let result = detect_floors(&triangles, &config).unwrap();
        // Floor closest to y=0 should be default
        let default_floor = result.floors.iter().find(|f| f.is_default).unwrap();
        let default_mid = (default_floor.y_min + default_floor.y_max) / 2.0;
        assert!(default_mid.abs() < 2.0, "Default floor mid={} should be near y=0", default_mid);
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p scuffed-map-pipeline floor_detect`
Expected: all 4 tests pass

**Step 3: Update `lib.rs`**

```rust
pub mod config;
pub mod floor_detect;
pub mod histogram;
pub mod mesh;
```

**Step 4: Commit**

```bash
git add crates/map-pipeline/src/floor_detect.rs crates/map-pipeline/src/lib.rs
git commit -m "feat(map-pipeline): add floor detection via histogram peak analysis"
```

---

## Task 6: Tile pyramid generation

**Files:**
- Create: `crates/map-pipeline/src/tiles.rs`
- Modify: `crates/map-pipeline/src/lib.rs`

This module takes a full-resolution floor PNG and generates a tile pyramid (WebP tiles at multiple zoom levels).

**Step 1: Write tile coordinate math with tests, then the generation logic**

Create `crates/map-pipeline/src/tiles.rs`:

```rust
use anyhow::Context;
use image::{DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use rayon::prelude::*;
use std::path::Path;

/// Calculate the number of tiles at a given zoom level.
pub fn tiles_at_zoom(full_width: u32, full_height: u32, tile_size: u32, zoom: u32, max_zoom: u32) -> (u32, u32) {
    let scale = 1u32 << (max_zoom - zoom.min(max_zoom));
    let scaled_w = full_width / scale;
    let scaled_h = full_height / scale;
    let cols = (scaled_w + tile_size - 1) / tile_size;
    let rows = (scaled_h + tile_size - 1) / tile_size;
    (cols.max(1), rows.max(1))
}

/// Calculate the maximum zoom level for a given image size and tile size.
/// At max_zoom, the image is served at full resolution.
/// Each lower zoom level halves the resolution.
pub fn calculate_max_zoom(full_width: u32, full_height: u32, tile_size: u32) -> u32 {
    let max_dim = full_width.max(full_height);
    if max_dim <= tile_size {
        return 0;
    }
    // How many times can we halve before fitting in one tile?
    let levels = (max_dim as f64 / tile_size as f64).log2().ceil() as u32;
    levels
}

/// Generate a full tile pyramid from a floor image.
///
/// Creates: `{output_dir}/{floor_id}/{zoom}/{x}/{y}.webp`
///
/// Returns (full_width, full_height) of the source image.
pub fn generate_tile_pyramid(
    image_path: &Path,
    output_dir: &Path,
    floor_id: &str,
    tile_size: u32,
    max_zoom: Option<u32>,
) -> anyhow::Result<(u32, u32)> {
    let img = image::open(image_path)
        .with_context(|| format!("Failed to open floor image: {:?}", image_path))?;

    let (full_width, full_height) = img.dimensions();
    let max_zoom = max_zoom.unwrap_or_else(|| calculate_max_zoom(full_width, full_height, tile_size));

    tracing::info!("Generating tiles for floor '{}': {}x{}, max_zoom={}", floor_id, full_width, full_height, max_zoom);

    for zoom in 0..=max_zoom {
        generate_zoom_level(&img, output_dir, floor_id, tile_size, zoom, max_zoom)?;
    }

    Ok((full_width, full_height))
}

fn generate_zoom_level(
    img: &DynamicImage,
    output_dir: &Path,
    floor_id: &str,
    tile_size: u32,
    zoom: u32,
    max_zoom: u32,
) -> anyhow::Result<()> {
    let (full_w, full_h) = img.dimensions();
    let scale = 1u32 << (max_zoom - zoom);

    // Resize image for this zoom level
    let scaled_w = (full_w / scale).max(1);
    let scaled_h = (full_h / scale).max(1);

    let scaled_img = if zoom == max_zoom {
        img.clone()
    } else {
        img.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Lanczos3)
    };

    let (cols, rows) = tiles_at_zoom(full_w, full_h, tile_size, zoom, max_zoom);

    tracing::info!("  Zoom {}: {}x{} -> {}x{} tiles", zoom, scaled_w, scaled_h, cols, rows);

    // Generate tiles in parallel
    let tile_coords: Vec<(u32, u32)> = (0..rows)
        .flat_map(|y| (0..cols).map(move |x| (x, y)))
        .collect();

    tile_coords.par_iter().try_for_each(|&(x, y)| -> anyhow::Result<()> {
        let tile_dir = output_dir.join(format!("floors/{}/{}", floor_id, zoom));
        std::fs::create_dir_all(tile_dir.join(format!("{}", x)))?;

        let src_x = x * tile_size;
        let src_y = y * tile_size;
        let crop_w = tile_size.min(scaled_w.saturating_sub(src_x));
        let crop_h = tile_size.min(scaled_h.saturating_sub(src_y));

        if crop_w == 0 || crop_h == 0 {
            return Ok(());
        }

        // Crop the tile from the scaled image
        let tile_img = scaled_img.crop_imm(src_x, src_y, crop_w, crop_h);

        // If tile is smaller than tile_size, pad with transparent pixels
        let final_tile = if crop_w < tile_size || crop_h < tile_size {
            let mut padded = RgbaImage::new(tile_size, tile_size);
            image::imageops::overlay(&mut padded, &tile_img.to_rgba8(), 0, 0);
            DynamicImage::ImageRgba8(padded)
        } else {
            tile_img
        };

        let tile_path = tile_dir.join(format!("{}/{}.webp", x, y));
        final_tile.save_with_format(&tile_path, ImageFormat::WebP)?;

        Ok(())
    })?;

    Ok(())
}

/// Generate a thumbnail from a floor image.
pub fn generate_thumbnail(
    image_path: &Path,
    output_path: &Path,
    max_dimension: u32,
) -> anyhow::Result<()> {
    let img = image::open(image_path)?;
    let thumbnail = img.thumbnail(max_dimension, max_dimension);
    thumbnail.save_with_format(output_path, ImageFormat::WebP)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_at_max_zoom() {
        // 1024x512 image with 256 tiles at max zoom (zoom=2)
        let (cols, rows) = tiles_at_zoom(1024, 512, 256, 2, 2);
        assert_eq!(cols, 4); // 1024/256
        assert_eq!(rows, 2); // 512/256
    }

    #[test]
    fn tiles_at_zoom_zero() {
        // At zoom 0 with max_zoom=2, scale is 4x
        // 1024/4 = 256 -> 1 tile wide
        // 512/4 = 128 -> 1 tile tall
        let (cols, rows) = tiles_at_zoom(1024, 512, 256, 0, 2);
        assert_eq!(cols, 1);
        assert_eq!(rows, 1);
    }

    #[test]
    fn tiles_at_zoom_noneven() {
        // 1000x500 at max zoom should round up
        let (cols, rows) = tiles_at_zoom(1000, 500, 256, 3, 3);
        assert_eq!(cols, 4); // ceil(1000/256)
        assert_eq!(rows, 2); // ceil(500/256)
    }

    #[test]
    fn calculate_max_zoom_small_image() {
        // 256x256 fits in one tile
        assert_eq!(calculate_max_zoom(256, 256, 256), 0);
    }

    #[test]
    fn calculate_max_zoom_large_image() {
        // 2048x2048 needs 3 zoom levels (2048/256 = 8 = 2^3)
        assert_eq!(calculate_max_zoom(2048, 2048, 256), 3);
    }

    #[test]
    fn calculate_max_zoom_nonsquare() {
        // 4096x1024 → max dim is 4096 → log2(4096/256) = 4
        assert_eq!(calculate_max_zoom(4096, 1024, 256), 4);
    }

    #[test]
    fn generate_tiles_from_test_image() {
        // Create a small test image (512x512 red square)
        let img = RgbaImage::from_fn(512, 512, |_, _| image::Rgba([255, 0, 0, 255]));
        let tmp = tempfile::tempdir().unwrap();
        let img_path = tmp.path().join("test_floor.png");
        img.save(&img_path).unwrap();

        let output_dir = tmp.path().join("output");
        let (w, h) = generate_tile_pyramid(&img_path, &output_dir, "ground", 256, Some(1)).unwrap();

        assert_eq!(w, 512);
        assert_eq!(h, 512);

        // At zoom 1 (max): 2x2 tiles
        assert!(output_dir.join("floors/ground/1/0/0.webp").exists());
        assert!(output_dir.join("floors/ground/1/0/1.webp").exists());
        assert!(output_dir.join("floors/ground/1/1/0.webp").exists());
        assert!(output_dir.join("floors/ground/1/1/1.webp").exists());

        // At zoom 0: 1x1 tile
        assert!(output_dir.join("floors/ground/0/0/0.webp").exists());
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p scuffed-map-pipeline tiles`
Expected: all 7 tests pass

**Step 3: Update `lib.rs`**

```rust
pub mod config;
pub mod floor_detect;
pub mod histogram;
pub mod mesh;
pub mod tiles;
```

**Step 4: Commit**

```bash
git add crates/map-pipeline/src/tiles.rs crates/map-pipeline/src/lib.rs
git commit -m "feat(map-pipeline): add tile pyramid generation with WebP output"
```

---

## Task 7: Metadata generation

**Files:**
- Create: `crates/map-pipeline/src/metadata.rs`
- Modify: `crates/map-pipeline/src/lib.rs`

Generates `metadata.json` compatible with the existing `MapMetadata` struct from `scuffed-types`.

**Step 1: Write the metadata builder with tests**

Create `crates/map-pipeline/src/metadata.rs`:

```rust
use crate::config::{FloorConfig, MapConfig, RenderConfig};
use scuffed_types::{
    CoordinateTransform, FloorLevel, MapMetadata, TilePyramidInfo, WorldBounds,
};
use std::path::Path;

/// Build MapMetadata from pipeline config and detected image dimensions.
pub fn build_metadata(
    config: &MapConfig,
    floor_image_sizes: &[(String, u32, u32)], // (floor_id, width, height)
    world_bounds: WorldBounds,
) -> MapMetadata {
    // Use the largest floor image dimensions as composite size
    let (composite_width, composite_height) = floor_image_sizes
        .iter()
        .fold((0u32, 0u32), |(w, h), (_, fw, fh)| (w.max(*fw), h.max(*fh)));

    let transform = CoordinateTransform::from_bounds(&world_bounds, composite_width, composite_height);

    let tile_pyramid = Some(TilePyramidInfo {
        tile_size: config.tiles.tile_size,
        max_zoom: config.tiles.max_zoom,
        full_width: composite_width,
        full_height: composite_height,
    });

    let floors: Vec<FloorLevel> = config
        .floors
        .iter()
        .map(|f| FloorLevel {
            id: f.id.clone(),
            name: f.name.clone(),
            y_min: f.y_min,
            y_max: f.y_max,
            image_path: format!("floors/{}", f.id),
            is_default: f.is_default,
        })
        .collect();

    MapMetadata {
        transform,
        world_bounds,
        composite_width,
        composite_height,
        floors,
        tile_pyramid,
        health_packs: Vec::new(),  // Populated from entities.json later
        connections: Vec::new(),   // Populated from entities.json later
    }
}

/// Write metadata.json to the output directory.
pub fn write_metadata(metadata: &MapMetadata, output_dir: &Path) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(metadata)?;
    let path = output_dir.join("metadata.json");
    std::fs::write(&path, &json)?;
    tracing::info!("Wrote metadata to {:?}", path);
    Ok(())
}

/// Estimate world bounds from mesh triangles (using X and Z coordinates).
/// glTF is Y-up, so top-down view is X/Z plane.
pub fn estimate_world_bounds(triangles: &[crate::mesh::Triangle]) -> WorldBounds {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    let mut z_min = f64::INFINITY;
    let mut z_max = f64::NEG_INFINITY;

    for tri in triangles {
        for v in [tri.v0, tri.v1, tri.v2] {
            x_min = x_min.min(v.x as f64);
            x_max = x_max.max(v.x as f64);
            z_min = z_min.min(v.z as f64);
            z_max = z_max.max(v.z as f64);
        }
    }

    WorldBounds { x_min, x_max, z_min, z_max }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use glam::Vec3;

    fn test_config() -> MapConfig {
        MapConfig {
            map: MapInfo {
                name: "Test".into(),
                id: "test".into(),
                game_mode: "escort".into(),
                blend_file: String::new(),
            },
            cleanup: CleanupConfig::default(),
            detection: DetectionConfig::default(),
            render: RenderConfig { pixels_per_meter: 32.0, camera_padding: 5.0 },
            tiles: TileConfig { tile_size: 256, max_zoom: 3 },
            floors: vec![
                FloorConfig { id: "ground".into(), name: "Ground".into(), y_min: 0.0, y_max: 4.0, is_default: true },
                FloorConfig { id: "upper".into(), name: "Upper".into(), y_min: 4.0, y_max: 8.0, is_default: false },
            ],
        }
    }

    #[test]
    fn build_metadata_basic() {
        let config = test_config();
        let bounds = WorldBounds { x_min: -40.0, x_max: 40.0, z_min: -30.0, z_max: 30.0 };
        let sizes = vec![
            ("ground".into(), 2560u32, 1920u32),
            ("upper".into(), 2560u32, 1920u32),
        ];

        let meta = build_metadata(&config, &sizes, bounds);

        assert_eq!(meta.composite_width, 2560);
        assert_eq!(meta.composite_height, 1920);
        assert_eq!(meta.floors.len(), 2);
        assert_eq!(meta.floors[0].id, "ground");
        assert!(meta.floors[0].is_default);
        assert_eq!(meta.floors[1].id, "upper");
        assert!(meta.tile_pyramid.is_some());

        let tp = meta.tile_pyramid.unwrap();
        assert_eq!(tp.tile_size, 256);
        assert_eq!(tp.max_zoom, 3);
    }

    #[test]
    fn metadata_serializes_to_json() {
        let config = test_config();
        let bounds = WorldBounds { x_min: -40.0, x_max: 40.0, z_min: -30.0, z_max: 30.0 };
        let sizes = vec![("ground".into(), 2560u32, 1920u32)];

        let meta = build_metadata(&config, &sizes, bounds);
        let json = serde_json::to_string_pretty(&meta).unwrap();

        // Should be deserializable back
        let deserialized: MapMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.floors.len(), 2);
        assert_eq!(deserialized.world_bounds.x_min, -40.0);
    }

    #[test]
    fn estimate_bounds_from_triangles() {
        let triangles = vec![
            crate::mesh::Triangle::new(
                Vec3::new(-10.0, 0.0, -5.0),
                Vec3::new(10.0, 0.0, -5.0),
                Vec3::new(0.0, 0.0, 5.0),
            ),
        ];
        let bounds = estimate_world_bounds(&triangles);
        assert_eq!(bounds.x_min, -10.0);
        assert_eq!(bounds.x_max, 10.0);
        assert_eq!(bounds.z_min, -5.0);
        assert_eq!(bounds.z_max, 5.0);
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p scuffed-map-pipeline metadata`
Expected: all 3 tests pass

**Step 3: Update `lib.rs`**

```rust
pub mod config;
pub mod floor_detect;
pub mod histogram;
pub mod mesh;
pub mod metadata;
pub mod tiles;
```

**Step 4: Commit**

```bash
git add crates/map-pipeline/src/metadata.rs crates/map-pipeline/src/lib.rs
git commit -m "feat(map-pipeline): add metadata generation compatible with MapMetadata"
```

---

## Task 8: CLI with clap subcommands

**Files:**
- Modify: `crates/map-pipeline/src/main.rs`

Wire up all modules into three CLI subcommands: `detect-floors`, `generate-tiles`, `process-map`.

**Step 1: Write the CLI**

Replace `crates/map-pipeline/src/main.rs`:

```rust
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

use scuffed_map_pipeline::config::MapConfig;
use scuffed_map_pipeline::floor_detect;
use scuffed_map_pipeline::mesh;
use scuffed_map_pipeline::metadata;
use scuffed_map_pipeline::tiles;

#[derive(Parser)]
#[command(name = "scuffed-map-pipeline")]
#[command(about = "Generate 2D tactical map tiles from 3D Overwatch map models")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Detect floor levels from glTF geometry
    DetectFloors {
        /// Path to the .glb file
        #[arg(long)]
        glb: PathBuf,

        /// Output TOML config path (created or updated with detected floors)
        #[arg(long)]
        output: PathBuf,

        /// Map name (used if creating a new config)
        #[arg(long, default_value = "Unknown Map")]
        name: String,

        /// Map ID (used if creating a new config)
        #[arg(long, default_value = "unknown")]
        id: String,
    },

    /// Generate tile pyramid from rendered floor PNGs
    GenerateTiles {
        /// Path to TOML config file
        #[arg(long)]
        config: PathBuf,

        /// Directory containing floor PNGs (named {floor_id}.png)
        #[arg(long)]
        images: PathBuf,

        /// Output directory for tiles and metadata
        #[arg(long)]
        output: PathBuf,
    },

    /// Run full pipeline: detect floors + generate tiles
    ProcessMap {
        /// Path to the .glb file
        #[arg(long)]
        glb: PathBuf,

        /// Directory containing floor PNGs
        #[arg(long)]
        images: PathBuf,

        /// Output directory
        #[arg(long)]
        output: PathBuf,

        /// Path to TOML config (optional — will detect floors and create if missing)
        #[arg(long)]
        config: Option<PathBuf>,

        /// Map name
        #[arg(long, default_value = "Unknown Map")]
        name: String,

        /// Map ID
        #[arg(long, default_value = "unknown")]
        id: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("scuffed_map_pipeline=info".parse()?))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::DetectFloors { glb, output, name, id } => {
            cmd_detect_floors(&glb, &output, &name, &id)
        }
        Commands::GenerateTiles { config, images, output } => {
            cmd_generate_tiles(&config, &images, &output)
        }
        Commands::ProcessMap { glb, images, output, config, name, id } => {
            cmd_process_map(&glb, &images, &output, config.as_deref(), &name, &id)
        }
    }
}

fn cmd_detect_floors(glb: &PathBuf, output: &PathBuf, name: &str, id: &str) -> Result<()> {
    tracing::info!("Loading mesh from {:?}", glb);
    let triangles = mesh::load_glb(glb)?;

    // Load existing config or create new one
    let mut config = if output.exists() {
        let content = std::fs::read_to_string(output)?;
        MapConfig::from_toml(&content)?
    } else {
        MapConfig {
            map: scuffed_map_pipeline::config::MapInfo {
                name: name.into(),
                id: id.into(),
                game_mode: String::new(),
                blend_file: String::new(),
            },
            cleanup: Default::default(),
            detection: Default::default(),
            render: Default::default(),
            tiles: Default::default(),
            floors: Vec::new(),
        }
    };

    let result = floor_detect::detect_floors(&triangles, &config.detection)?;
    floor_detect::print_histogram(&result);

    config.floors = result.floors;

    let toml_str = config.to_toml()?;
    std::fs::write(output, &toml_str)?;
    tracing::info!("Wrote config to {:?}", output);

    println!("\nDetected {} floors. Review and edit {:?} before generating tiles.", config.floors.len(), output);

    Ok(())
}

fn cmd_generate_tiles(config_path: &PathBuf, images_dir: &PathBuf, output_dir: &PathBuf) -> Result<()> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read config: {:?}", config_path))?;
    let config = MapConfig::from_toml(&content)?;

    if config.floors.is_empty() {
        anyhow::bail!("No floors defined in config. Run detect-floors first.");
    }

    std::fs::create_dir_all(output_dir)?;

    let mut floor_sizes = Vec::new();

    for floor in &config.floors {
        let img_path = images_dir.join(format!("{}.png", floor.id));
        if !img_path.exists() {
            tracing::warn!("Floor image not found: {:?}, skipping", img_path);
            continue;
        }

        let (w, h) = tiles::generate_tile_pyramid(
            &img_path,
            output_dir,
            &floor.id,
            config.tiles.tile_size,
            Some(config.tiles.max_zoom),
        )?;

        floor_sizes.push((floor.id.clone(), w, h));
    }

    if floor_sizes.is_empty() {
        anyhow::bail!("No floor images found in {:?}. Expected files like ground.png, upper.png", images_dir);
    }

    // Generate thumbnail from the default floor
    let default_floor = config.floors.iter()
        .find(|f| f.is_default)
        .unwrap_or(&config.floors[0]);
    let default_img = images_dir.join(format!("{}.png", default_floor.id));
    if default_img.exists() {
        tiles::generate_thumbnail(&default_img, &output_dir.join("thumbnail.webp"), 512)?;
    }

    // Build and write metadata
    // Estimate world bounds from render config and image dimensions
    let (max_w, max_h) = floor_sizes.iter()
        .fold((0u32, 0u32), |(w, h), (_, fw, fh)| (w.max(*fw), h.max(*fh)));
    let world_width = max_w as f64 / config.render.pixels_per_meter;
    let world_height = max_h as f64 / config.render.pixels_per_meter;
    let world_bounds = scuffed_types::WorldBounds {
        x_min: -world_width / 2.0,
        x_max: world_width / 2.0,
        z_min: -world_height / 2.0,
        z_max: world_height / 2.0,
    };

    let meta = metadata::build_metadata(&config, &floor_sizes, world_bounds);
    metadata::write_metadata(&meta, output_dir)?;

    println!("\nGenerated tiles for {} floors in {:?}", floor_sizes.len(), output_dir);

    Ok(())
}

fn cmd_process_map(
    glb: &PathBuf,
    images_dir: &PathBuf,
    output_dir: &PathBuf,
    config_path: Option<&std::path::Path>,
    name: &str,
    id: &str,
) -> Result<()> {
    let config_path_buf;
    let config_path = match config_path {
        Some(p) => p,
        None => {
            config_path_buf = output_dir.join("config.toml");
            &config_path_buf
        }
    };

    // Step 1: Detect floors (if config doesn't already have floors)
    if !config_path.exists() || {
        let content = std::fs::read_to_string(config_path).unwrap_or_default();
        MapConfig::from_toml(&content).map(|c| c.floors.is_empty()).unwrap_or(true)
    } {
        tracing::info!("No existing floor config — running detection");
        cmd_detect_floors(&glb.clone(), &config_path.to_path_buf(), name, id)?;
        println!("\nFloor detection complete. Review the config at {:?} then re-run to generate tiles.", config_path);
        return Ok(());
    }

    // Step 2: Generate tiles
    cmd_generate_tiles(&config_path.to_path_buf(), images_dir, output_dir)?;

    Ok(())
}
```

**Step 2: Verify it compiles and help text works**

Run: `cargo build -p scuffed-map-pipeline`
Expected: compiles with no errors

Run: `cargo run -p scuffed-map-pipeline -- --help`
Expected: shows subcommands: detect-floors, generate-tiles, process-map

**Step 3: Commit**

```bash
git add crates/map-pipeline/src/main.rs
git commit -m "feat(map-pipeline): add CLI with detect-floors, generate-tiles, process-map subcommands"
```

---

## Task 9: Blender Python export script

**Files:**
- Create: `scripts/export_map.py`

This is a standalone Blender Python script, not part of the Rust crate.

**Step 1: Create the script**

Create `scripts/export_map.py`:

```python
"""
Blender script to export OW maps for the tactical map pipeline.

Usage:
    blender -b map.blend -P scripts/export_map.py -- \
        --config maps/kings_row.toml \
        --output ./out/kings_row/

Requires: Blender 5.0+, io_scene_owm addon installed.
"""

import bpy
import sys
import os
import json
import math
import argparse

# ─────────────────────────────────────────────────────
# Argument parsing (args after --)
# ─────────────────────────────────────────────────────

def parse_args():
    argv = sys.argv
    if "--" in argv:
        argv = argv[argv.index("--") + 1:]
    else:
        argv = []

    parser = argparse.ArgumentParser(description="Export OW map for tactical pipeline")
    parser.add_argument("--config", required=True, help="Path to map TOML config")
    parser.add_argument("--output", required=True, help="Output directory")
    parser.add_argument("--export-glb", action="store_true", help="Also export .glb geometry")
    parser.add_argument("--skip-render", action="store_true", help="Skip rendering floor PNGs")
    return parser.parse_args(argv)


# ─────────────────────────────────────────────────────
# TOML parsing (minimal, no external deps)
# ─────────────────────────────────────────────────────

def parse_toml_simple(path):
    """Minimal TOML parser for our config format. Handles sections, key=value, [[arrays]]."""
    config = {}
    current_section = None
    current_array = None

    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue

            if line.startswith("[[") and line.endswith("]]"):
                key = line[2:-2].strip()
                if key not in config:
                    config[key] = []
                current_section = {}
                config[key].append(current_section)
                current_array = key
                continue

            if line.startswith("[") and line.endswith("]"):
                key = line[1:-1].strip()
                config[key] = {}
                current_section = config[key]
                current_array = None
                continue

            if "=" in line and current_section is not None:
                k, v = line.split("=", 1)
                k = k.strip()
                v = v.strip()
                # Parse value
                if v.startswith('"') and v.endswith('"'):
                    v = v[1:-1]
                elif v == "true":
                    v = True
                elif v == "false":
                    v = False
                else:
                    try:
                        v = float(v) if "." in v else int(v)
                    except ValueError:
                        pass
                current_section[k] = v

    return config


# ─────────────────────────────────────────────────────
# Scene cleanup
# ─────────────────────────────────────────────────────

def cleanup_scene(cleanup_config):
    """Remove non-gameplay objects from the scene."""
    max_dist = cleanup_config.get("max_distance_from_center", 200.0)
    min_size = cleanup_config.get("min_object_size", 0.01)
    skybox_threshold = cleanup_config.get("skybox_size_threshold", 500.0)

    removed = 0

    # Remove lights
    if cleanup_config.get("remove_lights", True):
        for obj in list(bpy.data.objects):
            if obj.type == "LIGHT":
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1

        # Remove "Lights" collection if it exists
        if "Lights" in bpy.data.collections:
            col = bpy.data.collections["Lights"]
            for obj in list(col.objects):
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1
            bpy.data.collections.remove(col)

    # Remove cameras
    if cleanup_config.get("remove_cameras", True):
        for obj in list(bpy.data.objects):
            if obj.type == "CAMERA":
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1

    # Remove particles
    if cleanup_config.get("remove_particles", True):
        for obj in list(bpy.data.objects):
            if obj.particle_systems:
                bpy.data.objects.remove(obj, do_unlink=True)
                removed += 1

    # Remove skybox (objects larger than threshold)
    for obj in list(bpy.data.objects):
        if obj.type != "MESH":
            continue
        dims = obj.dimensions
        if max(dims.x, dims.y, dims.z) > skybox_threshold:
            print(f"  Removing skybox: {obj.name} (dims: {dims.x:.0f}x{dims.y:.0f}x{dims.z:.0f})")
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1

    # Remove OOB objects (too far from center)
    for obj in list(bpy.data.objects):
        if obj.type != "MESH":
            continue
        loc = obj.location
        dist = math.sqrt(loc.x**2 + loc.y**2 + loc.z**2)
        if dist > max_dist:
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1

    # Remove tiny objects
    for obj in list(bpy.data.objects):
        if obj.type != "MESH":
            continue
        dims = obj.dimensions
        if max(dims.x, dims.y, dims.z) < min_size:
            bpy.data.objects.remove(obj, do_unlink=True)
            removed += 1

    print(f"  Cleanup: removed {removed} objects")


# ─────────────────────────────────────────────────────
# Rendering
# ─────────────────────────────────────────────────────

def get_scene_bounds():
    """Get the XZ bounding box of all mesh objects (Y-up)."""
    x_min = z_min = float("inf")
    x_max = z_max = float("-inf")

    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        # Get world-space bounding box corners
        for corner in obj.bound_box:
            world = obj.matrix_world @ bpy.mathutils.Vector(corner)
            # Blender is Z-up internally, but glTF export converts
            # We work in Blender's native Z-up here
            x_min = min(x_min, world.x)
            x_max = max(x_max, world.x)
            # For top-down view in Blender (Z-up), Y is the "depth" axis
            z_min = min(z_min, world.y)
            z_max = max(z_max, world.y)

    return x_min, x_max, z_min, z_max


def setup_workbench_render(render_config):
    """Configure Workbench engine for fast textured rendering."""
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"

    # Solid mode settings
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "TEXTURE"
    scene.display.shading.show_shadows = False
    scene.display.shading.show_cavity = False

    # Transparent background
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"

    # Resolution
    ppm = render_config.get("pixels_per_meter", 32)
    x_min, x_max, z_min, z_max = get_scene_bounds()
    padding = render_config.get("camera_padding", 5.0)

    width_m = (x_max - x_min) + 2 * padding
    height_m = (z_max - z_min) + 2 * padding

    scene.render.resolution_x = int(width_m * ppm)
    scene.render.resolution_y = int(height_m * ppm)
    scene.render.resolution_percentage = 100

    return x_min, x_max, z_min, z_max, padding


def setup_camera(x_min, x_max, z_min, z_max, padding):
    """Create orthographic top-down camera."""
    # Remove existing cameras
    for obj in list(bpy.data.objects):
        if obj.type == "CAMERA":
            bpy.data.objects.remove(obj, do_unlink=True)

    cam_data = bpy.data.cameras.new("PipelineCamera")
    cam_data.type = "ORTHO"

    width_m = (x_max - x_min) + 2 * padding
    height_m = (z_max - z_min) + 2 * padding
    cam_data.ortho_scale = max(width_m, height_m)

    cam_obj = bpy.data.objects.new("PipelineCamera", cam_data)
    bpy.context.scene.collection.objects.link(cam_obj)
    bpy.context.scene.camera = cam_obj

    # Position camera above center, looking down (Blender Z-up)
    center_x = (x_min + x_max) / 2
    center_y = (z_min + z_max) / 2
    cam_obj.location = (center_x, center_y, 100)  # High above
    cam_obj.rotation_euler = (0, 0, 0)  # Looking down -Z

    return cam_obj


def hide_objects_outside_floor(floor_config):
    """Hide mesh objects whose Z-center (Blender Z-up) is outside the floor range.

    Note: In Blender Z is up, but our config uses Y (glTF convention).
    Blender Z maps to glTF Y. So floor y_min/y_max correspond to Blender Z.
    """
    y_min = floor_config.get("y_min", float("-inf"))
    y_max = floor_config.get("y_max", float("inf"))

    hidden = 0
    shown = 0
    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        # Object center Z in world space (Blender Z = height)
        z_center = obj.matrix_world.translation.z
        if z_center < y_min or z_center > y_max:
            obj.hide_render = True
            hidden += 1
        else:
            obj.hide_render = False
            shown += 1

    print(f"  Floor [{y_min:.1f}, {y_max:.1f}]: showing {shown}, hiding {hidden} objects")


def render_floor(floor_config, output_dir, render_bounds):
    """Render a single floor to PNG."""
    floor_id = floor_config["id"]
    output_path = os.path.join(output_dir, f"{floor_id}.png")

    hide_objects_outside_floor(floor_config)

    bpy.context.scene.render.filepath = output_path
    bpy.ops.render.render(write_still=True)

    print(f"  Rendered floor '{floor_id}' -> {output_path}")


# ─────────────────────────────────────────────────────
# glTF export
# ─────────────────────────────────────────────────────

def export_glb(output_dir, map_id):
    """Export the scene as GLB."""
    # Show all objects for export
    for obj in bpy.data.objects:
        obj.hide_render = False

    output_path = os.path.join(output_dir, f"{map_id}.glb")

    bpy.ops.export_scene.gltf(
        filepath=output_path,
        export_format="GLB",
        use_selection=False,
        export_apply=True,
        export_materials="PLACEHOLDER",
        export_draco_mesh_compression_enable=True,
    )

    print(f"  Exported GLB -> {output_path}")


# ─────────────────────────────────────────────────────
# Entity export
# ─────────────────────────────────────────────────────

def export_entities(output_dir):
    """Export entity positions (health packs, spawns) as JSON.

    Entities imported by io_scene_owm are typically Empty objects.
    This is a best-effort extraction — GUIDs need manual mapping.
    """
    entities = {
        "health_packs": [],
        "spawns": [],
        "objectives": [],
        "empties": [],  # Raw empty positions for manual review
    }

    for obj in bpy.data.objects:
        if obj.type == "EMPTY":
            loc = obj.matrix_world.translation
            entities["empties"].append({
                "name": obj.name,
                "x": round(loc.x, 3),
                "y": round(loc.z, 3),  # Blender Z -> glTF Y (height)
                "z": round(loc.y, 3),  # Blender Y -> glTF Z (depth)
            })

    output_path = os.path.join(output_dir, "entities.json")
    with open(output_path, "w") as f:
        json.dump(entities, f, indent=2)

    print(f"  Exported {len(entities['empties'])} entities -> {output_path}")


# ─────────────────────────────────────────────────────
# Main
# ─────────────────────────────────────────────────────

def main():
    args = parse_args()

    print(f"\n{'='*60}")
    print(f"  Scuffed Map Pipeline — Blender Export")
    print(f"{'='*60}\n")

    # Parse config
    config = parse_toml_simple(args.config)
    map_info = config.get("map", {})
    cleanup_config = config.get("cleanup", {})
    render_config = config.get("render", {})
    floors = config.get("floors", [])

    map_id = map_info.get("id", "unknown")
    print(f"Map: {map_info.get('name', 'Unknown')} ({map_id})")

    # Create output directory
    os.makedirs(args.output, exist_ok=True)

    # Cleanup
    print("\nStep 1: Scene cleanup")
    cleanup_scene(cleanup_config)

    # Export GLB if requested
    if args.export_glb:
        print("\nStep 2: GLB export")
        export_glb(args.output, map_id)

    # Render floors
    if not args.skip_render:
        if not floors:
            print("\nERROR: No floors defined in config. Run 'detect-floors' first.")
            sys.exit(1)

        print(f"\nStep 3: Rendering {len(floors)} floors (Workbench engine)")
        bounds = setup_workbench_render(render_config)
        setup_camera(*bounds)

        for floor in floors:
            render_floor(floor, args.output, bounds)

    # Export entities
    print("\nStep 4: Entity export")
    export_entities(args.output)

    # Restore visibility
    for obj in bpy.data.objects:
        obj.hide_render = False

    print(f"\n{'='*60}")
    print(f"  Done! Output in: {args.output}")
    print(f"{'='*60}\n")


if __name__ == "__main__":
    main()
```

**Step 2: Verify the script has valid Python syntax**

Run: `python3 -c "import ast; ast.parse(open('scripts/export_map.py').read()); print('OK')"` from project root
Expected: `OK`

**Step 3: Commit**

```bash
git add scripts/export_map.py
git commit -m "feat: add Blender export script for tactical map pipeline"
```

---

## Task 10: Integration test with synthetic data

**Files:**
- Create: `crates/map-pipeline/tests/integration.rs`

End-to-end test: create synthetic floor images, run tile generation, verify output structure and metadata.

**Step 1: Write integration test**

Create `crates/map-pipeline/tests/integration.rs`:

```rust
use image::RgbaImage;
use scuffed_map_pipeline::config::*;
use scuffed_map_pipeline::metadata;
use scuffed_map_pipeline::tiles;
use scuffed_types::WorldBounds;
use std::path::Path;

fn create_test_floor_image(path: &Path, width: u32, height: u32, r: u8, g: u8, b: u8) {
    let img = RgbaImage::from_fn(width, height, |_, _| image::Rgba([r, g, b, 255]));
    img.save(path).unwrap();
}

#[test]
fn end_to_end_tile_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let images_dir = tmp.path().join("images");
    let output_dir = tmp.path().join("output");
    std::fs::create_dir_all(&images_dir).unwrap();
    std::fs::create_dir_all(&output_dir).unwrap();

    // Create test floor images (different colors)
    create_test_floor_image(&images_dir.join("ground.png"), 1024, 768, 200, 200, 200);
    create_test_floor_image(&images_dir.join("upper.png"), 1024, 768, 150, 150, 200);

    let config = MapConfig {
        map: MapInfo {
            name: "Test Map".into(),
            id: "test".into(),
            game_mode: "escort".into(),
            blend_file: String::new(),
        },
        cleanup: CleanupConfig::default(),
        detection: DetectionConfig::default(),
        render: RenderConfig { pixels_per_meter: 32.0, camera_padding: 5.0 },
        tiles: TileConfig { tile_size: 256, max_zoom: 2 },
        floors: vec![
            FloorConfig {
                id: "ground".into(),
                name: "Ground".into(),
                y_min: 0.0,
                y_max: 4.0,
                is_default: true,
            },
            FloorConfig {
                id: "upper".into(),
                name: "Upper".into(),
                y_min: 4.0,
                y_max: 8.0,
                is_default: false,
            },
        ],
    };

    // Generate tiles for each floor
    let mut floor_sizes = Vec::new();
    for floor in &config.floors {
        let img_path = images_dir.join(format!("{}.png", floor.id));
        let (w, h) = tiles::generate_tile_pyramid(
            &img_path,
            &output_dir,
            &floor.id,
            config.tiles.tile_size,
            Some(config.tiles.max_zoom),
        )
        .unwrap();
        floor_sizes.push((floor.id.clone(), w, h));
    }

    // Generate thumbnail
    tiles::generate_thumbnail(
        &images_dir.join("ground.png"),
        &output_dir.join("thumbnail.webp"),
        512,
    )
    .unwrap();

    // Generate metadata
    let bounds = WorldBounds {
        x_min: -16.0,
        x_max: 16.0,
        z_min: -12.0,
        z_max: 12.0,
    };
    let meta = metadata::build_metadata(&config, &floor_sizes, bounds);
    metadata::write_metadata(&meta, &output_dir).unwrap();

    // Verify output structure
    assert!(output_dir.join("metadata.json").exists());
    assert!(output_dir.join("thumbnail.webp").exists());

    // Verify tile pyramid for ground floor
    assert!(output_dir.join("floors/ground/0/0/0.webp").exists()); // zoom 0
    assert!(output_dir.join("floors/ground/2/0/0.webp").exists()); // zoom 2

    // Verify tile pyramid for upper floor
    assert!(output_dir.join("floors/upper/0/0/0.webp").exists());

    // Verify metadata content
    let meta_json = std::fs::read_to_string(output_dir.join("metadata.json")).unwrap();
    let loaded: scuffed_types::MapMetadata = serde_json::from_str(&meta_json).unwrap();
    assert_eq!(loaded.floors.len(), 2);
    assert_eq!(loaded.floors[0].id, "ground");
    assert!(loaded.floors[0].is_default);
    assert_eq!(loaded.composite_width, 1024);
    assert_eq!(loaded.composite_height, 768);
    assert!(loaded.tile_pyramid.is_some());
}
```

**Step 2: Run the integration test**

Run: `cargo test -p scuffed-map-pipeline --test integration`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/map-pipeline/tests/integration.rs
git commit -m "test(map-pipeline): add end-to-end integration test with synthetic data"
```

---

## Task 11: Run all tests and verify

**Step 1: Run the full test suite**

Run: `cargo test -p scuffed-map-pipeline`
Expected: All unit tests + integration test pass

**Step 2: Run the CLI help**

Run: `cargo run -p scuffed-map-pipeline -- detect-floors --help`
Expected: shows --glb, --output, --name, --id flags

Run: `cargo run -p scuffed-map-pipeline -- generate-tiles --help`
Expected: shows --config, --images, --output flags

**Step 3: Verify no warnings**

Run: `cargo clippy -p scuffed-map-pipeline`
Expected: no warnings (fix any that appear)

**Step 4: Final commit if clippy fixes were needed**

```bash
git add -A crates/map-pipeline/
git commit -m "fix(map-pipeline): address clippy warnings"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | Scaffold crate | `crates/map-pipeline/Cargo.toml`, `src/lib.rs`, `src/main.rs`, root `Cargo.toml` |
| 2 | Config types | `src/config.rs` — TOML parsing with defaults |
| 3 | Histogram utils | `src/histogram.rs` — build, smooth, find valleys |
| 4 | glTF loading | `src/mesh.rs` — Triangle struct, walkable detection, glTF import |
| 5 | Floor detection | `src/floor_detect.rs` — histogram → peaks → floor ranges |
| 6 | Tile generation | `src/tiles.rs` — PNG → WebP tile pyramid with rayon |
| 7 | Metadata | `src/metadata.rs` — MapMetadata JSON generation |
| 8 | CLI | `src/main.rs` — clap subcommands wiring everything together |
| 9 | Blender script | `scripts/export_map.py` — cleanup + Workbench render + entity export |
| 10 | Integration test | `tests/integration.rs` — end-to-end synthetic test |
| 11 | Verification | Run all tests, clippy, CLI help |

**Total files:** 8 Rust source files + 1 test file + 1 Python script + 2 config files (Cargo.toml)

**Dependencies added:** `gltf`, `glam`, `find_peaks`, `image`, `fast_image_resize`, `rayon`, `rustc-hash`, `clap`, `toml`, `anyhow`, `tracing`, `tracing-subscriber`, `tempfile` (dev)
