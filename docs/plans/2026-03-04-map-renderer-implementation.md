# Map Renderer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace Blender's Workbench renderer with a Bevy 0.18 headless renderer that loads a GLB, renders orthographic top-down floor views, and outputs RGBA PNGs.

**Architecture:** New `crates/map-renderer` crate using Bevy 0.18 in headless mode (no window). Loads GLB via Bevy's asset system, sets up an orthographic camera looking down -Y, filters mesh visibility by floor Y range, renders to an offscreen texture, reads back to CPU, and saves as PNG. Reuses `MapConfig`/`FloorConfig`/`RenderConfig` types from `map-pipeline::config`.

**Tech Stack:** Bevy 0.18, crossbeam-channel (for GPU→CPU image readback), image crate (PNG save), clap (CLI), map-pipeline (config types)

---

### Task 1: Scaffold crate and add to workspace

**Files:**
- Create: `crates/map-renderer/Cargo.toml`
- Create: `crates/map-renderer/src/lib.rs`
- Create: `crates/map-renderer/src/main.rs`
- Modify: `Cargo.toml` (workspace root)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "scuffed-map-renderer"
version = "0.1.0"
edition = "2024"

[dependencies]
scuffed-map-pipeline = { path = "../map-pipeline" }

# Rendering engine
bevy = { version = "0.18", default-features = false, features = [
    "bevy_render",
    "bevy_core_pipeline",
    "bevy_pbr",
    "bevy_gltf",
    "bevy_scene",
    "bevy_asset",
    "bevy_log",
    "multi_threaded",
    "png",
] }
crossbeam-channel = "0.5"

# Image export
image = { version = "0.25", default-features = false, features = ["png"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Config
toml = "0.8"
serde = { workspace = true }

# Error handling + logging
anyhow = "1"
tracing = { workspace = true }
```

**Step 2: Create minimal lib.rs**

```rust
pub mod plugin;
```

**Step 3: Create minimal main.rs**

```rust
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "scuffed-map-renderer")]
#[command(about = "Render orthographic floor views from GLB models using Bevy")]
struct Cli {
    /// Path to the .glb file
    #[arg(long)]
    glb: PathBuf,

    /// Path to map TOML config with floor definitions
    #[arg(long)]
    config: PathBuf,

    /// Output directory for floor PNGs
    #[arg(long)]
    output: PathBuf,

    /// Render only this floor ID (renders all if omitted)
    #[arg(long)]
    floor: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    println!("GLB: {:?}, Config: {:?}, Output: {:?}", cli.glb, cli.config, cli.output);
    Ok(())
}
```

**Step 4: Add to workspace**

Add `"crates/map-renderer"` to the `members` list in the root `Cargo.toml`, under the `# Map pipeline tooling` comment.

**Step 5: Verify it builds**

Run: `cargo build -p scuffed-map-renderer`
Expected: builds successfully

**Step 6: Commit**

```bash
git add crates/map-renderer/ Cargo.toml
git commit -m "feat: scaffold map-renderer crate with Bevy 0.18"
```

---

### Task 2: Headless Bevy app with offscreen render target

This task sets up the core Bevy headless infrastructure: an app that creates an offscreen render target, renders one frame, reads it back to CPU, and exits. No GLB loading yet — just a colored background to verify the pipeline works.

**Files:**
- Create: `crates/map-renderer/src/plugin.rs`
- Modify: `crates/map-renderer/src/lib.rs`
- Modify: `crates/map-renderer/src/main.rs`

**Step 1: Create plugin.rs with the headless render infrastructure**

This is adapted from Bevy's official `headless_renderer.rs` example (v0.18.1). The key components are:

1. `ImageCopyPlugin` — render graph node that copies GPU texture to a CPU-readable buffer each frame
2. `CaptureFramePlugin` — receives the CPU buffer data via crossbeam channel and saves to PNG
3. `RenderJob` resource — tracks what to render and when to exit

