//! Strategy editor page — main composition layer for the strategy editor.
//!
//! Composes all editor sub-components (canvas, toolbar, panels, timeline)
//! and manages top-level editor state using Dioxus signals.
//!
//! Layout:
//! ```text
//! EditorPage
//! +-- style { EDITOR_CSS }
//! +-- Left Sidebar (collapsible)
//! |   +-- TeamPanel
//! |   +-- HeroPicker
//! +-- Main Area
//! |   +-- Toolbar (top)
//! |   +-- MapCanvas (center, fills remaining space)
//! |   +-- ElementList (bottom, collapsible)
//! +-- Right Sidebar (collapsible)
//! |   +-- PropertiesPanel
//! |   +-- Timeline
//! +-- Status Bar (bottom)
//!     +-- Map info, Element count, Active tool, Current phase
//! ```
//!
//! Ported from Leptos to Dioxus 0.7.

use dioxus::prelude::*;
use serde::Serialize;
use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use scuffed_api_client::ApiClient;
use scuffed_types::strategy::{
    ElementType, PlaybackState, Position, Strategy, StrategyElement, TimelinePhase, Tool,
    Visibility,
};

use crate::components::strategy::MapCanvas;
use crate::keybindings::{self, EditorAction};
use crate::state::editor::{CanvasState, DrawingState, StrategyState};
use crate::state::undo::{UndoManager, UndoableAction};

// =============================================================================
// API request/response types
// =============================================================================

#[derive(Debug, Serialize)]
struct CreateStrategyRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    map_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub_map_id: Option<String>,
    game_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
    visibility: String,
}

#[derive(Debug, Serialize)]
struct UpdateStrategyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    elements: Option<Vec<StrategyElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    phases: Option<Vec<TimelinePhase>>,
}

// =============================================================================
// CSS
// =============================================================================

