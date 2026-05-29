use crate::config::MapConfig;
use scuffed_types::{CoordinateTransform, FloorLevel, MapMetadata, TilePyramidInfo, WorldBounds};
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

    let transform =
        CoordinateTransform::from_bounds(&world_bounds, composite_width, composite_height);

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
        health_packs: Vec::new(),
        connections: Vec::new(),
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

    WorldBounds {
        x_min,
        x_max,
        z_min,
        z_max,
    }
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
            render: RenderConfig {
                pixels_per_meter: 32.0,
                camera_padding: 5.0,
            },
            tiles: TileConfig {
                tile_size: 256,
                max_zoom: 3,
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
        }
    }

    #[test]
    fn build_metadata_basic() {
        let config = test_config();
        let bounds = WorldBounds {
            x_min: -40.0,
            x_max: 40.0,
            z_min: -30.0,
            z_max: 30.0,
        };
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
        let bounds = WorldBounds {
            x_min: -40.0,
            x_max: 40.0,
            z_min: -30.0,
            z_max: 30.0,
        };
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
        let triangles = vec![crate::mesh::Triangle::new(
            Vec3::new(-10.0, 0.0, -5.0),
            Vec3::new(10.0, 0.0, -5.0),
            Vec3::new(0.0, 0.0, 5.0),
        )];
        let bounds = estimate_world_bounds(&triangles);
        assert_eq!(bounds.x_min, -10.0);
        assert_eq!(bounds.x_max, 10.0);
        assert_eq!(bounds.z_min, -5.0);
        assert_eq!(bounds.z_max, 5.0);
    }
}