```rust
//! Bevy headless rendering plugin.
//!
//! Adapted from Bevy's `headless_renderer.rs` example (v0.18.1).
//! Sets up offscreen render-to-texture and GPU→CPU readback via crossbeam channel.

use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    core_pipeline::tonemapping::Tonemapping,
    image::TextureFormatPixelInfo,
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_graph::{self, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel},
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode,
            PollType, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat, TextureUsages,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::GpuImage,
        Extract, Render, RenderApp, RenderSystems,
    },
    window::ExitCondition,
    winit::WinitPlugin,
};
use crossbeam_channel::{Receiver, Sender};
use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

// ── Resources ──────────────────────────────────────────────────────

/// Tracks the render job state.
#[derive(Resource)]
pub struct RenderJob {
    /// Frames to wait before capturing (scene needs time to load).
    pub warmup_frames: u32,
    /// Output path for the current render.
    pub output_path: PathBuf,
    /// Width/height of the render target.
    pub width: u32,
    pub height: u32,
}

#[derive(Resource, Deref)]
struct MainWorldReceiver(Receiver<Vec<u8>>);

#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<Vec<u8>>);

// ── Render target setup ────────────────────────────────────────────

/// Marker for the CPU-side image used for readback.
#[derive(Component, Deref, DerefMut)]
pub struct ImageToSave(pub Handle<Image>);

/// Creates the offscreen render target and camera. Call this in your Startup system.
pub fn create_render_target(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    render_device: &RenderDevice,
    width: u32,
    height: u32,
) -> Handle<Image> {
    // GPU render target
    let mut render_target_image = Image::new_target_texture(
        width,
        height,
        TextureFormat::bevy_default(),
        None,
    );
    render_target_image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let render_target_handle = images.add(render_target_image);

    // CPU-side image for readback
    let cpu_image = Image::new_target_texture(
        width,
        height,
        TextureFormat::bevy_default(),
        None,
    );
    let cpu_image_handle = images.add(cpu_image);

    // Image copier (GPU → CPU buffer)
    commands.spawn(ImageCopier::new(
        render_target_handle.clone(),
        Extent3d { width, height, ..default() },
        render_device,
    ));

    // CPU image for saving
    commands.spawn(ImageToSave(cpu_image_handle));

    render_target_handle
}

// ── ImageCopyPlugin ────────────────────────────────────────────────
// Copies the GPU render target to a CPU-readable buffer each frame.

pub struct ImageCopyPlugin;

impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (s, r) = crossbeam_channel::unbounded();
        let render_app = app
            .insert_resource(MainWorldReceiver(r))
            .sub_app_mut(RenderApp);

        let mut graph = render_app.world_mut().resource_mut::<RenderGraph>();
        graph.add_node(ImageCopy, ImageCopyDriver);
        graph.add_node_edge(bevy::render::graph::CameraDriverLabel, ImageCopy);

        render_app
            .insert_resource(RenderWorldSender(s))
            .add_systems(ExtractSchedule, image_copy_extract)
            .add_systems(Render, receive_image_from_buffer.after(RenderSystems::Render));
    }
}

#[derive(Clone, Default, Resource, Deref, DerefMut)]
struct ImageCopiers(pub Vec<ImageCopier>);

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    enabled: Arc<AtomicBool>,
    src_image: Handle<Image>,
}

impl ImageCopier {
    pub fn new(
        src_image: Handle<Image>,
        size: Extent3d,
        render_device: &RenderDevice,
    ) -> Self {
        let padded_bytes_per_row =
            RenderDevice::align_copy_bytes_per_row((size.width) as usize) * 4;
        let cpu_buffer = render_device.create_buffer(&BufferDescriptor {
            label: None,
            size: padded_bytes_per_row as u64 * size.height as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ImageCopier {
            buffer: cpu_buffer,
            src_image,
            enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

fn image_copy_extract(mut commands: Commands, image_copiers: Extract<Query<&ImageCopier>>) {
    commands.insert_resource(ImageCopiers(
        image_copiers.iter().cloned().collect(),
    ));
}

#[derive(Debug, PartialEq, Eq, Clone, Hash, RenderLabel)]
struct ImageCopy;

#[derive(Default)]
struct ImageCopyDriver;

impl render_graph::Node for ImageCopyDriver {
    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let image_copiers = world.get_resource::<ImageCopiers>().unwrap();
        let gpu_images = world.get_resource::<RenderAssets<GpuImage>>().unwrap();

        for image_copier in image_copiers.iter() {
            if !image_copier.enabled() {
                continue;
            }
            let Some(src_image) = gpu_images.get(&image_copier.src_image) else {
                continue;
            };

            let mut encoder = render_context
                .render_device()
                .create_command_encoder(&CommandEncoderDescriptor::default());

            let block_dimensions = src_image.texture_format.block_dimensions();
            let block_size = src_image.texture_format.block_copy_size(None).unwrap();
            let padded_bytes_per_row = RenderDevice::align_copy_bytes_per_row(
                (src_image.size.x as usize / block_dimensions.0 as usize) * block_size as usize,
            );

            encoder.copy_texture_to_buffer(
                src_image.texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &image_copier.buffer,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(
                            std::num::NonZero::<u32>::new(padded_bytes_per_row as u32)
                                .unwrap()
                                .into(),
                        ),
                        rows_per_image: None,
                    },
                },
                src_image.size,
            );

            let render_queue = world.get_resource::<RenderQueue>().unwrap();
            render_queue.submit(std::iter::once(encoder.finish()));
        }
        Ok(())
    }
}

fn receive_image_from_buffer(
    image_copiers: Res<ImageCopiers>,
    render_device: Res<RenderDevice>,
    sender: Res<RenderWorldSender>,
) {
    for image_copier in image_copiers.0.iter() {
        if !image_copier.enabled() {
            continue;
        }
        let buffer_slice = image_copier.buffer.slice(..);
        let (s, r) = crossbeam_channel::bounded(1);
        buffer_slice.map_async(MapMode::Read, move |r| match r {
            Ok(r) => s.send(r).expect("Failed to send map update"),
            Err(err) => panic!("Failed to map buffer {err}"),
        });
        render_device
            .poll(PollType::wait_indefinitely())
            .expect("Failed to poll device");
        r.recv().expect("Failed to receive map_async");
        let _ = sender.send(buffer_slice.get_mapped_range().to_vec());
        image_copier.buffer.unmap();
    }
}

// ── CaptureFramePlugin ─────────────────────────────────────────────
// Waits for warmup frames, then saves the rendered image and exits.

pub struct CaptureFramePlugin;

impl Plugin for CaptureFramePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, capture_and_save);
    }
}

fn capture_and_save(
    images_to_save: Query<&ImageToSave>,
    receiver: Res<MainWorldReceiver>,
    mut images: ResMut<Assets<Image>>,
    mut job: ResMut<RenderJob>,
    mut app_exit_writer: EventWriter<AppExit>,
) {
    if job.warmup_frames > 0 {
        // Drain any early frames
        while receiver.try_recv().is_ok() {}
        job.warmup_frames -= 1;
        return;
    }

    // Grab the latest frame data
    let mut image_data = Vec::new();
    while let Ok(data) = receiver.try_recv() {
        image_data = data;
    }

    if image_data.is_empty() {
        return;
    }

    for image_handle in images_to_save.iter() {
        let img_bytes = images.get_mut(image_handle.id()).unwrap();

        let row_bytes =
            img_bytes.width() as usize * img_bytes.texture_descriptor.format.pixel_size().unwrap();
        let aligned_row_bytes = RenderDevice::align_copy_bytes_per_row(row_bytes);

        if row_bytes == aligned_row_bytes {
            img_bytes.data.as_mut().unwrap().clone_from(&image_data);
        } else {
            img_bytes.data = Some(
                image_data
                    .chunks(aligned_row_bytes)
                    .take(img_bytes.height() as usize)
                    .flat_map(|row| &row[..row_bytes.min(row.len())])
                    .cloned()
                    .collect(),
            );
        }

        let img = match img_bytes.clone().try_into_dynamic() {
            Ok(img) => img.to_rgba8(),
            Err(e) => panic!("Failed to create image buffer {e:?}"),
        };

        if let Some(parent) = job.output_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        img.save(&job.output_path)
            .expect("Failed to save PNG");
        info!("Saved render to {:?}", job.output_path);
    }

    app_exit_writer.send(AppExit::Success);
}

// ── App builder ────────────────────────────────────────────────────

/// Build a headless Bevy app configured for offscreen rendering.
pub fn build_headless_app() -> App {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgba(0.0, 0.0, 0.0, 0.0)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default_nearest())
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        )
        .add_plugins(ImageCopyPlugin)
        .add_plugins(CaptureFramePlugin)
        .add_plugins(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1.0 / 60.0),
        ));
    app
}
```

