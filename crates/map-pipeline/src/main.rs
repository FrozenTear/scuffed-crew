use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
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

fn cmd_detect_floors(glb: &Path, output: &Path, name: &str, id: &str) -> Result<()> {
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

fn cmd_generate_tiles(config_path: &Path, images_dir: &Path, output_dir: &Path) -> Result<()> {
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
    glb: &Path,
    images_dir: &Path,
    output_dir: &Path,
    config_path: Option<&Path>,
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
        std::fs::create_dir_all(output_dir)?;
        cmd_detect_floors(glb, config_path, name, id)?;
        println!("\nFloor detection complete. Review the config at {:?} then re-run to generate tiles.", config_path);
        return Ok(());
    }

    // Step 2: Generate tiles
    cmd_generate_tiles(config_path, images_dir, output_dir)?;

    Ok(())
}
