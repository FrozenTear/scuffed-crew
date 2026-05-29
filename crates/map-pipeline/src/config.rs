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
fn default_max_distance() -> f64 {
    200.0
}
fn default_min_object_size() -> f64 {
    0.01
}
fn default_true() -> bool {
    true
}
fn default_skybox_threshold() -> f64 {
    500.0
}
fn default_slope() -> f64 {
    50.0
}
fn default_bin_width() -> f64 {
    0.25
}
fn default_sigma() -> f64 {
    0.4
}
fn default_floor_gap() -> f64 {
    2.0
}
fn default_prominence() -> f64 {
    10.0
}
fn default_ppm() -> f64 {
    32.0
}
fn default_padding() -> f64 {
    5.0
}
fn default_tile_size() -> u32 {
    256
}
fn default_max_zoom() -> u32 {
    4
}

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