**Step 2: Update lib.rs**

```rust
pub mod plugin;
```

**Step 3: Update main.rs to run a smoke test**

Wire up the CLI to actually create a Bevy app with a solid-color background and save it as a PNG. This proves the headless pipeline works end-to-end before we add GLB loading.

```rust
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::camera::ScalingMode;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::renderer::RenderDevice;

use scuffed_map_renderer::plugin::{
    build_headless_app, create_render_target, RenderJob,
};

#[derive(Parser)]
#[command(name = "scuffed-map-renderer")]
#[command(about = "Render orthographic floor views from GLB models using Bevy")]
struct Cli {
    /// Path to the .glb file
    #[arg(long)]
    glb: PathBuf,

    /// Path to map TOML config with floor definitions
    #[arg(long)]
    config: PathBuf,

    /// Output directory for floor PNGs
    #[arg(long)]
    output: PathBuf,

    /// Render only this floor ID (renders all if omitted)
    #[arg(long)]
    floor: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    std::fs::create_dir_all(&cli.output)?;

    let output_path = cli.output.join("test.png");

    let mut app = build_headless_app();
    app.insert_resource(RenderJob {
        warmup_frames: 5,
        output_path,
        width: 256,
        height: 256,
    });
    app.add_systems(Startup, setup_test_scene);
    app.run();

    Ok(())
}

fn setup_test_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    render_device: Res<RenderDevice>,
    job: Res<RenderJob>,
) {
    let render_target = create_render_target(
        &mut commands,
        &mut images,
        &render_device,
        job.width,
        job.height,
    );

    // Simple test: colored plane
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(10.0, 10.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.5, 0.3),
            unlit: true,
            ..default()
        })),
    ));

    // Orthographic top-down camera
    commands.spawn((
        Camera3d::default(),
        Projection::from(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: 12.0,
                height: 12.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        bevy::camera::RenderTarget::Image(render_target.into()),
        Tonemapping::None,
        Transform::from_xyz(0.0, 50.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
    ));
}
```

