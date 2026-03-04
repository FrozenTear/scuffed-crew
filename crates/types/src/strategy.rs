use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// Basic Geometry
// =============================================================================

/// 2D position on the map canvas
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance_to(&self, other: &Position) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// RGBA color representation
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn to_css(&self) -> String {
        format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, self.a)
    }

    pub fn to_css_alpha(&self, alpha: f32) -> String {
        format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, alpha)
    }

    pub const BLUE_TEAM: Color = Color::rgb(66, 135, 245);
    pub const RED_TEAM: Color = Color::rgb(239, 83, 80);
    pub const TANK: Color = Color::rgb(245, 180, 60);
    pub const DAMAGE: Color = Color::rgb(230, 80, 80);
    pub const SUPPORT: Color = Color::rgb(100, 200, 120);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
}

/// Bounding box
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(&self, pos: &Position) -> bool {
        pos.x >= self.x
            && pos.x <= self.x + self.width
            && pos.y >= self.y
            && pos.y <= self.y + self.height
    }
}

// =============================================================================
// Hero References (minimal types needed by editor)
// =============================================================================

pub type HeroId = String;
pub type AbilityId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeroRole {
    Tank,
    Damage,
    Support,
}

impl HeroRole {
    pub fn color_hex(&self) -> &'static str {
        match self {
            HeroRole::Tank => "#f5b43c",
            HeroRole::Damage => "#e65050",
            HeroRole::Support => "#64c878",
        }
    }
}

impl std::fmt::Display for HeroRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeroRole::Tank => write!(f, "Tank"),
            HeroRole::Damage => write!(f, "Damage"),
            HeroRole::Support => write!(f, "Support"),
        }
    }
}

