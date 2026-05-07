//! Bevy headless rendering plugin.
//!
//! Adapted from Bevy's `headless_renderer.rs` example (v0.18.1).
//! Sets up offscreen render-to-texture and GPU->CPU readback via crossbeam channel.

use crate::bounds::MeshInstance;
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    gltf::{Gltf, GltfMesh},
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
};
use crossbeam_channel::{Receiver, Sender};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

// ── Load state machine ─────────────────────────────────────────────

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

// ── Resources ──────────────────────────────────────────────────────

/// Tracks the render job state.
#[derive(Resource)]
pub struct RenderJob {
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
    /// Scene loading state machine.
    pub load_state: LoadState,
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
    let mut render_target_image =
        Image::new_target_texture(width, height, TextureFormat::bevy_default(), None);
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

    // Image copier (GPU -> CPU buffer)
    commands.spawn(ImageCopier::new(
        render_target_handle.clone(),
        Extent3d {
            width,
            height,
            ..default()
        },
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
    pub fn new(src_image: Handle<Image>, size: Extent3d, render_device: &RenderDevice) -> Self {
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
    commands.insert_resource(ImageCopiers(image_copiers.iter().cloned().collect()));
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
                (src_image.size.width as usize / block_dimensions.0 as usize) * block_size as usize,
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
// Uses LoadState to detect scene readiness, then saves the rendered image and exits.

pub struct CaptureFramePlugin;

impl Plugin for CaptureFramePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, (check_scene_loaded, capture_and_save).chain());
    }
}

/// Transition from WaitingForMeshes to Stabilizing once meshes appear.
fn check_scene_loaded(query: Query<&Mesh3d>, mut job: ResMut<RenderJob>) {
    if job.load_state == LoadState::WaitingForMeshes {
        let mesh_count = query.iter().count();
        if mesh_count > 0 {
            info!("Scene loaded: {} mesh entities. Stabilizing...", mesh_count);
            job.load_state = LoadState::Stabilizing(60);
        }
    }
}

fn capture_and_save(
    images_to_save: Query<&ImageToSave>,
    receiver: Res<MainWorldReceiver>,
    mut images: ResMut<Assets<Image>>,
    mut job: ResMut<RenderJob>,
    mut app_exit_writer: MessageWriter<AppExit>,
) {
    match &job.load_state {
        LoadState::WaitingForMeshes => {
            // Drain any early frames
            while receiver.try_recv().is_ok() {}
        }
        LoadState::Stabilizing(remaining) => {
            // Drain frames while stabilizing
            while receiver.try_recv().is_ok() {}
            let remaining = *remaining;
            if remaining <= 1 {
                job.load_state = LoadState::ReadyToCapture;
            } else {
                job.load_state = LoadState::Stabilizing(remaining - 1);
            }
        }
        LoadState::ReadyToCapture => {
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

                let row_bytes = img_bytes.width() as usize
                    * img_bytes.texture_descriptor.format.pixel_size().unwrap();
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
                img.save(&job.output_path).expect("Failed to save PNG");
                info!("Saved render to {:?}", job.output_path);
            }

            job.load_state = LoadState::Done;
            app_exit_writer.write(AppExit::Success);
        }
        LoadState::Done => {}
    }
}

// ── Floor visibility filtering ─────────────────────────────────────

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

// ── Unlit materials ────────────────────────────────────────────────

/// Marker to prevent re-processing materials.
#[derive(Component)]
pub struct UnlitApplied;

/// Force all materials to unlit so textures render as flat colors (no PBR shading).
pub fn setup_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    query: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Without<UnlitApplied>>,
) {
    for (entity, material_handle) in query.iter() {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            material.unlit = true;
        }
        commands.entity(entity).insert(UnlitApplied);
    }
}

// ── Manual GLB mesh spawning ───────────────────────────────────────
// Bypasses Bevy's scene spawner (which panics on unregistered types)
// by loading the Gltf asset for mesh/material handles and using
// pre-computed world transforms from the gltf crate.

/// Handle to the loading Gltf asset.
#[derive(Resource)]
pub struct PendingGltf(pub Handle<Gltf>);

/// Pre-computed mesh placements from gltf crate analysis.
#[derive(Resource)]
pub struct MeshPlacements(pub Vec<MeshInstance>);

/// Spawns mesh entities once the Gltf asset is fully loaded.
#[allow(clippy::too_many_arguments)]
pub fn spawn_gltf_meshes(
    mut commands: Commands,
    pending: Option<Res<PendingGltf>>,
    gltf_assets: Res<Assets<Gltf>>,
    gltf_mesh_assets: Res<Assets<GltfMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<AssetServer>,
    placements: Res<MeshPlacements>,
    mut spawned: Local<bool>,
) {
    if *spawned {
        return;
    }
    let Some(pending) = pending else { return };

    if !asset_server.is_loaded_with_dependencies(&pending.0) {
        return;
    }

    let gltf = gltf_assets.get(&pending.0).unwrap();

    // Default material: white + unlit so vertex colors (COLOR_0) show through as-is
    let default_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        ..default()
    });

    let mut mesh_count = 0u32;
    for instance in placements.0.iter() {
        let Some(gltf_mesh_handle) = gltf.meshes.get(instance.mesh_index) else {
            continue;
        };
        let Some(gltf_mesh) = gltf_mesh_assets.get(gltf_mesh_handle) else {
            continue;
        };

        let transform = Transform::from_matrix(instance.world_transform);

        for primitive in &gltf_mesh.primitives {
            let material = primitive
                .material
                .clone()
                .unwrap_or_else(|| default_material.clone());
            commands.spawn((
                Mesh3d(primitive.mesh.clone()),
                MeshMaterial3d(material),
                transform,
            ));
            mesh_count += 1;
        }
    }

    info!(
        "Spawned {} mesh primitives from {} instances",
        mesh_count,
        placements.0.len()
    );
    *spawned = true;
}

// ── App builder ────────────────────────────────────────────────────

/// Build a headless Bevy app configured for offscreen rendering.
/// `asset_dir` is the base directory for Bevy's asset server (absolute path).
pub fn build_headless_app(asset_dir: String) -> App {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgba(0.0, 0.0, 0.0, 0.0)))
        .add_plugins(
            DefaultPlugins
                .set(ImagePlugin::default())
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: asset_dir,
                    ..default()
                }),
        )
        .add_plugins(ImageCopyPlugin)
        .add_plugins(CaptureFramePlugin)
        .add_plugins(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f64(1.0 / 60.0),
        ));
    app
}