**Step 4: Build and run the smoke test**

Run: `cargo build -p scuffed-map-renderer`
Expected: builds successfully

Run: `cargo run -p scuffed-map-renderer -- --glb /dev/null --config /dev/null --output /tmp/renderer-test/`
Expected: creates `/tmp/renderer-test/test.png` (256x256 image with green plane on transparent background), then exits.

If the PNG is created and has non-zero content, the headless pipeline works. If Bevy panics about missing GPU/display, check that the system has a GPU available (even headless Bevy needs a GPU via wgpu).

**Step 5: Commit**

```bash
git add crates/map-renderer/src/plugin.rs crates/map-renderer/src/main.rs crates/map-renderer/src/lib.rs
git commit -m "feat: headless Bevy render pipeline with offscreen capture"
```

---

### Task 3: GLB loading and scene bounds calculation

Load a GLB file via Bevy's asset system and compute the XZ bounding box of the scene for camera setup.

**Files:**
- Modify: `crates/map-renderer/src/plugin.rs`
- Modify: `crates/map-renderer/src/main.rs`

**Step 1: Add GLB path and scene state tracking to RenderJob**

Update `RenderJob` to hold the GLB path and track loading state:

```rust
/// Tracks the render job state.
#[derive(Resource)]
pub struct RenderJob {
    /// Frames to wait before capturing (scene needs time to load).
    pub warmup_frames: u32,
    /// Output path for the current render.
    pub output_path: PathBuf,
    /// Width/height of the render target.
    pub width: u32,
    pub height: u32,
    /// Path to the GLB file to load.
    pub glb_path: PathBuf,
    /// Camera ortho scale (world units).
    pub camera_scale_x: f32,
    pub camera_scale_y: f32,
    /// Camera center position (world XZ mapped to Bevy XZ).
    pub camera_center: Vec3,
}
```

**Step 2: Add a scene bounds system**

Add a system that runs after the scene is spawned to compute the XZ bounding box from all mesh entities. In `plugin.rs`, add:

```rust
/// Compute axis-aligned bounding box of all mesh entities in XZ plane.
pub fn compute_scene_bounds(
    meshes: Res<Assets<Mesh>>,
    query: Query<(&GlobalTransform, &Mesh3d)>,
) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found_any = false;

    for (transform, mesh_handle) in query.iter() {
        let Some(mesh) = meshes.get(&mesh_handle.0) else {
            continue;
        };
        let Some(aabb) = mesh.compute_aabb() else {
            continue;
        };

        // Transform AABB corners to world space
        let center = aabb.center;
        let half = aabb.half_extents;
        for &sx in &[-1.0f32, 1.0] {
            for &sy in &[-1.0, 1.0] {
                for &sz in &[-1.0, 1.0] {
                    let local = Vec3::new(
                        center.x + half.x * sx,
                        center.y + half.y * sy,
                        center.z + half.z * sz,
                    );
                    let world = transform.transform_point(local);
                    min = min.min(world);
                    max = max.max(world);
                }
            }
        }
        found_any = true;
    }

    if found_any { Some((min, max)) } else { None }
}
```

