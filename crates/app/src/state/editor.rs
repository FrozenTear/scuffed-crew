//! Editor state management for the strategy editor.
//!
//! These are plain Rust structs — no Dioxus signals here. The component layer
//! wraps them in `Signal<T>` as needed.

use scuffed_types::strategy::{
    Color, ElementType, GameMode, HeroId, HeroRole, HeroSelection, MapMetadata, PlaybackState,
    Position, Strategy, StrategyElement, TeamFormat, TeamSlot, TimelinePhase, Tool, Visibility,
};
use uuid::Uuid;

// =============================================================================
// Canvas State
// =============================================================================

/// Canvas/viewport state — zoom, pan, floor, map display settings.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasState {
    /// Canvas zoom level (1.0 = 100%).
    pub zoom: f64,
    /// Canvas pan offset in screen pixels.
    pub pan_offset: Position,
    /// Currently selected floor ID (for multi-floor maps).
    pub selected_floor: Option<String>,
    /// Whether to show health pack overlay.
    pub show_health_packs: bool,
    /// Loaded map metadata (from metadata.json).
    pub map_metadata: Option<MapMetadata>,
    /// Current map ID.
    pub current_map: Option<String>,
    /// Selected sub-map ID (for Control maps with multiple arenas).
    pub selected_sub_map: Option<String>,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_offset: Position::new(0.0, 0.0),
            selected_floor: None,
            show_health_packs: true,
            map_metadata: None,
            current_map: None,
            selected_sub_map: None,
        }
    }
}

// =============================================================================
// Drawing State
// =============================================================================

/// Drawing tool state — active tool, drawing in progress, colors.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingState {
    /// Active drawing tool.
    pub active_tool: Tool,
    /// Whether a drawing operation is in progress.
    pub is_drawing: bool,
    /// Temporary points accumulated during a drawing stroke.
    pub drawing_points: Vec<Position>,
    /// Current drawing color.
    pub draw_color: Color,
    /// Area fill opacity (0.0 - 1.0).
    pub fill_opacity: f32,
    /// Currently selected hero (for player markers).
    pub selected_hero: Option<String>,
}

impl Default for DrawingState {
    fn default() -> Self {
        Self {
            active_tool: Tool::Select,
            is_drawing: false,
            drawing_points: Vec::new(),
            draw_color: Color::BLUE_TEAM,
            fill_opacity: 0.3,
            selected_hero: None,
        }
    }
}

// =============================================================================
// Strategy State
// =============================================================================

/// Core strategy data — the document being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyState {
    /// Strategy ID (None for new unsaved strategies).
    pub strategy_id: Option<String>,
    /// Owner user ID (None = new strategy, current user owns it).
    pub owner_id: Option<String>,
    /// Strategy name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// All strategy elements (markers, routes, areas, etc.).
    pub elements: Vec<StrategyElement>,
    /// Timeline phases.
    pub phases: Vec<TimelinePhase>,
    /// Currently selected phase for viewing/editing.
    pub selected_phase: Option<Uuid>,
    /// Currently selected element (for properties panel).
    pub selected_element: Option<Uuid>,
    /// Team format (5v5 or 6v6).
    pub team_format: TeamFormat,
    /// Team composition — hero assignments per slot.
    pub team_composition: Vec<HeroSelection>,
    /// Timeline playback state.
    pub playback_state: PlaybackState,
    /// Whether there are unsaved changes.
    pub has_unsaved_changes: bool,
    /// Strategy visibility.
    pub visibility: Visibility,
}

impl Default for StrategyState {
    fn default() -> Self {
        Self {
            strategy_id: None,
            owner_id: None,
            name: "Untitled Strategy".to_string(),
            description: None,
            elements: Vec::new(),
            phases: vec![TimelinePhase::default()],
            selected_phase: None,
            selected_element: None,
            team_format: TeamFormat::FiveVFive,
            team_composition: Vec::new(),
            playback_state: PlaybackState::Stopped,
            has_unsaved_changes: false,
            visibility: Visibility::Private,
        }
    }
}

impl StrategyState {
    // =========================================================================
    // Element operations
    // =========================================================================

    /// Add an element and mark state as unsaved.
    pub fn add_element(&mut self, element: StrategyElement) {
        self.elements.push(element);
        self.has_unsaved_changes = true;
    }

    /// Update an element by ID, replacing it in place. Marks state as unsaved.
    pub fn update_element(&mut self, id: Uuid, updated: StrategyElement) {
        if let Some(existing) = self.elements.iter_mut().find(|e| e.id == id) {
            *existing = updated;
            self.has_unsaved_changes = true;
        }
    }