// =============================================================================
// Team Composition
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeroSelection {
    pub hero_id: HeroId,
    pub player_name: Option<String>,
    pub slot: TeamSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TeamFormat {
    #[default]
    FiveVFive,
    SixVSix,
}

impl TeamFormat {
    pub fn slots(&self) -> &'static [TeamSlot] {
        match self {
            TeamFormat::FiveVFive => &[
                TeamSlot::Tank1,
                TeamSlot::Dps1,
                TeamSlot::Dps2,
                TeamSlot::Support1,
                TeamSlot::Support2,
            ],
            TeamFormat::SixVSix => &[
                TeamSlot::Tank1,
                TeamSlot::Tank2,
                TeamSlot::Dps1,
                TeamSlot::Dps2,
                TeamSlot::Support1,
                TeamSlot::Support2,
            ],
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            TeamFormat::FiveVFive => "5v5",
            TeamFormat::SixVSix => "6v6",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamSlot {
    Tank1,
    Tank2,
    Dps1,
    Dps2,
    Support1,
    Support2,
}

impl TeamSlot {
    pub fn required_role(&self) -> HeroRole {
        match self {
            TeamSlot::Tank1 | TeamSlot::Tank2 => HeroRole::Tank,
            TeamSlot::Dps1 | TeamSlot::Dps2 => HeroRole::Damage,
            TeamSlot::Support1 | TeamSlot::Support2 => HeroRole::Support,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            TeamSlot::Tank1 => "Tank",
            TeamSlot::Tank2 => "Tank 2",
            TeamSlot::Dps1 => "DPS 1",
            TeamSlot::Dps2 => "DPS 2",
            TeamSlot::Support1 => "Support 1",
            TeamSlot::Support2 => "Support 2",
        }
    }
}

// =============================================================================
// Map Types
// =============================================================================

pub type MapId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    Escort,
    Hybrid,
    Control,
    Push,
    Flashpoint,
    Clash,
    PayloadRace,
    Assault,
}

impl std::fmt::Display for GameMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GameMode::Escort => write!(f, "Escort"),
            GameMode::Hybrid => write!(f, "Hybrid"),
            GameMode::Control => write!(f, "Control"),
            GameMode::Push => write!(f, "Push"),
            GameMode::Flashpoint => write!(f, "Flashpoint"),
            GameMode::Clash => write!(f, "Clash"),
            GameMode::PayloadRace => write!(f, "Payload Race"),
            GameMode::Assault => write!(f, "Assault (2CP)"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubMap {
    pub id: String,
    pub name: String,
    pub full_name: String,
    pub thumbnail_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Map {
    pub id: MapId,
    pub name: String,
    pub game_mode: GameMode,
    pub layers: Vec<MapLayer>,
    pub bounds: Bounds,
    pub thumbnail_path: String,
    #[serde(default)]
    pub sub_maps: Vec<SubMap>,
}

impl Map {
    pub fn has_sub_maps(&self) -> bool {
        !self.sub_maps.is_empty()
    }

    pub fn sub_map_by_id(&self, id: &str) -> Option<&SubMap> {
        self.sub_maps.iter().find(|s| s.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapLayer {
    pub id: String,
    pub name: String,
    pub image_path: String,
    pub z_order: i32,
    pub bounds: Bounds,
}

/// World coordinate bounds (in meters)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WorldBounds {
    pub x_min: f64,
    pub x_max: f64,
    pub z_min: f64,
    pub z_max: f64,
}

impl WorldBounds {
    pub fn width(&self) -> f64 {
        self.x_max - self.x_min
    }

    pub fn height(&self) -> f64 {
        self.z_max - self.z_min
    }
}

/// Coordinate transformation between world meters and pixel coordinates
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoordinateTransform {
    pub origin_x: f64,
    pub origin_z: f64,
    pub pixels_per_meter: f64,
    pub z_flip: bool,
}

impl CoordinateTransform {
    pub fn new(origin_x: f64, origin_z: f64, pixels_per_meter: f64) -> Self {
        Self { origin_x, origin_z, pixels_per_meter, z_flip: true }
    }

    pub fn world_to_pixel(&self, world_x: f64, world_z: f64) -> (f64, f64) {
        let pixel_x = (world_x - self.origin_x) * self.pixels_per_meter;
        let pixel_y = if self.z_flip {
            (self.origin_z - world_z) * self.pixels_per_meter
        } else {
            (world_z - self.origin_z) * self.pixels_per_meter
        };
        (pixel_x, pixel_y)
    }

    pub fn pixel_to_world(&self, pixel_x: f64, pixel_y: f64) -> (f64, f64) {
        let world_x = self.origin_x + pixel_x / self.pixels_per_meter;
        let world_z = if self.z_flip {
            self.origin_z - pixel_y / self.pixels_per_meter
        } else {
            self.origin_z + pixel_y / self.pixels_per_meter
        };
        (world_x, world_z)
    }

    pub fn from_bounds(bounds: &WorldBounds, image_width: u32, image_height: u32) -> Self {
        let ppm_x = image_width as f64 / bounds.width();
        let ppm_z = image_height as f64 / bounds.height();
        let pixels_per_meter = ppm_x.min(ppm_z);
        Self {
            origin_x: bounds.x_min,
            origin_z: bounds.z_max,
            pixels_per_meter,
            z_flip: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorLevel {
    pub id: String,
    pub name: String,
    pub y_min: f64,
    pub y_max: f64,
    pub image_path: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TilePyramidInfo {
    pub tile_size: u32,
    pub max_zoom: u32,
    pub full_width: u32,
    pub full_height: u32,
}

impl TilePyramidInfo {
    pub fn tiles_at_zoom(&self, zoom: u32) -> (u32, u32) {
        let scale = 1 << (self.max_zoom - zoom.min(self.max_zoom));
        let width = (self.full_width / scale + self.tile_size - 1) / self.tile_size;
        let height = (self.full_height / scale + self.tile_size - 1) / self.tile_size;
        (width.max(1), height.max(1))
    }

    pub fn tile_path(&self, floor_id: &str, z: u32, x: u32, y: u32) -> String {
        format!("floors/{}/{}/{}/{}.webp", floor_id, z, x, y)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorConnection {
    pub from_floor: i32,
    pub to_floor: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub from_z: f64,
    pub to_z: f64,
    pub connection_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthPack {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub size: HealthPackSize,
}

impl HealthPack {
    pub fn position_2d(&self) -> (f64, f64) {
        (self.x, self.z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthPackSize {
    Small,
    Large,
}

impl HealthPackSize {
    pub fn hp(&self) -> u32 {
        match self {
            HealthPackSize::Small => 75,
            HealthPackSize::Large => 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapMetadata {
    pub transform: CoordinateTransform,
    pub world_bounds: WorldBounds,
    pub composite_width: u32,
    pub composite_height: u32,
    pub floors: Vec<FloorLevel>,
    pub tile_pyramid: Option<TilePyramidInfo>,
    #[serde(default)]
    pub health_packs: Vec<HealthPack>,
    #[serde(default)]
    pub connections: Vec<FloorConnection>,
}

impl MapMetadata {
    pub fn default_floor(&self) -> Option<&FloorLevel> {
        self.floors.iter().find(|f| f.is_default).or(self.floors.first())
    }

    pub fn playable_pixel_bounds(&self) -> (f64, f64, f64, f64) {
        let (min_x, max_y) =
            self.transform.world_to_pixel(self.world_bounds.x_min, self.world_bounds.z_min);
        let (max_x, min_y) =
            self.transform.world_to_pixel(self.world_bounds.x_max, self.world_bounds.z_max);
        (min_x, min_y, max_x - min_x, max_y - min_y)
    }

    pub fn playable_center_pixel(&self) -> (f64, f64) {
        let (min_x, min_y, width, height) = self.playable_pixel_bounds();
        (min_x + width / 2.0, min_y + height / 2.0)
    }

    pub fn floor_by_id(&self, id: &str) -> Option<&FloorLevel> {
        self.floors.iter().find(|f| f.id == id)
    }

    pub fn health_packs_for_floor(&self, floor: &FloorLevel) -> Vec<&HealthPack> {
        self.health_packs.iter().filter(|hp| hp.y >= floor.y_min && hp.y <= floor.y_max).collect()
    }
}

// =============================================================================
// Strategy Core
// =============================================================================

pub type StrategyId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordinateVersion {
    #[default]
    V1,
    V2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    #[default]
    Private,
    Unlisted,
    Public,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Private => write!(f, "Private"),
            Visibility::Unlisted => write!(f, "Unlisted"),
            Visibility::Public => write!(f, "Public"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Strategy {
    pub id: StrategyId,
    pub name: String,
    pub description: Option<String>,
    pub map_id: MapId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_map_id: Option<String>,
    pub game_mode: GameMode,
    pub owner_id: String,
    pub team_id: Option<String>,
    pub visibility: Visibility,
    pub elements: Vec<StrategyElement>,
    pub phases: Vec<TimelinePhase>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub coordinate_version: CoordinateVersion,
}

impl Strategy {
    pub fn new(name: String, map_id: MapId, game_mode: GameMode, owner_id: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            description: None,
            map_id,
            sub_map_id: None,
            game_mode,
            owner_id,
            team_id: None,
            visibility: Visibility::Private,
            elements: Vec::new(),
            phases: vec![TimelinePhase::default()],
            created_at: now,
            updated_at: now,
            coordinate_version: CoordinateVersion::V2,
        }
    }

    pub fn elements_for_phase(
        &self,
        phase_id: Option<Uuid>,
    ) -> impl Iterator<Item = &StrategyElement> {
        self.elements
            .iter()
            .filter(move |e| e.phase_id == phase_id || e.phase_id.is_none())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyElement {
    pub id: Uuid,
    pub element_type: ElementType,
    pub position: Position,
    pub hero_id: Option<HeroId>,
    pub label: Option<String>,
    pub color: Color,
    pub phase_id: Option<Uuid>,
    pub z_index: i32,
}

impl StrategyElement {
    pub fn new(element_type: ElementType, position: Position) -> Self {
        Self {
            id: Uuid::new_v4(),
            element_type,
            position,
            hero_id: None,
            label: None,
            color: Color::BLUE_TEAM,
            phase_id: None,
            z_index: 0,
        }
    }

    pub fn with_hero(mut self, hero_id: HeroId) -> Self {
        self.hero_id = Some(hero_id);
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_phase(mut self, phase_id: Uuid) -> Self {
        self.phase_id = Some(phase_id);
        self
    }

    pub fn with_label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ElementType {
    PlayerMarker,
    Route { points: Vec<Position> },
    Area { points: Vec<Position> },
    Ability { ability_id: AbilityId },
    Text { content: String, font_size: f32 },
    Icon { icon_type: IconType },
    Drawing { points: Vec<Position>, stroke_width: f32 },
    Arrow { end: Position },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconType {
    Skull,
    Warning,
    Star,
    Flag,
    Eye,
    Shield,
    Target,
    Question,
}

impl IconType {
    pub fn emoji(&self) -> &'static str {
        match self {
            IconType::Skull => "\u{1F480}",
            IconType::Warning => "\u{26A0}\u{FE0F}",
            IconType::Star => "\u{2B50}",
            IconType::Flag => "\u{1F6A9}",
            IconType::Eye => "\u{1F441}\u{FE0F}",
            IconType::Shield => "\u{1F6E1}\u{FE0F}",
            IconType::Target => "\u{1F3AF}",
            IconType::Question => "\u{2753}",
        }
    }
}

// =============================================================================
// Strategy Summary (for listings)
// =============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategySummary {
    pub id: StrategyId,
    pub name: String,
    pub map_id: MapId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_map_id: Option<String>,
    pub game_mode: GameMode,
    pub owner_name: String,
    pub visibility: Visibility,
    pub element_count: usize,
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Timeline
// =============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelinePhase {
    pub id: Uuid,
    pub name: String,
    pub order: u32,
    pub timestamp: Option<String>,
    pub description: Option<String>,
}

impl Default for TimelinePhase {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Setup".to_string(),
            order: 0,
            timestamp: Some("0:00".to_string()),
            description: Some("Initial positioning".to_string()),
        }
    }
}

impl TimelinePhase {
    pub fn new(name: String, order: u32) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            order,
            timestamp: None,
            description: None,
        }
    }
}

// =============================================================================
// Editor-specific enums
// =============================================================================

/// Active drawing tool
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Pan,
    PlayerMarker,
    Route,
    Area,
    Arrow,
    Text,
    Eraser,
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tool::Select => write!(f, "Select"),
            Tool::Pan => write!(f, "Pan"),
            Tool::PlayerMarker => write!(f, "Marker"),
            Tool::Route => write!(f, "Route"),
            Tool::Area => write!(f, "Area"),
            Tool::Arrow => write!(f, "Arrow"),
            Tool::Text => write!(f, "Text"),
            Tool::Eraser => write!(f, "Eraser"),
        }
    }
}

/// Playback state for timeline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

// =============================================================================
// WebSocket Messages
// =============================================================================

/// Minimal user info for collaboration display
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollabUserInfo {
    pub id: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    JoinRoom { strategy_id: StrategyId },
    LeaveRoom,
    ElementAdd { element: StrategyElement },
    ElementUpdate { id: Uuid, changes: ElementPatch },
    ElementDelete { id: Uuid },
    PhaseAdd { phase: TimelinePhase },
    PhaseUpdate { id: Uuid, changes: PhasePatch },
    PhaseDelete { id: Uuid },
    CursorMove { position: Position },
    Ping,
}

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    RoomJoined { strategy: Strategy, users: Vec<CollabUserInfo> },
    UserJoined { user: CollabUserInfo },
    UserLeft { user_id: String },
    ElementAdded { by: String, element: StrategyElement },
    ElementUpdated { by: String, id: Uuid, changes: ElementPatch },
    ElementDeleted { by: String, id: Uuid },
    PhaseAdded { by: String, phase: TimelinePhase },
    PhaseUpdated { by: String, id: Uuid, changes: PhasePatch },
    PhaseDeleted { by: String, id: Uuid },
    CursorMoved { user_id: String, position: Position },
    Pong,
    Error { message: String },
}

/// Wrapper for WS requests with optional request tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(flatten)]
    pub message: ClientMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(flatten)]
    pub message: ServerMessage,
}

impl From<ServerMessage> for WsResponse {
    fn from(message: ServerMessage) -> Self {
        Self { request_id: None, message }
    }
}

impl WsResponse {
    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }
}

/// Partial update for element properties (used in WebSocket messages)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ElementPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero_id: Option<Option<HeroId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_id: Option<Option<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i32>,
}

/// Partial update for phase properties
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PhasePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
}

impl StrategyElement {
    pub fn apply_patch(&mut self, patch: &ElementPatch) {
        if let Some(position) = patch.position {
            self.position = position;
        }
        if let Some(ref hero_id) = patch.hero_id {
            self.hero_id = hero_id.clone();
        }
        if let Some(ref label) = patch.label {
            self.label = label.clone();
        }
        if let Some(color) = patch.color {
            self.color = color;
        }
        if let Some(ref phase_id) = patch.phase_id {
            self.phase_id = *phase_id;
        }
        if let Some(z_index) = patch.z_index {
            self.z_index = z_index;
        }
    }
}

impl TimelinePhase {
    pub fn apply_patch(&mut self, patch: &PhasePatch) {
        if let Some(ref name) = patch.name {
            self.name = name.clone();
        }
        if let Some(order) = patch.order {
            self.order = order;
        }
        if let Some(ref timestamp) = patch.timestamp {
            self.timestamp = timestamp.clone();
        }
        if let Some(ref description) = patch.description {
            self.description = description.clone();
        }
    }
}
