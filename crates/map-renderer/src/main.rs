use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

use bevy::camera::ScalingMode;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;

use scuffed_map_pipeline::config::MapConfig;
use scuffed_map_renderer::bounds::analyze_glb;
use scuffed_map_renderer::plugin::{
    apply_floor_filter, build_headless_app, create_render_target, setup_materials,
    spawn_gltf_meshes, FloorFilter, LoadState, MeshPlacements, PendingGltf, RenderJob,
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
    // Parse config
    let cli = Cli::parse();
    let config_str = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("Failed to read config: {:?}", cli.config))?;
    let config = MapConfig::from_toml(&config_str)?;

    if config.floors.is_empty() {
        anyhow::bail!("No floors defined in config. Run detect-floors first.");
    }

    // Filter floors
    let floors: Vec<_> = if let Some(ref floor_id) = cli.floor {
        config.floors.iter().filter(|f| f.id == *floor_id).collect()
    } else {
        config.floors.iter().collect()
    };

    if floors.is_empty() {
        anyhow::bail!(
            "Floor '{}' not found in config",
            cli.floor.as_deref().unwrap_or("?")
        );
    }

    std::fs::create_dir_all(&cli.output)?;

    // Resolve GLB to absolute path for Bevy's asset server
    let glb_abs = std::fs::canonicalize(&cli.glb)
        .with_context(|| format!("GLB not found: {:?}", cli.glb))?;
    let asset_dir = glb_abs
        .parent()
        .expect("GLB has no parent dir")
        .to_string_lossy()
        .to_string();
    let glb_filename = glb_abs
        .file_name()
        .expect("GLB has no filename")
        .to_string_lossy()
        .to_string();

    // Pre-compute scene bounds + mesh instances from GLB (without Bevy, fast)
    eprintln!("Analyzing GLB {:?}...", cli.glb);
    let max_distance = config.cleanup.max_distance_from_center as f32;
    let scene_info = analyze_glb(&glb_abs, max_distance)?;
    let bounds = &scene_info.bounds;
    let padding = config.render.camera_padding as f32;
    let ppm = config.render.pixels_per_meter as f32;

    let world_width = bounds.width() + 2.0 * padding;
    let world_height = bounds.height() + 2.0 * padding;
    let mut px_width = (world_width * ppm) as u32;
    let mut px_height = (world_height * ppm) as u32;

    // Clamp to reasonable render size (GPU limit is 16384, but 8192 is more practical)
    const MAX_TEXTURE_DIM: u32 = 8192;
    if px_width > MAX_TEXTURE_DIM || px_height > MAX_TEXTURE_DIM {
        let scale = MAX_TEXTURE_DIM as f32 / px_width.max(px_height) as f32;
        px_width = (px_width as f32 * scale) as u32;
        px_height = (px_height as f32 * scale) as u32;
        eprintln!(
            "Clamped to GPU limit: {}x{}px (effective {:.1} ppm)",
            px_width,
            px_height,
            px_width as f32 / world_width
        );
    }

    eprintln!(
        "Scene: {:.1}x{:.1}m, render: {}x{}px ({} ppm)",
        world_width, world_height, px_width, px_height, ppm
    );

    // Render each floor (one Bevy app per floor — fresh visibility state)
    for floor in &floors {
        eprintln!(
            "Rendering floor '{}' [{:.1}, {:.1}]...",
            floor.id, floor.y_min, floor.y_max
        );

        let output_path = cli.output.join(format!("{}.png", floor.id));
        let center_x = bounds.center_x();
        let center_z = bounds.center_z();
        let y_min = floor.y_min as f32;
        let y_max = floor.y_max as f32;

        let mut app = build_headless_app(asset_dir.clone());
        app.insert_resource(RenderJob {
            output_path,
            width: px_width,
            height: px_height,
            glb_path: PathBuf::from(&glb_filename),
            camera_scale_x: world_width,
            camera_scale_y: world_height,
            camera_center: Vec3::new(center_x, 0.0, center_z),
            load_state: LoadState::default(),
        });
        app.insert_resource(FloorFilter { y_min, y_max });
        app.insert_resource(MeshPlacements(scene_info.mesh_instances.clone()));
        app.add_systems(Startup, setup_scene);
        app.add_systems(Update, (spawn_gltf_meshes, setup_materials, apply_floor_filter));
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

    // Load GLB as Gltf asset (mesh/material handles only — no scene spawner)
    let gltf_handle = asset_server.load::<bevy::gltf::Gltf>(
        job.glb_path.to_string_lossy().to_string(),
    );
    commands.insert_resource(PendingGltf(gltf_handle));

    // Directional light from above at slight angle to reveal geometry
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            -std::f32::consts::FRAC_PI_3, // 60° from horizontal = 30° off vertical
            0.3,                           // slight yaw offset
            0.0,
        )),
    ));

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
        Transform::from_xyz(job.camera_center.x, 100.0, job.camera_center.z).looking_at(
            Vec3::new(job.camera_center.x, 0.0, job.camera_center.z),
            Vec3::NEG_Z,
        ),
    ));
}