**Step 3: Update main.rs to load the GLB**

Replace the test scene setup with actual GLB loading:

```rust
fn setup_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    job: Res<RenderJob>,
    asset_server: Res<AssetServer>,
) {
    let render_target = create_render_target(
        &mut commands,
        &mut images,
        &render_device,
        job.width,
        job.height,
    );

    // Load GLB scene
    commands.spawn(SceneRoot(
        asset_server.load(
            GltfAssetLabel::Scene(0).from_asset(job.glb_path.to_string_lossy().to_string()),
        ),
    ));

    // Ambient light for unlit vertex colors
    commands.insert_resource(AmbientLight {
        color: Color::WHITE,
        brightness: 1000.0,
    });

    // Camera — will be repositioned once scene bounds are computed
    commands.spawn((
        Camera3d::default(),
        Projection::from(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: job.camera_scale_x,
                height: job.camera_scale_y,
            },
            ..OrthographicProjection::default_3d()
        }),
        bevy::camera::RenderTarget::Image(render_target.into()),
        Tonemapping::None,
        Transform::from_xyz(
            job.camera_center.x,
            100.0,
            job.camera_center.z,
        ).looking_at(
            Vec3::new(job.camera_center.x, 0.0, job.camera_center.z),
            Vec3::Z,
        ),
    ));
}
```

**Step 4: Build**

Run: `cargo build -p scuffed-map-renderer`
Expected: builds successfully

**Step 5: Commit**

```bash
git add crates/map-renderer/src/
git commit -m "feat: GLB loading and scene bounds calculation"
```

---

### Task 4: Floor visibility filtering

Add a system that hides/shows mesh entities based on their Y position relative to the current floor's y_min/y_max range.

**Files:**
- Modify: `crates/map-renderer/src/plugin.rs`

**Step 1: Add floor filter resource and system**

```rust
/// Current floor Y range for visibility filtering.
#[derive(Resource)]
pub struct FloorFilter {
    pub y_min: f32,
    pub y_max: f32,
}

/// Hide mesh entities whose world-space Y center is outside the floor range.
pub fn apply_floor_filter(
    filter: Res<FloorFilter>,
    mut query: Query<(&GlobalTransform, &mut Visibility), With<Mesh3d>>,
) {
    for (transform, mut visibility) in query.iter_mut() {
        let y = transform.translation().y;
        if y >= filter.y_min && y <= filter.y_max {
            *visibility = Visibility::Inherited;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}
```

**Step 2: Build**

Run: `cargo build -p scuffed-map-renderer`
Expected: builds successfully

**Step 3: Commit**

```bash
git add crates/map-renderer/src/plugin.rs
git commit -m "feat: floor visibility filtering by Y range"
```

---

### Task 5: Make materials unlit for vertex color rendering

When the GLB loads, Bevy assigns default PBR materials. We need to replace them with unlit white materials so vertex colors (COLOR_0 / AO bake data) render as-is without PBR lighting distortion.

**Files:**
- Modify: `crates/map-renderer/src/plugin.rs`

**Step 1: Add a system that switches all materials to unlit after scene load**

```rust
/// Marker to prevent re-processing materials.
#[derive(Component)]
struct UnlitApplied;

/// Replace all StandardMaterial instances with unlit white so vertex colors show as-is.
pub fn make_materials_unlit(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    query: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Without<UnlitApplied>>,
) {
    for (entity, material_handle) in query.iter() {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            material.base_color = Color::WHITE;
            material.unlit = true;
        }
        commands.entity(entity).insert(UnlitApplied);
    }
}
```

**Step 2: Build**

Run: `cargo build -p scuffed-map-renderer`
Expected: builds successfully

**Step 3: Commit**

```bash
git add crates/map-renderer/src/plugin.rs
git commit -m "feat: force unlit materials for vertex color rendering"
```

---

### Task 6: Wire up CLI with config parsing and multi-floor rendering

Connect the CLI to parse the TOML config, compute render dimensions from `pixels_per_meter` and scene bounds, and render each floor sequentially (one Bevy app run per floor, since we need to restart with fresh visibility state).

**Files:**
- Modify: `crates/map-renderer/src/main.rs`
- Modify: `crates/map-renderer/src/plugin.rs`

**Step 1: Update main.rs with full config-driven workflow**