const EDITOR_CSS: &str = r#"
    .editor-page {
        display: flex;
        flex-direction: column;
        height: 100vh;
        background: var(--bg-base, #0d0d1a);
        color: var(--text-primary, #e0e0e0);
        overflow: hidden;
    }

    .editor-container {
        display: flex;
        flex: 1;
        overflow: hidden;
    }

    /* Sidebars */
    .editor-sidebar {
        display: flex;
        flex-direction: column;
        width: 260px;
        min-width: 260px;
        background: var(--bg-surface, #1a1a2e);
        border-color: var(--border, #2a2a3e);
        transition: width 0.2s ease, min-width 0.2s ease;
        overflow: hidden;
    }
    .editor-sidebar.collapsed {
        width: 36px;
        min-width: 36px;
    }
    .editor-sidebar-left {
        border-right: 1px solid var(--border, #2a2a3e);
    }
    .editor-sidebar-right {
        border-left: 1px solid var(--border, #2a2a3e);
    }

    .sidebar-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0.5rem;
        border-bottom: 1px solid var(--border, #2a2a3e);
        min-height: 36px;
    }
    .sidebar-title {
        font-size: 0.8rem;
        font-weight: 600;
        color: var(--text-bright, #fff);
        text-transform: uppercase;
        letter-spacing: 0.05em;
        white-space: nowrap;
        overflow: hidden;
    }
    .collapsed .sidebar-title {
        display: none;
    }
    .sidebar-collapse-btn {
        background: none;
        border: none;
        color: var(--text-muted, #666);
        cursor: pointer;
        padding: 0.2rem 0.4rem;
        font-size: 0.7rem;
        border-radius: 3px;
        transition: background 0.15s, color 0.15s;
        flex-shrink: 0;
    }
    .sidebar-collapse-btn:hover {
        background: var(--bg-card, #252540);
        color: var(--text-bright, #fff);
    }
    .sidebar-content {
        flex: 1;
        overflow-y: auto;
        padding: 0.5rem;
    }
    .collapsed .sidebar-content {
        display: none;
    }

    /* Main canvas area */
    .editor-main {
        display: flex;
        flex-direction: column;
        flex: 1;
        overflow: hidden;
        position: relative;
    }

    .canvas-container {
        flex: 1;
        position: relative;
        overflow: hidden;
    }

    /* Bottom panel (element list) */
    .element-list-wrapper {
        border-top: 1px solid var(--border, #2a2a3e);
        background: var(--bg-surface, #1a1a2e);
        max-height: 200px;
        overflow-y: auto;
        transition: max-height 0.2s ease;
    }
    .element-list-wrapper.collapsed {
        max-height: 0;
        overflow: hidden;
        border-top: none;
    }
    .panel-collapse-btn {
        display: block;
        width: 100%;
        padding: 0.15rem;
        text-align: center;
        background: var(--bg-card, #252540);
        border: none;
        color: var(--text-muted, #666);
        cursor: pointer;
        font-size: 0.65rem;
        transition: background 0.15s, color 0.15s;
    }
    .panel-collapse-btn:hover {
        background: var(--border, #2a2a3e);
        color: var(--text-bright, #fff);
    }

    /* Element list items */
    .element-list {
        padding: 0.25rem 0.5rem;
    }
    .element-list-item {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.3rem 0.5rem;
        border-radius: 4px;
        cursor: pointer;
        font-size: 0.8rem;
        color: var(--text-secondary, #999);
        transition: background 0.1s;
    }
    .element-list-item:hover {
        background: var(--bg-card, #252540);
    }
    .element-list-item.selected {
        background: rgba(255, 106, 0, 0.15);
        color: var(--text-bright, #fff);
    }
    .element-list-item .el-number {
        font-size: 0.7rem;
        color: var(--text-muted, #666);
        width: 1.5em;
        text-align: right;
    }
    .element-list-item .el-type {
        font-size: 0.7rem;
        padding: 0.1rem 0.35rem;
        border-radius: 3px;
        background: rgba(255, 255, 255, 0.05);
        color: #ff6a00;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .element-list-item .el-color {
        width: 10px;
        height: 10px;
        border-radius: 50%;
        flex-shrink: 0;
    }

    /* Status bar */
    .editor-statusbar {
        display: flex;
        align-items: center;
        gap: 1rem;
        padding: 0 0.75rem;
        height: 28px;
        background: var(--bg-surface, #1a1a2e);
        border-top: 1px solid var(--border, #2a2a3e);
        font-size: 0.75rem;
        color: var(--text-muted, #666);
    }
    .status-item {
        white-space: nowrap;
    }
    .status-map-link {
        background: none;
        border: none;
        color: var(--text-secondary, #999);
        cursor: pointer;
        font-size: 0.75rem;
        padding: 0;
        transition: color 0.15s;
    }
    .status-map-link:hover {
        color: #ff6a00;
    }
    .status-item.unsaved {
        color: #ff6a00;
        font-weight: 600;
    }
    .status-item.collaborators {
        color: var(--text-secondary, #999);
    }

    /* Map picker modal */
    .map-picker-overlay {
        position: fixed;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        background: rgba(0, 0, 0, 0.7);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 100;
    }
    .map-picker-dialog {
        background: var(--bg-surface, #1e1e30);
        border: 1px solid var(--border, #333);
        border-radius: 12px;
        max-width: 600px;
        width: 90%;
        max-height: 70vh;
        display: flex;
        flex-direction: column;
    }
    .map-picker-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 1rem 1.5rem;
        border-bottom: 1px solid var(--border, #333);
    }
    .map-picker-header h3 {
        margin: 0;
        color: var(--text-bright, #fff);
        font-family: var(--font-display, sans-serif);
    }
    .map-picker-close {
        background: none;
        border: none;
        color: var(--text-muted, #666);
        font-size: 1.2rem;
        cursor: pointer;
        padding: 0.25rem;
        transition: color 0.15s;
    }
    .map-picker-close:hover {
        color: var(--text-bright, #fff);
    }
    .map-picker-content {
        padding: 1rem;
        overflow-y: auto;
        flex: 1;
    }

    /* Placeholder sections for components not yet ported */
    .panel-placeholder {
        padding: 0.75rem;
        text-align: center;
        color: var(--text-muted, #666);
        font-size: 0.8rem;
        font-style: italic;
    }
"#;

// =============================================================================
// Entry points
// =============================================================================

/// Editor page for creating a new strategy.
#[component]
pub fn StrategyEditorNew() -> Element {
    rsx! {
        EditorLayout { initial_strategy: None }
    }
}

/// Editor page for editing an existing strategy (loaded by ID).
#[component]
pub fn StrategyEditor(id: String) -> Element {
    let id_clone = id.clone();
    let strategy_resource = use_resource(move || {
        let id = id_clone.clone();
        async move {
            ApiClient::web()
                .fetch::<Strategy>(&format!("/api/strategy/strategies/{id}"))
                .await
                .ok()
        }
    });

    let data = strategy_resource.read();
    match data.as_ref() {
        None => rsx! {
            div {
                style: "display: flex; align-items: center; justify-content: center; height: 100vh; color: #aaa;",
                "Loading strategy..."
            }
        },
        Some(None) => rsx! {
            div {
                style: "display: flex; align-items: center; justify-content: center; height: 100vh; color: #f88;",
                "Strategy not found or access denied."
            }
        },
        Some(Some(strategy)) => rsx! {
            EditorLayout { initial_strategy: Some(strategy.clone()) }
        },
    }
}

// =============================================================================
// EditorLayout — main composition component
// =============================================================================

#[component]
fn EditorLayout(initial_strategy: Option<Strategy>) -> Element {
    // =========================================================================
    // All editor state as signals
    // =========================================================================
    let mut canvas_state = use_signal(CanvasState::default);
    let mut drawing_state = use_signal(DrawingState::default);
    let mut strategy_state = use_signal(|| {
        let mut state = StrategyState::default();
        if let Some(ref strategy) = initial_strategy {
            state.load_strategy(strategy.clone());
        }
        state
    });
    let mut undo_manager = use_signal(UndoManager::default);

    // Sidebar collapse state
    let mut left_open = use_signal(|| true);
    let mut right_open = use_signal(|| true);
    let mut bottom_open = use_signal(|| false);

    // Map picker modal
    let mut show_map_picker = use_signal(|| false);

    // Save status
    let mut save_in_progress = use_signal(|| false);

    // Track previous tool for space-to-pan behavior
    let prev_tool = use_hook(|| std::cell::Cell::new(Tool::Select));

    // =========================================================================
    // Playback auto-advance effect
    // =========================================================================
    use_effect(move || {
        let playback = strategy_state.read().playback_state;
        if playback == PlaybackState::Playing {
            // Schedule next advance after 3 seconds
            let callback = Closure::once(Box::new(move || {
                let current = strategy_state.read().playback_state;
                if current == PlaybackState::Playing {
                    let is_last = {
                        let s = strategy_state.read();
                        match s.current_phase_index() {
                            Some(idx) => idx + 1 >= s.phases.len(),
                            None => true,
                        }
                    };
                    if is_last {
                        strategy_state.with_mut(|s| s.playback_state = PlaybackState::Stopped);
                    } else {
                        strategy_state.with_mut(|s| s.next_phase());
                    }
                }
            }) as Box<dyn FnOnce()>);

            let window = web_sys::window().expect("no window");
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                3000,
            );
            callback.forget();
        }
    });

    // =========================================================================
    // Keyboard shortcuts effect — registers global listeners once
    // =========================================================================
    {
        let listeners_registered = use_hook(|| std::cell::Cell::new(false));

        use_effect(move || {
            if listeners_registered.get() {
                return;
            }
            listeners_registered.set(true);

            let window = web_sys::window().expect("no window");
            let document = window.document().expect("no document");

            // Keydown handler
            let keydown_handler = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
                move |event: web_sys::KeyboardEvent| {
                    // Don't handle shortcuts when typing in an input
                    if let Some(target) = event.target() {
                        if let Ok(element) = target.dyn_into::<web_sys::HtmlElement>() {
                            let tag = element.tag_name().to_lowercase();
                            if tag == "input" || tag == "textarea" || tag == "select" {
                                return;
                            }
                        }
                    }

                    let Some(action) = keybindings::from_keyboard_event(&event) else {
                        return;
                    };

                    match action {
                        // Tool selection
                        EditorAction::SelectTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Select);
                        }
                        EditorAction::PanTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Pan);
                        }
                        EditorAction::MarkerTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::PlayerMarker);
                        }
                        EditorAction::RouteTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Route);
                        }
                        EditorAction::AreaTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Area);
                        }
                        EditorAction::ArrowTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Arrow);
                        }
                        EditorAction::TextTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Text);
                        }
                        EditorAction::EraserTool => {
                            drawing_state.with_mut(|d| d.active_tool = Tool::Eraser);
                        }

                        // Panel toggles
                        EditorAction::ToggleLeftPanel => {
                            event.prevent_default();
                            left_open.with_mut(|v| *v = !*v);
                        }
                        EditorAction::ToggleRightPanel => {
                            event.prevent_default();
                            right_open.with_mut(|v| *v = !*v);
                        }
                        EditorAction::ToggleBottomPanel => {
                            event.prevent_default();
                            bottom_open.with_mut(|v| *v = !*v);
                        }
                        EditorAction::ToggleHealthPacks => {
                            event.prevent_default();
                            canvas_state
                                .with_mut(|c| c.show_health_packs = !c.show_health_packs);
                        }

                        // Element actions
                        EditorAction::Delete => {
                            let sel = strategy_state.read().selected_element;
                            if let Some(id) = sel {
                                let removed = strategy_state.with_mut(|s| s.remove_element(id));
                                if let Some((idx, elem)) = removed {
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::RemoveElement {
                                            element: elem,
                                            index: idx,
                                        });
                                    });
                                }
                            }
                        }

                        // Zoom
                        EditorAction::ZoomIn => {
                            event.prevent_default();
                            canvas_state.with_mut(|c| {
                                c.zoom = (c.zoom * 1.2).clamp(0.03, 4.0);
                            });
                        }
                        EditorAction::ZoomOut => {
                            event.prevent_default();
                            canvas_state.with_mut(|c| {
                                c.zoom = (c.zoom / 1.2).clamp(0.03, 4.0);
                            });
                        }
                        EditorAction::ZoomReset => {
                            event.prevent_default();
                            canvas_state.with_mut(|c| {
                                c.zoom = 1.0;
                                c.pan_offset = Position::new(0.0, 0.0);
                            });
                        }

                        // Playback
                        EditorAction::PlayPause => {
                            strategy_state.with_mut(|s| {
                                s.playback_state = match s.playback_state {
                                    PlaybackState::Playing => PlaybackState::Paused,
                                    _ => PlaybackState::Playing,
                                };
                            });
                        }
                        EditorAction::NextPhase => {
                            strategy_state.with_mut(|s| s.next_phase());
                        }
                        EditorAction::PrevPhase => {
                            strategy_state.with_mut(|s| s.prev_phase());
                        }
                        EditorAction::FirstPhase => {
                            strategy_state.with_mut(|s| {
                                if let Some(phase) = s.phases.first() {
                                    s.selected_phase = Some(phase.id);
                                }
                            });
                        }
                        EditorAction::LastPhase => {
                            strategy_state.with_mut(|s| {
                                if let Some(phase) = s.phases.last() {
                                    s.selected_phase = Some(phase.id);
                                }
                            });
                        }

                        // Undo/Redo
                        EditorAction::Undo => {
                            event.prevent_default();
                            let action = undo_manager.with_mut(|u| u.undo());
                            if let Some(action) = action {
                                apply_undo(&mut strategy_state, action);
                            }
                        }
                        EditorAction::Redo => {
                            event.prevent_default();
                            let action = undo_manager.with_mut(|u| u.redo());
                            if let Some(action) = action {
                                apply_redo(&mut strategy_state, action);
                            }
                        }

                        // Save
                        EditorAction::Save => {
                            event.prevent_default();
                            if *save_in_progress.peek() {
                                return;
                            }
                            save_in_progress.set(true);

                            // Snapshot current state for the async save
                            let snapshot = strategy_state.read().clone();
                            let map_id = canvas_state.read().current_map.clone()
                                .unwrap_or_default();
                            let sub_map_id = canvas_state.read().selected_sub_map.clone();

                            spawn(async move {
                                let client = ApiClient::web();
                                let result = if let Some(ref id) = snapshot.strategy_id {
                                    // Update existing strategy
                                    let body = UpdateStrategyRequest {
                                        name: Some(snapshot.name.clone()),
                                        description: Some(snapshot.description.clone()),
                                        visibility: Some(visibility_to_str(snapshot.visibility).to_string()),
                                        elements: Some(snapshot.elements.clone()),
                                        phases: Some(snapshot.phases.clone()),
                                    };
                                    client.put_json::<_, Strategy>(
                                        &format!("/api/strategy/strategies/{id}"),
                                        &body,
                                    ).await
                                } else {
                                    // Create new strategy
                                    let body = CreateStrategyRequest {
                                        name: snapshot.name.clone(),
                                        description: snapshot.description.clone(),
                                        map_id,
                                        sub_map_id,
                                        game_mode: "control".to_string(),
                                        team_id: None,
                                        visibility: visibility_to_str(snapshot.visibility).to_string(),
                                    };
                                    client.post_json::<_, Strategy>(
                                        "/api/strategy/strategies",
                                        &body,
                                    ).await
                                };

                                match result {
                                    Ok(saved) => {
                                        strategy_state.with_mut(|s| {
                                            s.strategy_id = Some(saved.id);
                                            s.owner_id = Some(saved.owner_id);
                                            s.has_unsaved_changes = false;
                                        });
                                        tracing::info!("Strategy saved successfully");
                                    }
                                    Err(e) => {
                                        tracing::error!("Failed to save strategy: {e}");
                                    }
                                }
                                save_in_progress.set(false);
                            });
                        }
                    }
                },
            );

            // Use capture phase so we intercept Tab before browser focus navigation
            document
                .add_event_listener_with_callback_and_bool(
                    "keydown",
                    keydown_handler.as_ref().unchecked_ref(),
                    true,
                )
                .expect("failed to add keydown listener");
            keydown_handler.forget();
        });
    }

    // =========================================================================
    // Read state for rendering
    // =========================================================================
    let cs = canvas_state.read();
    let ds = drawing_state.read();
    let ss = strategy_state.read();

    let left_collapsed = !*left_open.read();
    let right_collapsed = !*right_open.read();
    let bottom_collapsed = !*bottom_open.read();

    // Derive display values
    let map_display = cs
        .current_map
        .as_deref()
        .unwrap_or("None")
        .to_string();
    let element_count = ss.elements.len();
    let tool_display = format!("{}", ds.active_tool);
    let phase_display = {
        let idx = ss.current_phase_index();
        let total = ss.phases.len();
        match idx {
            Some(i) => format!("{}/{}", i + 1, total),
            None => format!("0/{}", total),
        }
    };
    let has_unsaved = ss.has_unsaved_changes;

    // Clone values needed in callbacks and child components
    let zoom_val = cs.zoom;
    let pan_val = cs.pan_offset;
    let floor_val = cs.selected_floor.clone();
    let show_hp = cs.show_health_packs;
    let metadata_val = cs.map_metadata.clone();
    let map_id_val = cs.current_map.clone();
    let tool_val = ds.active_tool;
    let color_val = ds.draw_color;
    let opacity_val = ds.fill_opacity;
    let hero_val = ds.selected_hero.clone();
    let drawing_val = ds.is_drawing;
    let points_val = ds.drawing_points.clone();
    let elements_val = ss.elements.clone();
    let sel_elem = ss.selected_element;
    let sel_phase = ss.selected_phase;

    // Drop borrows before rsx (we've cloned what we need)
    drop(cs);
    drop(ds);
    drop(ss);

    rsx! {
        style { {EDITOR_CSS} }
        style { {crate::components::strategy::MAP_CANVAS_CSS} }

        div { class: "editor-page",

            // ===========================================================
            // Editor container (sidebars + main area)
            // ===========================================================
            div { class: "editor-container",

                // ---- Left sidebar ----
                aside {
                    class: if left_collapsed { "editor-sidebar editor-sidebar-left collapsed" } else { "editor-sidebar editor-sidebar-left" },

                    div { class: "sidebar-header",
                        span { class: "sidebar-title", "Team & Heroes" }
                        button {
                            class: "sidebar-collapse-btn",
                            title: "Toggle panel (Tab)",
                            onclick: move |_| left_open.with_mut(|v| *v = !*v),
                            if left_collapsed { ">" } else { "<" }
                        }
                    }
                    div { class: "sidebar-content",
                        // TODO: TeamPanel component
                        div { class: "panel-placeholder", "TeamPanel" }
                        // TODO: HeroPicker component
                        div { class: "panel-placeholder", "HeroPicker" }
                    }
                }

                // ---- Main canvas area ----
                div { class: "editor-main",

                    // TODO: Toolbar component
                    div { class: "panel-placeholder", style: "border-bottom: 1px solid var(--border, #2a2a3e); padding: 0.4rem 0.75rem;",
                        "Toolbar: {tool_display}"
                    }

                    div { class: "canvas-container",
                        MapCanvas {
                            zoom: zoom_val,
                            pan_offset: pan_val,
                            selected_floor: floor_val,
                            show_health_packs: show_hp,
                            map_metadata: metadata_val,
                            current_map_id: map_id_val,
                            active_tool: tool_val,
                            draw_color: color_val,
                            fill_opacity: opacity_val,
                            selected_hero: hero_val,
                            is_drawing: drawing_val,
                            drawing_points: points_val,
                            elements: elements_val.clone(),
                            selected_element: sel_elem,
                            selected_phase: sel_phase,
                            on_element_add: move |elem: StrategyElement| {
                                let elem_clone = elem.clone();
                                strategy_state.with_mut(|s| s.add_element(elem.clone()));
                                undo_manager.with_mut(|u| {
                                    u.push(UndoableAction::AddElement { element: elem_clone });
                                });
                            },
                            on_element_select: move |id: Option<Uuid>| {
                                strategy_state.with_mut(|s| s.selected_element = id);
                            },
                            on_element_move: move |(id, pos): (Uuid, Position)| {
                                strategy_state.with_mut(|s| {
                                    if let Some(elem) = s.elements.iter_mut().find(|e| e.id == id) {
                                        elem.position = pos;
                                        s.has_unsaved_changes = true;
                                    }
                                });
                            },
                            on_element_drag_end: move |(id, before, after): (Uuid, Position, Position)| {
                                undo_manager.with_mut(|u| {
                                    u.push(UndoableAction::MoveElement { id, before, after });
                                });
                            },
                            on_pan_change: move |pos: Position| {
                                canvas_state.with_mut(|c| c.pan_offset = pos);
                            },
                            on_zoom_change: move |z: f64| {
                                canvas_state.with_mut(|c| c.zoom = z);
                            },
                            on_drawing_start: move |pos: Position| {
                                drawing_state.with_mut(|d| {
                                    d.is_drawing = true;
                                    d.drawing_points = vec![pos];
                                });
                            },
                            on_drawing_continue: move |pos: Position| {
                                drawing_state.with_mut(|d| {
                                    d.drawing_points.push(pos);
                                });
                            },
                            on_drawing_finish: move |_: ()| {
                                let (tool, color, points, phase) = {
                                    let d = drawing_state.read();
                                    let s = strategy_state.read();
                                    (d.active_tool, d.draw_color, d.drawing_points.clone(), s.selected_phase)
                                };

                                if points.len() >= 2 {
                                    let position = points[0];
                                    let element_type = match tool {
                                        Tool::Route => ElementType::Route { points: points.clone() },
                                        Tool::Area => ElementType::Area { points: points.clone() },
                                        _ => ElementType::Route { points: points.clone() },
                                    };

                                    let mut elem = StrategyElement::new(element_type, position)
                                        .with_color(color);
                                    if let Some(pid) = phase {
                                        elem = elem.with_phase(pid);
                                    }

                                    let elem_clone = elem.clone();
                                    strategy_state.with_mut(|s| s.add_element(elem));
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::AddElement { element: elem_clone });
                                    });
                                }

                                drawing_state.with_mut(|d| {
                                    d.is_drawing = false;
                                    d.drawing_points.clear();
                                });
                            },
                            on_arrow_create: move |(start, end): (Position, Position)| {
                                let (color, phase) = {
                                    let d = drawing_state.read();
                                    let s = strategy_state.read();
                                    (d.draw_color, s.selected_phase)
                                };

                                let mut elem = StrategyElement::new(
                                    ElementType::Arrow { end },
                                    start,
                                ).with_color(color);
                                if let Some(pid) = phase {
                                    elem = elem.with_phase(pid);
                                }

                                let elem_clone = elem.clone();
                                strategy_state.with_mut(|s| s.add_element(elem));
                                undo_manager.with_mut(|u| {
                                    u.push(UndoableAction::AddElement { element: elem_clone });
                                });
                            },
                            on_text_create: move |(pos, text): (Position, String)| {
                                let (color, phase) = {
                                    let d = drawing_state.read();
                                    let s = strategy_state.read();
                                    (d.draw_color, s.selected_phase)
                                };

                                let mut elem = StrategyElement::new(
                                    ElementType::Text { content: text, font_size: 18.0 },
                                    pos,
                                ).with_color(color);
                                if let Some(pid) = phase {
                                    elem = elem.with_phase(pid);
                                }

                                let elem_clone = elem.clone();
                                strategy_state.with_mut(|s| s.add_element(elem));
                                undo_manager.with_mut(|u| {
                                    u.push(UndoableAction::AddElement { element: elem_clone });
                                });
                            },
                            on_erase_at: move |pos: Position| {
                                let found = {
                                    let s = strategy_state.read();
                                    s.select_at(pos, 30.0)
                                };
                                if let Some(id) = found {
                                    let removed = strategy_state.with_mut(|s| s.remove_element(id));
                                    if let Some((idx, elem)) = removed {
                                        undo_manager.with_mut(|u| {
                                            u.push(UndoableAction::RemoveElement {
                                                element: elem,
                                                index: idx,
                                            });
                                        });
                                    }
                                }
                            },
                        }
                    }

                    // Bottom panel — element list
                    div {
                        class: if bottom_collapsed { "element-list-wrapper collapsed" } else { "element-list-wrapper" },

                        button {
                            class: "panel-collapse-btn panel-collapse-bottom",
                            title: "Toggle elements panel ([)",
                            onclick: move |_| bottom_open.with_mut(|v| *v = !*v),
                            if bottom_collapsed { "^" } else { "v" }
                        }

                        div { class: "element-list",
                            for (idx, elem) in elements_val.iter().enumerate() {
                                {
                                    let elem_id = elem.id;
                                    let is_selected = sel_elem == Some(elem_id);
                                    let type_name = element_type_name(&elem.element_type);
                                    let color_css = elem.color.to_css();
                                    let label = elem.label.clone().unwrap_or_default();

                                    rsx! {
                                        div {
                                            key: "{elem_id}",
                                            class: if is_selected { "element-list-item selected" } else { "element-list-item" },
                                            onclick: move |_| {
                                                strategy_state.with_mut(|s| {
                                                    s.selected_element = Some(elem_id);
                                                });
                                            },
                                            span { class: "el-number", "{idx + 1}" }
                                            span {
                                                class: "el-color",
                                                style: "background: {color_css};",
                                            }
                                            span { class: "el-type", "{type_name}" }
                                            if !label.is_empty() {
                                                span { style: "color: var(--text-secondary);", "{label}" }
                                            }
                                        }
                                    }
                                }
                            }
                            if elements_val.is_empty() {
                                div { class: "panel-placeholder",
                                    "No elements yet. Use the tools to add markers, routes, and areas."
                                }
                            }
                        }
                    }
                }

                // ---- Right sidebar ----
                aside {
                    class: if right_collapsed { "editor-sidebar editor-sidebar-right collapsed" } else { "editor-sidebar editor-sidebar-right" },

                    div { class: "sidebar-header",
                        button {
                            class: "sidebar-collapse-btn",
                            title: "Toggle panel (Shift+Tab)",
                            onclick: move |_| right_open.with_mut(|v| *v = !*v),
                            if right_collapsed { "<" } else { ">" }
                        }
                        span { class: "sidebar-title", "Properties" }
                    }
                    div { class: "sidebar-content",
                        // TODO: PropertiesPanel component
                        div { class: "panel-placeholder", "PropertiesPanel" }
                        // TODO: Timeline component
                        div { class: "panel-placeholder", "Timeline" }
                    }
                }
            }

            // ===========================================================
            // Status bar
            // ===========================================================
            div { class: "editor-statusbar",
                button {
                    class: "status-item status-map-link",
                    title: "Change map",
                    onclick: move |_| show_map_picker.set(true),
                    "Map: {map_display}"
                }
                span { class: "status-item",
                    "Elements: {element_count}"
                }
                span { class: "status-item",
                    "Tool: {tool_display}"
                }
                span { class: "status-item",
                    "Phase: {phase_display}"
                }
                if *save_in_progress.read() {
                    span { class: "status-item", "Saving..." }
                } else if has_unsaved {
                    span { class: "status-item unsaved", "Unsaved changes" }
                }
            }

            // ===========================================================
            // Map picker modal
            // ===========================================================
            if *show_map_picker.read() {
                div {
                    class: "map-picker-overlay",
                    onclick: move |_| show_map_picker.set(false),

                    div {
                        class: "map-picker-dialog",
                        onclick: move |evt| evt.stop_propagation(),

                        div { class: "map-picker-header",
                            h3 { "Change Map" }
                            button {
                                class: "map-picker-close",
                                onclick: move |_| show_map_picker.set(false),
                                "X"
                            }
                        }
                        div { class: "map-picker-content",
                            // TODO: Populate with map data from scuffed_types::constants
                            p {
                                style: "color: var(--text-muted); text-align: center; padding: 2rem;",
                                "Map picker — will be populated when map constants are available."
                            }
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn visibility_to_str(v: Visibility) -> &'static str {
    match v {
        Visibility::Private => "private",
        Visibility::Unlisted => "unlisted",
        Visibility::Public => "public",
    }
}

/// Get a display name for an element type.
fn element_type_name(element_type: &ElementType) -> &'static str {
    match element_type {
        ElementType::PlayerMarker => "Marker",
        ElementType::Route { .. } => "Route",
        ElementType::Area { .. } => "Area",
        ElementType::Arrow { .. } => "Arrow",
        ElementType::Text { .. } => "Text",
        ElementType::Icon { .. } => "Icon",
        ElementType::Drawing { .. } => "Drawing",
        ElementType::Ability { .. } => "Ability",
    }
}

/// Apply an undo action to the strategy state.
fn apply_undo(strategy_state: &mut Signal<StrategyState>, action: UndoableAction) {
    match action {
        UndoableAction::AddElement { element } => {
            // Undo add = remove
            strategy_state.with_mut(|s| {
                s.remove_element(element.id);
            });
        }
        UndoableAction::RemoveElement { element, index } => {
            // Undo remove = re-insert at original position
            strategy_state.with_mut(|s| {
                let idx = index.min(s.elements.len());
                s.elements.insert(idx, element);
                s.has_unsaved_changes = true;
            });
        }
        UndoableAction::UpdateElement { id, before, .. } => {
            // Undo update = restore previous version
            strategy_state.with_mut(|s| {
                s.update_element(id, before);
            });
        }
        UndoableAction::MoveElement { id, before, .. } => {
            // Undo move = restore previous position
            strategy_state.with_mut(|s| {
                if let Some(elem) = s.elements.iter_mut().find(|e| e.id == id) {
                    elem.position = before;
                    s.has_unsaved_changes = true;
                }
            });
        }
        UndoableAction::AddPhase { phase } => {
            strategy_state.with_mut(|s| {
                s.remove_phase(phase.id);
            });
        }
        UndoableAction::RemovePhase { phase, index } => {
            strategy_state.with_mut(|s| {
                let idx = index.min(s.phases.len());
                s.phases.insert(idx, phase);
                // Reorder
                for (i, p) in s.phases.iter_mut().enumerate() {
                    p.order = i as u32;
                }
                s.has_unsaved_changes = true;
            });
        }
        UndoableAction::AssignHeroToSlot {
            slot,
            previous,
            ..
        } => {
            strategy_state.with_mut(|s| {
                if let Some(prev_hero) = previous {
                    s.assign_hero_to_slot(slot, prev_hero);
                } else {
                    s.clear_slot(slot);
                }
            });
        }
        UndoableAction::ClearSlot {
            slot,
            previous_hero_id,
        } => {
            strategy_state.with_mut(|s| {
                s.assign_hero_to_slot(slot, previous_hero_id);
            });
        }
        UndoableAction::ClearAllSlots { previous } => {
            strategy_state.with_mut(|s| {
                s.team_composition = previous;
                s.has_unsaved_changes = true;
            });
        }
    }
}

/// Apply a redo action to the strategy state.
fn apply_redo(strategy_state: &mut Signal<StrategyState>, action: UndoableAction) {
    match action {
        UndoableAction::AddElement { element } => {
            strategy_state.with_mut(|s| {
                s.add_element(element);
            });
        }
        UndoableAction::RemoveElement { element, .. } => {
            strategy_state.with_mut(|s| {
                s.remove_element(element.id);
            });
        }
        UndoableAction::UpdateElement { id, after, .. } => {
            strategy_state.with_mut(|s| {
                s.update_element(id, after);
            });
        }
        UndoableAction::MoveElement { id, after, .. } => {
            // Redo move = apply new position
            strategy_state.with_mut(|s| {
                if let Some(elem) = s.elements.iter_mut().find(|e| e.id == id) {
                    elem.position = after;
                    s.has_unsaved_changes = true;
                }
            });
        }
        UndoableAction::AddPhase { phase } => {
            strategy_state.with_mut(|s| {
                s.phases.push(phase);
                s.has_unsaved_changes = true;
            });
        }
        UndoableAction::RemovePhase { phase, .. } => {
            strategy_state.with_mut(|s| {
                s.remove_phase(phase.id);
            });
        }
        UndoableAction::AssignHeroToSlot {
            slot, hero_id, ..
        } => {
            strategy_state.with_mut(|s| {
                s.assign_hero_to_slot(slot, hero_id);
            });
        }
        UndoableAction::ClearSlot { slot, .. } => {
            strategy_state.with_mut(|s| {
                s.clear_slot(slot);
            });
        }
        UndoableAction::ClearAllSlots { .. } => {
            strategy_state.with_mut(|s| {
                s.team_composition.clear();
                s.has_unsaved_changes = true;
            });
        }
    }
}