    /// Remove an element by ID. Returns the removed element and its former
    /// index (useful for undo), or `None` if not found.
    pub fn remove_element(&mut self, id: Uuid) -> Option<(usize, StrategyElement)> {
        let idx = self.elements.iter().position(|e| e.id == id)?;
        let element = self.elements.remove(idx);

        // Clear selection if the removed element was selected.
        if self.selected_element == Some(id) {
            self.selected_element = None;
        }

        self.has_unsaved_changes = true;
        Some((idx, element))
    }

    /// Return elements visible in the currently selected phase.
    ///
    /// An element is visible if it has no phase assignment (global) or if its
    /// `phase_id` matches the selected phase.
    pub fn visible_elements(&self) -> Vec<&StrategyElement> {
        self.elements
            .iter()
            .filter(|e| e.phase_id.is_none() || e.phase_id == self.selected_phase)
            .collect()
    }

    /// Find the element nearest to `pos` within `tolerance` pixels.
    /// Returns the element's ID, or `None` if nothing is close enough.
    /// Searches in reverse draw order so top-most elements are found first.
    pub fn select_at(&self, pos: Position, tolerance: f64) -> Option<Uuid> {
        self.elements
            .iter()
            .rev()
            .find(|e| is_position_near_element(pos, e, tolerance))
            .map(|e| e.id)
    }

    // =========================================================================
    // Phase operations
    // =========================================================================

    /// Add a new phase with the given name. Returns the new phase's ID.
    pub fn add_phase(&mut self, name: String) -> Uuid {
        let order = self.phases.len() as u32;
        let phase = TimelinePhase::new(name, order);
        let id = phase.id;
        self.phases.push(phase);
        self.has_unsaved_changes = true;
        id
    }

    /// Remove a phase by ID. Returns the removed phase and its former index
    /// (useful for undo), or `None` if not found. Reorders remaining phases.
    pub fn remove_phase(&mut self, id: Uuid) -> Option<(usize, TimelinePhase)> {
        let idx = self.phases.iter().position(|p| p.id == id)?;
        let phase = self.phases.remove(idx);

        // Reorder remaining phases.
        for (i, p) in self.phases.iter_mut().enumerate() {
            p.order = i as u32;
        }

        // Clear selection if the removed phase was selected.
        if self.selected_phase == Some(id) {
            self.selected_phase = None;
        }

        self.has_unsaved_changes = true;
        Some((idx, phase))
    }

    /// Index (0-based) of the currently selected phase.
    pub fn current_phase_index(&self) -> Option<usize> {
        let selected = self.selected_phase?;
        self.phases.iter().position(|p| p.id == selected)
    }

    /// Advance to the next phase (for playback / keyboard nav).
    pub fn next_phase(&mut self) {
        if self.phases.is_empty() {
            return;
        }

        let next_idx = match self.current_phase_index() {
            Some(idx) if idx + 1 < self.phases.len() => idx + 1,
            Some(_) => return, // already at last phase
            None => 0,         // nothing selected, go to first
        };

        if let Some(phase) = self.phases.get(next_idx) {
            self.selected_phase = Some(phase.id);
        }
    }

    /// Go back to the previous phase.
    pub fn prev_phase(&mut self) {
        if self.phases.is_empty() {
            return;
        }

        let prev_idx = match self.current_phase_index() {
            Some(idx) if idx > 0 => idx - 1,
            Some(_) => return,              // already at first phase
            None => self.phases.len() - 1,  // nothing selected, go to last
        };

        if let Some(phase) = self.phases.get(prev_idx) {
            self.selected_phase = Some(phase.id);
        }
    }

    // =========================================================================
    // Team composition
    // =========================================================================

    /// Assign a hero to a specific team slot.
    pub fn assign_hero_to_slot(&mut self, slot: TeamSlot, hero_id: HeroId) {
        // Remove any existing assignment for this slot.
        self.team_composition.retain(|h| h.slot != slot);

        self.team_composition.push(HeroSelection {
            hero_id,
            player_name: None,
            slot,
        });

        self.has_unsaved_changes = true;
    }

    /// Clear a hero from a team slot.
    pub fn clear_slot(&mut self, slot: TeamSlot) {
        self.team_composition.retain(|h| h.slot != slot);
        self.has_unsaved_changes = true;
    }