The approach: since Bevy apps are single-run (no easy way to reset scene visibility mid-run), we'll compute scene bounds from the GLB using the `gltf` crate directly (like `map-pipeline` does), then launch one Bevy app per floor.

Add `gltf` and `glam` dependencies to `Cargo.toml`:

```toml
# Scene bounds computation (without Bevy, for pre-pass)
gltf = { version = "1", features = ["utils", "names"] }
glam = "0.30"
```

Add a `bounds.rs` module:

```rust
//! Pre-compute scene XZ bounds from GLB without Bevy.
//! Uses the same gltf crate as map-pipeline.

use glam::{Mat4, Vec3};
use std::path::Path;

/// Axis-aligned bounding box in XZ plane (Bevy coordinates: Y is up).
/// glTF uses Y-up, same as Bevy, so no axis swap needed.
pub struct SceneBounds {
    pub x_min: f32,
    pub x_max: f32,
    pub z_min: f32,
    pub z_max: f32,
}

impl SceneBounds {
    pub fn width(&self) -> f32 { self.x_max - self.x_min }
    pub fn height(&self) -> f32 { self.z_max - self.z_min }
    pub fn center_x(&self) -> f32 { (self.x_min + self.x_max) / 2.0 }
    pub fn center_z(&self) -> f32 { (self.z_min + self.z_max) / 2.0 }
}

pub fn compute_bounds_from_glb(path: &Path) -> anyhow::Result<SceneBounds> {
    let (doc, buffers, _) = gltf::import(path)?;

    let mut x_min = f32::INFINITY;
    let mut x_max = f32::NEG_INFINITY;
    let mut z_min = f32::INFINITY;
    let mut z_max = f32::NEG_INFINITY;
    let mut found_any = false;

    for scene in doc.scenes() {
        for node in scene.nodes() {
            visit_node(&node, Mat4::IDENTITY, &buffers, &mut x_min, &mut x_max, &mut z_min, &mut z_max, &mut found_any);
        }
    }

    if !found_any {
        anyhow::bail!("No mesh geometry found in GLB");
    }

    Ok(SceneBounds { x_min, x_max, z_min, z_max })
}

fn visit_node(
    node: &gltf::Node,
    parent_transform: Mat4,
    buffers: &[gltf::buffer::Data],
    x_min: &mut f32, x_max: &mut f32,
    z_min: &mut f32, z_max: &mut f32,
    found_any: &mut bool,
) {
    let local = Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = parent_transform * local;

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            if let Some(positions) = reader.read_positions() {
                for pos in positions {
                    let world_pos = world.transform_point3(Vec3::from(pos));
                    *x_min = x_min.min(world_pos.x);
                    *x_max = x_max.max(world_pos.x);
                    *z_min = z_min.min(world_pos.z);
                    *z_max = z_max.max(world_pos.z);
                    *found_any = true;
                }
            }
        }
    }

    for child in node.children() {
        visit_node(&child, world, buffers, x_min, x_max, z_min, z_max, found_any);
    }
}
```

Update `lib.rs`:

```rust
pub mod bounds;
pub mod plugin;
```

Rewrite `main.rs`:

