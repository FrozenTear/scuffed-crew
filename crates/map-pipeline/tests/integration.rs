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
        render: RenderConfig {
            pixels_per_meter: 32.0,
            camera_padding: 5.0,
        },
        tiles: TileConfig {
            tile_size: 256,
            max_zoom: 2,
        },
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