    /// Find the next available slot for a given role.
    ///
    /// In 5v5: strict role enforcement (1 tank, 2 dps, 2 support).
    /// In 6v6: any slot allowed, but max 2 tanks.
    pub fn next_available_slot(&self, role: HeroRole) -> Option<TeamSlot> {
        let slots = self.team_format.slots();
        let occupied: Vec<TeamSlot> = self.team_composition.iter().map(|h| h.slot).collect();

        match self.team_format {
            TeamFormat::FiveVFive => {
                // Strict role enforcement.
                slots
                    .iter()
                    .find(|s| s.required_role() == role && !occupied.contains(s))
                    .copied()
            }
            TeamFormat::SixVSix => {
                // Open queue for 6v6 — check tank limit.
                if role == HeroRole::Tank {
                    let tank_count = self
                        .team_composition
                        .iter()
                        .filter(|h| h.slot == TeamSlot::Tank1 || h.slot == TeamSlot::Tank2)
                        .count();
                    if tank_count >= 2 {
                        return None;
                    }
                }
                // Find any empty slot.
                slots.iter().find(|s| !occupied.contains(s)).copied()
            }
        }
    }

    // =========================================================================
    // Serialisation
    // =========================================================================

    /// Serialize the editor state into an API-ready `Strategy` struct.
    pub fn to_strategy(&self) -> Strategy {
        Strategy {
            id: self
                .strategy_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: self.name.clone(),
            description: self.description.clone(),
            map_id: String::new(), // Caller should set from CanvasState.current_map
            sub_map_id: None,      // Caller should set from CanvasState.selected_sub_map
            game_mode: GameMode::Escort, // Caller should set from map data
            owner_id: self.owner_id.clone().unwrap_or_default(),
            team_id: None,
            visibility: self.visibility,
            elements: self.elements.clone(),
            phases: self.phases.clone(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            coordinate_version: scuffed_types::strategy::CoordinateVersion::V2,
        }
    }

    /// Load state from an API response.
    pub fn load_strategy(&mut self, strategy: Strategy) {
        self.strategy_id = Some(strategy.id);
        self.owner_id = Some(strategy.owner_id);
        self.name = strategy.name;
        self.description = strategy.description;
        self.visibility = strategy.visibility;
        self.elements = strategy.elements;
        self.phases = strategy.phases;
        self.has_unsaved_changes = false;
    }
}

// =============================================================================
// Geometry helpers
// =============================================================================

/// Check whether `pos` is within `tolerance` pixels of `element`.
///
/// Handles multi-point element types (routes, areas, arrows, drawings) by
/// checking proximity to every segment, not just the base position.
pub fn is_position_near_element(pos: Position, element: &StrategyElement, tolerance: f64) -> bool {
    // Check the base position first.
    if element.position.distance_to(&pos) < tolerance {
        return true;
    }

    match &element.element_type {
        ElementType::Route { points }
        | ElementType::Area { points }
        | ElementType::Drawing { points, .. } => {
            // Check proximity to each vertex.
            for point in points {
                if point.distance_to(&pos) < tolerance {
                    return true;
                }
            }
            // Check proximity to each line segment.
            for window in points.windows(2) {
                if point_to_segment_distance(pos, window[0], window[1]) < tolerance {
                    return true;
                }
            }
            false
        }
        ElementType::Arrow { end } => {
            if end.distance_to(&pos) < tolerance {
                return true;
            }
            point_to_segment_distance(pos, element.position, *end) < tolerance
        }
        _ => false,
    }
}

/// Shortest distance from `point` to the line segment `seg_start`--`seg_end`.
fn point_to_segment_distance(point: Position, seg_start: Position, seg_end: Position) -> f64 {
    let dx = seg_end.x - seg_start.x;
    let dy = seg_end.y - seg_start.y;
    let len_sq = dx * dx + dy * dy;

    if len_sq == 0.0 {
        // Segment is a point.
        return point.distance_to(&seg_start);
    }

    // Project `point` onto the line, clamped to [0, 1].
    let t = ((point.x - seg_start.x) * dx + (point.y - seg_start.y) * dy) / len_sq;
    let t = t.clamp(0.0, 1.0);

    let proj = Position::new(seg_start.x + t * dx, seg_start.y + t * dy);
    point.distance_to(&proj)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_strategy_state_has_one_phase() {
        let state = StrategyState::default();
        assert_eq!(state.phases.len(), 1);
        assert_eq!(state.phases[0].name, "Setup");
    }

    #[test]
    fn add_and_remove_element() {
        let mut state = StrategyState::default();
        let elem = StrategyElement::new(ElementType::PlayerMarker, Position::new(100.0, 200.0));
        let id = elem.id;

        state.add_element(elem);
        assert_eq!(state.elements.len(), 1);
        assert!(state.has_unsaved_changes);

        let removed = state.remove_element(id);
        assert!(removed.is_some());
        let (idx, removed_elem) = removed.unwrap();
        assert_eq!(idx, 0);
        assert_eq!(removed_elem.id, id);
        assert!(state.elements.is_empty());
    }

    #[test]
    fn visible_elements_filters_by_phase() {
        let mut state = StrategyState::default();
        let phase_id = state.phases[0].id;

        // Global element (no phase).
        let global = StrategyElement::new(ElementType::PlayerMarker, Position::new(0.0, 0.0));
        state.add_element(global);

        // Phase-specific element.
        let mut phased =
            StrategyElement::new(ElementType::PlayerMarker, Position::new(10.0, 10.0));
        phased.phase_id = Some(phase_id);
        state.add_element(phased);

        // Other-phase element.
        let mut other =
            StrategyElement::new(ElementType::PlayerMarker, Position::new(20.0, 20.0));
        other.phase_id = Some(Uuid::new_v4());
        state.add_element(other);

        // No phase selected -> only globals.
        state.selected_phase = None;
        assert_eq!(state.visible_elements().len(), 1);

        // Phase selected -> global + matching.
        state.selected_phase = Some(phase_id);
        assert_eq!(state.visible_elements().len(), 2);
    }

    #[test]
    fn phase_navigation() {
        let mut state = StrategyState::default();
        let p1 = state.phases[0].id;
        let p2 = state.add_phase("Attack".to_string());
        let p3 = state.add_phase("Cleanup".to_string());

        // Start with no selection, next goes to first.
        state.selected_phase = None;
        state.next_phase();
        assert_eq!(state.selected_phase, Some(p1));

        state.next_phase();
        assert_eq!(state.selected_phase, Some(p2));

        state.next_phase();
        assert_eq!(state.selected_phase, Some(p3));

        // Already at last — stays.
        state.next_phase();
        assert_eq!(state.selected_phase, Some(p3));

        state.prev_phase();
        assert_eq!(state.selected_phase, Some(p2));

        state.prev_phase();
        assert_eq!(state.selected_phase, Some(p1));

        // Already at first — stays.
        state.prev_phase();
        assert_eq!(state.selected_phase, Some(p1));
    }

    #[test]
    fn select_at_finds_nearest_element() {
        let state = StrategyState {
            elements: vec![
                StrategyElement::new(ElementType::PlayerMarker, Position::new(100.0, 100.0)),
                StrategyElement::new(ElementType::PlayerMarker, Position::new(500.0, 500.0)),
            ],
            ..Default::default()
        };

        let found = state.select_at(Position::new(105.0, 98.0), 30.0);
        assert_eq!(found, Some(state.elements[0].id));

        let found = state.select_at(Position::new(300.0, 300.0), 30.0);
        assert!(found.is_none());
    }

    #[test]
    fn team_composition_assign_and_clear() {
        let mut state = StrategyState::default();

        state.assign_hero_to_slot(TeamSlot::Tank1, "reinhardt".to_string());
        assert_eq!(state.team_composition.len(), 1);
        assert_eq!(state.team_composition[0].hero_id, "reinhardt");

        // Re-assigning the same slot replaces the hero.
        state.assign_hero_to_slot(TeamSlot::Tank1, "orisa".to_string());
        assert_eq!(state.team_composition.len(), 1);
        assert_eq!(state.team_composition[0].hero_id, "orisa");

        state.clear_slot(TeamSlot::Tank1);
        assert!(state.team_composition.is_empty());
    }

    #[test]
    fn next_available_slot_5v5() {
        let mut state = StrategyState::default();
        state.team_format = TeamFormat::FiveVFive;

        // First tank slot should be Tank1.
        assert_eq!(
            state.next_available_slot(HeroRole::Tank),
            Some(TeamSlot::Tank1)
        );

        state.assign_hero_to_slot(TeamSlot::Tank1, "reinhardt".to_string());

        // In 5v5 there is only one tank slot.
        assert_eq!(state.next_available_slot(HeroRole::Tank), None);

        // DPS slots should still be available.
        assert_eq!(
            state.next_available_slot(HeroRole::Damage),
            Some(TeamSlot::Dps1)
        );
    }

    #[test]
    fn point_to_segment_distance_zero_length() {
        let d = point_to_segment_distance(
            Position::new(3.0, 4.0),
            Position::new(0.0, 0.0),
            Position::new(0.0, 0.0),
        );
        assert!((d - 5.0).abs() < 1e-10);
    }
}