```rust
use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::camera::ScalingMode;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::render::renderer::RenderDevice;

use scuffed_map_pipeline::config::MapConfig;
use scuffed_map_renderer::bounds::compute_bounds_from_glb;
use scuffed_map_renderer::plugin::{
    build_headless_app, create_render_target, apply_floor_filter,
    make_materials_unlit, FloorFilter, RenderJob,
};

#[derive(Parser)]
#[command(name = "scuffed-map-renderer")]
#[command(about = "Render orthographic floor views from GLB models using Bevy")]
struct Cli {
    /// Path to the .glb file
    #[arg(long)]
    glb: PathBuf,

    /// Path to map TOML config with floor definitions
    #[arg(long)]
    config: PathBuf,

    /// Output directory for floor PNGs
    #[arg(long)]
    output: PathBuf,

    /// Render only this floor ID (renders all if omitted)
    #[arg(long)]
    floor: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Parse config
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config: {:?}", cli.config))?;
    let config = MapConfig::from_toml(&config_str)?;

    if config.floors.is_empty() {
        anyhow::bail!("No floors defined in config. Run detect-floors first.");
    }

    // Filter floors
    let floors: Vec<_> = if let Some(ref floor_id) = cli.floor {
        config.floors.iter()
            .filter(|f| f.id == *floor_id)
            .collect()
    } else {
        config.floors.iter().collect()
    };

    if floors.is_empty() {
        anyhow::bail!("Floor '{}' not found in config", cli.floor.as_deref().unwrap_or("?"));
    }

    std::fs::create_dir_all(&cli.output)?;

    // Pre-compute scene bounds from GLB (without Bevy, fast)
    eprintln!("Computing scene bounds from {:?}...", cli.glb);
    let bounds = compute_bounds_from_glb(&cli.glb)?;
    let padding = config.render.camera_padding as f32;
    let ppm = config.render.pixels_per_meter as f32;

    let world_width = bounds.width() + 2.0 * padding;
    let world_height = bounds.height() + 2.0 * padding;
    let px_width = (world_width * ppm) as u32;
    let px_height = (world_height * ppm) as u32;

    eprintln!(
        "Scene: {:.1}x{:.1}m, render: {}x{}px ({} ppm)",
        world_width, world_height, px_width, px_height, ppm
    );

    // Render each floor
    for floor in &floors {
        eprintln!("Rendering floor '{}' [{:.1}, {:.1}]...", floor.id, floor.y_min, floor.y_max);

        let output_path = cli.output.join(format!("{}.png", floor.id));
        let glb_path = cli.glb.clone();
        let center_x = bounds.center_x();
        let center_z = bounds.center_z();
        let y_min = floor.y_min as f32;
        let y_max = floor.y_max as f32;

        let mut app = build_headless_app();
        app.insert_resource(RenderJob {
            warmup_frames: 60, // GLB needs many frames to fully load
            output_path,
            width: px_width,
            height: px_height,
            glb_path: glb_path.clone(),
            camera_scale_x: world_width,
            camera_scale_y: world_height,
            camera_center: Vec3::new(center_x, 0.0, center_z),
        });
        app.insert_resource(FloorFilter { y_min, y_max });
        app.add_systems(Startup, setup_scene);
        app.add_systems(Update, (make_materials_unlit, apply_floor_filter));
        app.run();

        eprintln!("  -> {:?}", cli.output.join(format!("{}.png", floor.id)));
    }

    eprintln!("Done! Rendered {} floor(s).", floors.len());

    Ok(())
}

fn setup_scene(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    job: Res<RenderJob>,
    asset_server: Res<AssetServer>,
) {
    let render_target = create_render_target(
        &mut commands,
        &mut images,
        &render_device,
        job.width,
        job.height,
    );

    // Load GLB
    commands.spawn(SceneRoot(
        asset_server.load(
            GltfAssetLabel::Scene(0).from_asset(job.glb_path.to_string_lossy().to_string()),
        ),
    ));

    // Ambient light
    commands.insert_resource(AmbientLight {
        color: Color::WHITE,
        brightness: 1000.0,
    });

    // Orthographic top-down camera
    commands.spawn((
        Camera3d::default(),
        Projection::from(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: job.camera_scale_x,
                height: job.camera_scale_y,
            },
            ..OrthographicProjection::default_3d()
        }),
        bevy::camera::RenderTarget::Image(render_target.into()),
        Tonemapping::None,
        Transform::from_xyz(
            job.camera_center.x,
            100.0,
            job.camera_center.z,
        ).looking_at(
            Vec3::new(job.camera_center.x, 0.0, job.camera_center.z),
            Vec3::Z,
        ),
    ));
}
```

**Step 2: Build**

Run: `cargo build -p scuffed-map-renderer`
Expected: builds successfully

**Step 3: Commit**

```bash
git add crates/map-renderer/
git commit -m "feat: full CLI with config parsing, bounds computation, and multi-floor rendering"
```

---

### Task 7: Integration test with real GLB

Test the renderer end-to-end with the King's Row GLB file.

**Files:** No new files — this is a manual test.

**Step 1: Run the renderer on King's Row**

Run:
```bash
cargo run -p scuffed-map-renderer -- \
    --glb out/kings_row/kings_row.glb \
    --config maps/kings_row.toml \
    --output out/kings_row/ \
    --floor ground
```

Expected: creates `out/kings_row/ground.png`, a top-down orthographic view of the ground floor with vertex colors on transparent background.

**Step 2: Check the output**

Open the PNG and verify:
- Image dimensions match expected (world_width * ppm) x (world_height * ppm)
- Geometry is visible (not all transparent)
- Vertex colors are present (not all white or all black)
- Only ground-floor geometry is visible (upper floors hidden)

**Step 3: If output looks good, render all floors**

Run:
```bash
cargo run -p scuffed-map-renderer -- \
    --glb out/kings_row/kings_row.glb \
    --config maps/kings_row.toml \
    --output out/kings_row/
```

Expected: creates `ground.png`, `high_ground.png`, and `rooftops.png`.

**Step 4: Run the tile pipeline on the rendered images**

Run:
```bash
cargo run -p scuffed-map-pipeline -- generate-tiles \
    --config maps/kings_row.toml \
    --images out/kings_row/ \
    --output out/kings_row/tiles/
```

Expected: generates tile pyramid and metadata.json.

**Step 5: Commit if everything works**

```bash
git commit --allow-empty -m "test: verified map-renderer with King's Row GLB"
```

---

### Task 8: Troubleshooting and adjustments

This task handles likely issues discovered during Task 7.

**Common issues and fixes:**

1. **Bevy can't find the GLB file**: Bevy's asset server looks for files relative to the `assets/` directory by default. Fix: set `AssetPlugin { file_path: ".".into(), .. }` in the plugin configuration to use the current directory instead.

   In `build_headless_app()`, add:
   ```rust
   .set(AssetPlugin {
       file_path: ".".into(),
       ..default()
   })
   ```

2. **Scene doesn't load in time**: If the image is all transparent, increase `warmup_frames`. For a 92MB GLB, 60 frames may not be enough. Try 120 or add a system that checks if the scene is loaded before capturing.

3. **Vertex colors not showing**: If meshes appear all white, check that `make_materials_unlit` is running. Add debug logging: `info!("Made {} materials unlit", count);`

4. **Camera orientation wrong**: If the view is rotated, adjust the `looking_at` up vector. Try `Vec3::NEG_Z` instead of `Vec3::Z`.

5. **Y axis mapping confusion**: glTF/Bevy use Y-up. The config `y_min`/`y_max` are in glTF convention (Y is height). The floor filter uses `GlobalTransform.translation().y` which is Bevy Y. These should match since Bevy loads glTF without axis conversion.

**Step 1: Fix issues found during Task 7**

Apply needed fixes based on actual test results.

**Step 2: Re-test after fixes**

Run the renderer again and verify output.

**Step 3: Commit fixes**

```bash
git add crates/map-renderer/
git commit -m "fix: address issues from integration testing"
```

---

### Task 9: Smart scene-loaded detection

Replace the fixed `warmup_frames` countdown with a system that detects when the GLB scene has actually loaded by checking if meshes exist.

**Files:**
- Modify: `crates/map-renderer/src/plugin.rs`

**Step 1: Replace warmup_frames with scene-loaded detection**

Change `RenderJob` to track loading state:

```rust
#[derive(Debug, Default, PartialEq, Eq)]
pub enum LoadState {
    #[default]
    WaitingForMeshes,
    /// Scene loaded, rendering N stabilization frames.
    Stabilizing(u32),
    /// Ready to capture.
    ReadyToCapture,
    /// Captured and saved.
    Done,
}
```

Add a system that transitions from `WaitingForMeshes` to `Stabilizing` once meshes appear:

```rust
pub fn check_scene_loaded(
    query: Query<&Mesh3d>,
    mut job: ResMut<RenderJob>,
) {
    if job.load_state == LoadState::WaitingForMeshes {
        let mesh_count = query.iter().count();
        if mesh_count > 0 {
            info!("Scene loaded: {} mesh entities. Stabilizing...", mesh_count);
            job.load_state = LoadState::Stabilizing(10); // 10 frames to stabilize
        }
    }
}
```

Update `capture_and_save` to use `LoadState` instead of `warmup_frames`.

**Step 2: Build and test**

Run: `cargo build -p scuffed-map-renderer`
Run: test with King's Row GLB again

**Step 3: Commit**

```bash
git add crates/map-renderer/src/plugin.rs
git commit -m "feat: detect scene load completion instead of fixed warmup"
```

---

### Task 10: Clippy + final cleanup

**Step 1: Run clippy**

Run: `cargo clippy -p scuffed-map-renderer -- -D warnings`

Fix any warnings.

**Step 2: Run all map-pipeline tests to ensure no regressions**

Run: `cargo test -p scuffed-map-pipeline`
Expected: all tests pass

**Step 3: Build release**

Run: `cargo build -p scuffed-map-renderer --release`

**Step 4: Commit**

```bash
git add crates/map-renderer/
git commit -m "chore: clippy fixes and cleanup for map-renderer"
```
