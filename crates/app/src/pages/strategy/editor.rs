//! Strategy editor page: main composition layer for the strategy editor.
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
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::load_error::{is_retrying, strategy_load_error};
use crate::components::retry_label;

use scuffed_api_client::ApiClient;
use scuffed_types::strategy::{
    Color, ElementType, HeroId, PlaybackState, Position, Strategy, StrategyElement, TeamFormat,
    TeamSlot, TimelinePhase, Tool, Visibility,
};

use crate::components::strategy::{
    HeroPicker, HeroWinRate, MapCanvas, PropertiesPanel, TeamPanel, Timeline, Toolbar,
};
use crate::components::{Toast, use_toast};
use crate::keybindings::{self, CanvasHint, CanvasKey, EditorAction};
use crate::state::editor::{
    CanvasState, DrawingState, StrategyState, map_change_allowed, map_is_kept,
};
use crate::state::maps::{
    PICKER_MODE_ORDER, display_map_name, game_mode_api_str, game_mode_for_map, map_id_for_create,
    maps_for_mode,
};
use crate::state::undo::{UndoManager, UndoableAction};
use crate::state::unsaved::use_unsaved_guard;

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
    /// The drawing made before the first save, so it is not thrown away.
    elements: Vec<StrategyElement>,
    phases: Vec<TimelinePhase>,
}

/// Subset of `/api/strategy/meta` consumed by the editor for inline winrate badges.
#[derive(Debug, Clone, Deserialize)]
struct EditorMetaResponse {
    #[serde(default)]
    personal: Option<EditorMetaPersonal>,
}

#[derive(Debug, Clone, Deserialize)]
struct EditorMetaPersonal {
    #[serde(default)]
    heroes: Vec<EditorHeroEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct EditorHeroEntry {
    hero: String,
    winrate: f64,
}

/// Shown beside the map name once the map can no longer be changed.
const MAP_FIXED_NOTE: &str = "fixed once saved";
/// Shown beside a map picked for a saved strategy that has none stored: the
/// update API takes no map, so the pick is never sent (issue 817).
const MAP_NOT_SAVED_NOTE: &str = "not saved";

/// The note beside the map name in the status bar, if any: why it cannot
/// change, or that a map picked for a saved strategy is not kept.
fn map_status_note(saved: bool, can_change: bool, has_current_map: bool) -> Option<&'static str> {
    if !can_change {
        Some(MAP_FIXED_NOTE)
    } else if saved && has_current_map {
        Some(MAP_NOT_SAVED_NOTE)
    } else {
        None
    }
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
        /* The site shell keeps its top bar (and the tab bar on phone) on
           screen, so the editor takes the rest of the viewport. */
        height: calc(100vh - var(--shell-chrome-height, 0px));
        height: calc(100dvh - var(--shell-chrome-height, 0px));
        background: var(--bg);
        color: var(--text);
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
        background: var(--surface);
        border-color: var(--border);
        transition: width 0.2s ease, min-width 0.2s ease;
        overflow: hidden;
    }
    .editor-sidebar.collapsed {
        width: 36px;
        min-width: 36px;
    }
    .editor-sidebar-left {
        border-right: 1px solid var(--border);
    }
    .editor-sidebar-right {
        border-left: 1px solid var(--border);
    }

    .sidebar-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0.5rem;
        border-bottom: 1px solid var(--border);
        min-height: 36px;
    }
    .sidebar-title {
        font-size: 0.8rem;
        font-weight: 600;
        color: var(--text);
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
        color: var(--text-subtle);
        cursor: pointer;
        padding: 0.2rem 0.4rem;
        font-size: 0.7rem;
        border-radius: 3px;
        transition: background 0.15s, color 0.15s;
        flex-shrink: 0;
    }
    .sidebar-collapse-btn:hover {
        background: var(--surface-sunken);
        color: var(--text);
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
    /* The map's live hint. Always mounted (a status node inserted together
       with its text is often not announced); it shows only while it has
       text, that is while the map holds focus. */
    .canvas-key-hint {
        position: absolute;
        left: 50%;
        bottom: 0.5rem;
        transform: translateX(-50%);
        z-index: 4;
        max-width: calc(100% - 1rem);
        padding: 0.3rem 0.6rem;
        border: 1px solid var(--border);
        border-radius: 4px;
        background: var(--surface);
        color: var(--text);
        font-size: 0.75rem;
        text-align: center;
        pointer-events: none;
    }
    .canvas-key-hint:empty {
        padding: 0;
        border: 0;
    }

    /* Bottom panel (element list) */
    .element-list-wrapper {
        border-top: 1px solid var(--border);
        background: var(--surface);
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
        background: var(--surface-sunken);
        border: none;
        color: var(--text-subtle);
        cursor: pointer;
        font-size: 0.65rem;
        transition: background 0.15s, color 0.15s;
    }
    .panel-collapse-btn:hover {
        background: var(--border);
        color: var(--text);
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
        color: var(--text-3);
        transition: background 0.1s;
    }
    .element-list-item:hover {
        background: var(--surface-sunken);
    }
    .element-list-item.selected {
        background: var(--accent-soft);
        color: var(--text);
    }
    .element-list-item .el-number {
        font-size: 0.7rem;
        color: var(--text-subtle);
        width: 1.5em;
        text-align: right;
    }
    .element-list-item .el-type {
        font-size: 0.7rem;
        padding: 0.1rem 0.35rem;
        border-radius: 3px;
        background: var(--accent-soft);
        color: var(--accent-text);
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
        background: var(--surface);
        border-top: 1px solid var(--border);
        font-size: 0.75rem;
        color: var(--text-subtle);
    }
    .status-item {
        white-space: nowrap;
    }
    /* The map item gives way first, so Phase, Saving... and Unsaved
       changes stay visible on a narrow bar. */
    .status-map {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
    }
    .status-map-link {
        background: none;
        border: none;
        color: var(--text-3);
        cursor: pointer;
        font-size: 0.75rem;
        padding: 0;
        transition: color 0.15s;
    }
    .status-map-link:hover {
        color: var(--accent-text);
    }
    .status-item.unsaved {
        color: var(--accent-text);
        font-weight: 600;
    }
    .status-item.collaborators {
        color: var(--text-3);
    }

    /* Map picker modal */
    .map-picker-overlay {
        position: fixed;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        background: var(--overlay);
        display: flex;
        align-items: center;
        justify-content: center;
        /* Above the site top bar and phone tab bar (100), below the drawer. */
        z-index: 110;
    }
    .map-picker-dialog {
        background: var(--surface);
        border: 1px solid var(--border);
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
        border-bottom: 1px solid var(--border);
    }
    .map-picker-header h3 {
        margin: 0;
        color: var(--text);
        font-family: var(--font-head);
    }
    .map-picker-close {
        background: none;
        border: none;
        color: var(--text-subtle);
        font-size: 1.2rem;
        cursor: pointer;
        padding: 0.25rem;
        transition: color 0.15s;
    }
    .map-picker-close:hover {
        color: var(--text);
    }
    .map-picker-content {
        padding: 1rem;
        overflow-y: auto;
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 1rem;
    }
    .map-picker-error {
        margin: 0;
        padding: 0.65rem 0.75rem;
        border: 1px solid var(--loss);
        border-radius: 8px;
        background: color-mix(in srgb, var(--loss) 12%, transparent);
        color: var(--loss);
        font-size: 0.85rem;
    }
    .map-picker-mode-title {
        margin: 0 0 0.5rem;
        color: var(--text-3);
        font-size: 0.7rem;
        font-weight: 600;
        letter-spacing: 0.06em;
        text-transform: uppercase;
    }
    .map-picker-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
        gap: 0.5rem;
    }
    .map-picker-item {
        padding: 0.55rem 0.65rem;
        border: 1px solid var(--border);
        border-radius: 8px;
        background: var(--bg);
        color: var(--text);
        font-size: 0.8rem;
        text-align: left;
        cursor: pointer;
        transition: border-color 0.15s, background 0.15s, color 0.15s;
    }
    .map-picker-item:hover {
        border-color: var(--accent-solid);
        color: var(--text);
        background: var(--surface-sunken);
    }
    .map-picker-item.selected {
        border-color: var(--accent-solid);
        background: var(--accent-soft);
        color: var(--accent-text);
        font-weight: 600;
    }

    /* Placeholder sections for components not yet wired */
    .panel-placeholder {
        padding: 0.75rem;
        text-align: center;
        color: var(--text-subtle);
        font-size: 0.8rem;
        font-style: italic;
    }
"#;

/// The map picker dialog, its heading, its Close button and the selected map.
const MAP_PICKER_ID: &str = "map-picker";
const MAP_PICKER_TITLE_ID: &str = "map-picker-title";
const MAP_PICKER_CLOSE_ID: &str = "map-picker-close";
const MAP_PICKER_SELECTED_ID: &str = "map-picker-selected";
/// The status bar's map item: the "Map:" button that opens the picker while
/// the map can change, otherwise plain text with `tabindex="-1"`. Focus
/// returns to it when the picker closes.
const EDITOR_MAP_BUTTON_ID: &str = "editor-map-button";

/// The picker id of the stored map, through the same lookup the rest of the
/// app uses (a slug or a display name, any surrounding spaces), so the
/// current map is marked and focused however it was stored. `None` for no
/// map or one the picker does not list.
fn picker_current_id(current_map: Option<&str>) -> Option<&'static str> {
    current_map
        .and_then(crate::state::maps::pickable_map_by_id)
        .map(|map| map.id)
}

/// What a key does while the map picker is open. Every other key goes to
/// the focused control in the picker; no editor shortcut runs behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerKey {
    /// Esc: close the picker, focus back on the Map button.
    Close,
    /// Tab or Shift+Tab: wrap inside the picker.
    KeepFocusInside,
    /// Ctrl+S or Cmd+S: the editor does not save behind the modal (or the
    /// phone drawer), and the browser's Save Page dialog must not open either.
    BlockBrowserSave,
}

fn picker_key(binding: &keybindings::Keybinding) -> Option<PickerKey> {
    let held = binding.modifiers;
    match binding.key.as_str() {
        "escape" => Some(PickerKey::Close),
        "tab" => Some(PickerKey::KeepFocusInside),
        "s" if held.ctrl || held.meta => Some(PickerKey::BlockBrowserSave),
        _ => None,
    }
}

/// Accessible name for a side panel's collapse button.
fn panel_button_label(panel: &'static str, open: bool) -> String {
    if open {
        format!("Hide {panel} panel")
    } else {
        format!("Show {panel} panel")
    }
}

/// `inert` for the editor behind the open map picker. Text or removed,
/// never a bool (see `layouts::shell::background_inert`).
fn behind_picker(open: bool) -> Option<&'static str> {
    open.then_some("true")
}

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
    let mut refresh = use_signal(|| 0u32);
    let id_clone = id.clone();
    let strategy_resource = use_resource(move || {
        let id = id_clone.clone();
        let _ = refresh();
        async move {
            ApiClient::web()
                .fetch::<Strategy>(&format!("/api/strategy/strategies/{id}"))
                .await
                .map_err(|e| crate::client_error::logged_message("strategy load", &e))
        }
    });

    let data = strategy_resource.read();
    match data.as_ref() {
        None => rsx! {
            div {
                style: "display: flex; align-items: center; justify-content: center; height: calc(100vh - var(--shell-chrome-height, 0px)); color: var(--text-subtle);",
                "Loading strategy..."
            }
        },
        Some(Err(err)) => {
            let shown = strategy_load_error(err);
            let retrying = is_retrying(*strategy_resource.state().read());
            let label = retry_label(retrying);
            rsx! {
                div {
                    style: "display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 0.75rem; height: calc(100vh - var(--shell-chrome-height, 0px)); color: var(--loss);",
                    role: "alert",
                    aria_busy: if retrying { "true" },
                    "{shown.message}"
                    if shown.retry {
                        button {
                            r#type: "button",
                            class: "fetch-error__retry is-compact",
                            disabled: retrying,
                            onclick: move |_| refresh += 1,
                            "{label}"
                        }
                    }
                }
            }
        }
        Some(Ok(strategy)) => rsx! {
            EditorLayout { initial_strategy: Some(strategy.clone()) }
        },
    }
}

// =============================================================================
// EditorLayout: main composition component
// =============================================================================

#[component]
fn EditorLayout(initial_strategy: Option<Strategy>) -> Element {
    // =========================================================================
    // All editor state as signals
    // =========================================================================
    let mut canvas_state = use_signal({
        let initial = initial_strategy.clone();
        move || {
            let mut canvas = CanvasState::default();
            if let Some(ref strategy) = initial {
                canvas.load_strategy(strategy);
            }
            canvas
        }
    });
    let mut drawing_state = use_signal(DrawingState::default);
    let mut strategy_state = use_signal(|| {
        let mut state = StrategyState::default();
        if let Some(ref strategy) = initial_strategy {
            state.load_strategy(strategy.clone());
        }
        // The team of a strategy that was just created here (issue 817).
        state.take_team_hand_off();
        state
    });
    // Ask before a link, a reload or closing the tab drops an unsaved
    // drawing. Up here, ahead of any way out of the component, so it runs
    // on every render; a save that clears the flag takes the guard off.
    // The same `has_unsaved` feeds the toolbar and the "Unsaved changes"
    // line below, so the prompt and the line cannot disagree.
    let has_unsaved = strategy_state.read().has_unsaved_changes;
    use_unsaved_guard(has_unsaved);
    let mut undo_manager = use_signal(UndoManager::default);

    // Sidebar collapse state
    let mut left_open = use_signal(|| true);
    let mut right_open = use_signal(|| true);
    let mut bottom_open = use_signal(|| false);

    // Map picker modal: open when creating (or a loaded strategy has no map).
    let mut show_map_picker = use_signal(|| {
        initial_strategy
            .as_ref()
            .map(|s| s.map_id.trim().is_empty())
            .unwrap_or(true)
    });
    let mut map_picker_error = use_signal(|| None::<String>);
    // Set when the picker closes: focus goes back to the status bar's Map
    // button after the render that removed the picker and `inert`.
    let mut picker_refocus = use_signal(|| false);
    let picker_open = *show_map_picker.read();
    // The picker is a modal: the shell chrome goes inert while it is open.
    crate::layouts::use_page_modal(picker_open);
    // Focus moves into the picker when it opens: the current map, or Close
    // when there is none or it is not in the picker.
    use_effect(move || {
        if show_map_picker() && !crate::layouts::focus_element(MAP_PICKER_SELECTED_ID) {
            crate::layouts::focus_element(MAP_PICKER_CLOSE_ID);
        }
    });
    use_effect(move || {
        if picker_refocus() {
            picker_refocus.set(false);
            crate::layouts::focus_element(EDITOR_MAP_BUTTON_ID);
        }
    });
    let mut close_picker = move || {
        show_map_picker.set(false);
        picker_refocus.set(true);
    };

    // Save status
    let save_in_progress = use_signal(|| false);
    let toast = use_toast();
    let navigator = use_navigator();

    // Personal winrates per hero from /api/strategy/meta: fetched once on mount.
    // None while loading or when the user is not an authed org member.
    let hero_winrates = use_resource(|| async {
        let resp = ApiClient::web()
            .fetch::<EditorMetaResponse>("/api/strategy/meta")
            .await
            .ok()?;
        let personal = resp.personal?;
        Some(
            personal
                .heroes
                .into_iter()
                .map(|h| HeroWinRate {
                    hero_name: h.hero,
                    winrate: h.winrate,
                })
                .collect::<Vec<_>>(),
        )
    });

    let save_ctx = SaveContext {
        strategy_state,
        canvas_state,
        save_in_progress,
        show_map_picker,
        map_picker_error,
        toast,
        navigator,
        opened_as_new: initial_strategy.is_none(),
    };

    // The map's key hint: shown on the first key pressed in the canvas (not
    // on a mouse focus, so it does not cover the map while drawing), cleared
    // when focus leaves it. The status node stays mounted; only its text
    // changes.
    let mut canvas_hint = use_signal(CanvasHint::default);
    // Esc out of the canvas lands on `main#main`. Through the mounted handle,
    // so it works in the desktop webview as well as on web.
    let mut focus_main = crate::layouts::use_focus_main();

    // Track previous tool for space-to-pan behavior
    let _prev_tool = use_hook(|| std::cell::Cell::new(Tool::Select));

    // =========================================================================
    // Playback auto-advance
    // =========================================================================
    // One scope-owned timer, keyed on (playback state, selected phase). Pausing,
    // stopping, changing phase or leaving the editor drops the pending future,
    // so no timer outlives the editor or runs twice.
    let playback_key = use_memo(move || {
        let s = strategy_state.read();
        (s.playback_state, s.selected_phase)
    });
    let _playback_timer = use_resource(move || {
        let (playback, _phase) = playback_key();
        async move {
            if playback != PlaybackState::Playing {
                return;
            }
            playback_pause(PLAYBACK_PHASE_MS).await;
            strategy_state.with_mut(|s| s.playback_tick());
        }
    });

    // =========================================================================
    // Keyboard shortcuts: document keydown, capture phase. Removed when the
    // editor unmounts. Tab is not handled here: `from_keyboard_event` never
    // resolves it, so outside the map canvas Tab moves focus as on any page.
    // The Tab panel toggle lives on the map canvas element's own `onkeydown`
    // and reaches the editor through `on_canvas_key`.
    //
    // Where the key landed decides whether a shortcut fires
    // (`keybindings::shortcut_allowed`, FrozenTear 2026-11-17): every
    // shortcut, Ctrl/Cmd chords included, fires only with focus inside the
    // map canvas. On a button, link, slider, select, text field, the page
    // body or `main` the key stays with the browser and the focused element.
    // Space on a focused button only presses the button.
    // =========================================================================
    crate::layouts::use_document_keydown_capture(move |event: web_sys::KeyboardEvent| {
        // The site drawer (phone layout) covers the page and makes `main`
        // inert: the editor behind it takes no shortcut, even with focus
        // on the body after a click on blank drawer space. Ctrl/Cmd+S still
        // stops the browser's Save Page dialog, as in the picker (prevent
        // default only, nothing is saved).
        if crate::layouts::main_is_inert() {
            let binding = keybindings::Keybinding::from_keyboard_event(&event);
            if picker_key(&binding) == Some(PickerKey::BlockBrowserSave) {
                event.prevent_default();
            }
            return;
        }
        // The map picker is a modal: while it is open the editor behind it
        // takes no shortcut. Esc closes it and Tab stays inside it.
        if *show_map_picker.peek() {
            match picker_key(&keybindings::Keybinding::from_keyboard_event(&event)) {
                Some(PickerKey::Close) => {
                    event.prevent_default();
                    close_picker();
                }
                Some(PickerKey::KeepFocusInside) => {
                    crate::layouts::trap_tab_within(MAP_PICKER_ID, &event);
                }
                Some(PickerKey::BlockBrowserSave) => event.prevent_default(),
                None => {}
            }
            return;
        }

        let Some(action) = keybindings::from_keyboard_event(&event) else {
            return;
        };
        // A shortcut that fires owns the key: no page scroll on Space or the
        // arrows, no browser Back on Backspace.
        event.prevent_default();

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

            // Panel toggles. No default key resolves to the left panel
            // here (it is Tab inside the canvas); a custom binding could.
            EditorAction::ToggleLeftPanel => {
                left_open.with_mut(|v| *v = !*v);
            }
            EditorAction::ToggleRightPanel => {
                right_open.with_mut(|v| *v = !*v);
            }
            EditorAction::ToggleBottomPanel => {
                bottom_open.with_mut(|v| *v = !*v);
            }
            EditorAction::ToggleHealthPacks => {
                canvas_state.with_mut(|c| c.show_health_packs = !c.show_health_packs);
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
                canvas_state.with_mut(|c| {
                    c.zoom = (c.zoom * 1.2).clamp(0.03, 4.0);
                });
            }
            EditorAction::ZoomOut => {
                canvas_state.with_mut(|c| {
                    c.zoom = (c.zoom / 1.2).clamp(0.03, 4.0);
                });
            }
            EditorAction::ZoomReset => {
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
                let action = undo_manager.with_mut(|u| u.undo());
                if let Some(action) = action {
                    apply_undo(&mut strategy_state, action);
                }
            }
            EditorAction::Redo => {
                let action = undo_manager.with_mut(|u| u.redo());
                if let Some(action) = action {
                    apply_redo(&mut strategy_state, action);
                }
            }

            // Save
            EditorAction::Save => {
                save_strategy(save_ctx);
            }
        }
    });

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
    let map_display = match (
        cs.current_map.as_deref(),
        cs.selected_sub_map.as_deref().filter(|s| !s.is_empty()),
    ) {
        (Some(map), Some(sub)) => format!("{} / {sub}", display_map_name(map)),
        (Some(map), None) => display_map_name(map),
        (None, _) => "None".to_string(),
    };
    let current_pick = picker_current_id(cs.current_map.as_deref());
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
    // The update API takes no map, so the map is fixed once the strategy is
    // saved (and while a save is in flight).
    let saved = ss.strategy_id.is_some();
    let can_change_map = map_change_allowed(
        saved,
        *save_in_progress.read(),
        map_is_kept(saved, ss.map_saved, cs.current_map.is_some()),
    );
    let map_label = match map_status_note(saved, can_change_map, cs.current_map.is_some()) {
        Some(note) => format!("Map: {map_display} ({note})"),
        None => format!("Map: {map_display}"),
    };

    // Clone values needed in callbacks and child components
    let zoom_val = cs.zoom;
    let pan_val = cs.pan_offset;
    let floor_val = cs.selected_floor.clone();
    let floor_val_toolbar = cs.selected_floor.clone();
    let show_hp = cs.show_health_packs;
    let metadata_val = cs.map_metadata.clone();
    let map_id_val = cs.current_map.clone();
    let floors_val = cs
        .map_metadata
        .as_ref()
        .map(|m| m.floors.clone())
        .unwrap_or_default();
    let tool_val = ds.active_tool;
    let color_val = ds.draw_color;
    let opacity_val = ds.fill_opacity;
    let hero_val = ds.selected_hero.clone();
    let drawing_val = ds.is_drawing;
    let points_val = ds.drawing_points.clone();
    let elements_val = ss.elements.clone();
    let sel_elem = ss.selected_element;
    let sel_phase = ss.selected_phase;

    // Panel-bound values
    let team_format_val = ss.team_format;
    let composition_val = ss.team_composition.clone();
    let phases_val = ss.phases.clone();
    let playback_val = ss.playback_state;
    let name_val = ss.name.clone();
    let visibility_val = ss.visibility;
    let selected_element_obj = ss
        .selected_element
        .and_then(|id| ss.elements.iter().find(|e| e.id == id).cloned());

    // Drop borrows before rsx (we've cloned what we need)
    drop(cs);
    drop(ds);
    drop(ss);

    let hint_text = canvas_hint.read().text();
    let can_undo = undo_manager.read().can_undo();
    let can_redo = undo_manager.read().can_redo();
    let saving_val = *save_in_progress.read();

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-strategy-editor-editor-css", css: EDITOR_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-strategy-editor-crate-components-strategy-map-canvas-css", css: crate::components::strategy::MAP_CANVAS_CSS }

        div { class: "editor-page",

            // ===========================================================
            // Editor container (sidebars + main area)
            // ===========================================================
            div { class: "editor-container", inert: behind_picker(picker_open),

                // ---- Left sidebar ----
                aside {
                    class: if left_collapsed { "editor-sidebar editor-sidebar-left collapsed" } else { "editor-sidebar editor-sidebar-left" },

                    div { class: "sidebar-header",
                        span { class: "sidebar-title", "Team & Heroes" }
                        button {
                            class: "sidebar-collapse-btn",
                            r#type: "button",
                            title: "Toggle panel (Tab while the map has focus)",
                            aria_label: panel_button_label("team", !left_collapsed),
                            aria_expanded: if left_collapsed { "false" } else { "true" },
                            onclick: move |_| left_open.with_mut(|v| *v = !*v),
                            if left_collapsed { ">" } else { "<" }
                        }
                    }
                    div { class: "sidebar-content",
                        TeamPanel {
                            team_format: team_format_val,
                            composition: composition_val,
                            selected_hero: hero_val.clone(),
                            on_format_change: move |f: TeamFormat| {
                                strategy_state.with_mut(|s| s.set_team_format(f));
                            },
                            on_assign: move |slot: TeamSlot| {
                                let hero = drawing_state.read().selected_hero.clone();
                                if let Some(hero_id) = hero {
                                    let previous = strategy_state
                                        .read()
                                        .team_composition
                                        .iter()
                                        .find(|h| h.slot == slot)
                                        .map(|h| h.hero_id.clone());
                                    strategy_state
                                        .with_mut(|s| s.assign_hero_to_slot(slot, hero_id.clone()));
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::AssignHeroToSlot {
                                            slot,
                                            hero_id,
                                            previous,
                                        });
                                    });
                                }
                            },
                            on_clear_slot: move |slot: TeamSlot| {
                                let prev = strategy_state
                                    .read()
                                    .team_composition
                                    .iter()
                                    .find(|h| h.slot == slot)
                                    .map(|h| h.hero_id.clone());
                                strategy_state.with_mut(|s| s.clear_slot(slot));
                                if let Some(hero_id) = prev {
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::ClearSlot {
                                            slot,
                                            previous_hero_id: hero_id,
                                        });
                                    });
                                }
                            },
                            on_clear_all: move |_| {
                                let prev = strategy_state.read().team_composition.clone();
                                if !prev.is_empty() {
                                    strategy_state
                                        .with_mut(|s| s.set_team_composition(Vec::new()));
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::ClearAllSlots { previous: prev });
                                    });
                                }
                            },
                        }
                        {
                            let winrates = hero_winrates.read().as_ref().and_then(|r| r.clone());
                            rsx! {
                                HeroPicker {
                                    selected_hero: hero_val.clone(),
                                    on_select: move |id: String| {
                                        drawing_state.with_mut(|d| d.selected_hero = Some(id));
                                    },
                                    hero_winrates: winrates,
                                }
                            }
                        }
                    }
                }

                // ---- Main canvas area ----
                div { class: "editor-main",

                    Toolbar {
                        active_tool: tool_val,
                        on_tool_change: move |t: Tool| {
                            drawing_state.with_mut(|d| d.active_tool = t);
                        },
                        draw_color: color_val,
                        on_color_change: move |c: Color| {
                            drawing_state.with_mut(|d| d.draw_color = c);
                        },
                        fill_opacity: opacity_val,
                        on_fill_opacity_change: move |o: f32| {
                            drawing_state.with_mut(|d| d.fill_opacity = o);
                        },
                        can_undo,
                        can_redo,
                        on_undo: move |_| {
                            let action = undo_manager.with_mut(|u| u.undo());
                            if let Some(action) = action {
                                apply_undo(&mut strategy_state, action);
                            }
                        },
                        on_redo: move |_| {
                            let action = undo_manager.with_mut(|u| u.redo());
                            if let Some(action) = action {
                                apply_redo(&mut strategy_state, action);
                            }
                        },
                        zoom: zoom_val,
                        on_zoom_in: move |_| {
                            canvas_state.with_mut(|c| c.zoom = (c.zoom * 1.2).clamp(0.03, 4.0));
                        },
                        on_zoom_out: move |_| {
                            canvas_state.with_mut(|c| c.zoom = (c.zoom / 1.2).clamp(0.03, 4.0));
                        },
                        on_zoom_reset: move |_| {
                            canvas_state.with_mut(|c| {
                                c.zoom = 1.0;
                                c.pan_offset = Position::new(0.0, 0.0);
                            });
                        },
                        floors: floors_val,
                        selected_floor: floor_val_toolbar,
                        on_floor_change: move |f: String| {
                            canvas_state.with_mut(|c| c.selected_floor = Some(f));
                        },
                        show_health_packs: show_hp,
                        on_toggle_health_packs: move |_| {
                            canvas_state.with_mut(|c| c.show_health_packs = !c.show_health_packs);
                        },
                        strategy_name: name_val,
                        on_name_change: move |n: String| {
                            strategy_state.with_mut(|s| {
                                s.name = n;
                                s.has_unsaved_changes = true;
                            });
                        },
                        has_unsaved_changes: has_unsaved,
                        on_save: move |_| {
                            save_strategy(save_ctx);
                        },
                        saving: saving_val,
                        visibility: visibility_val,
                        on_visibility_change: move |v: Visibility| {
                            strategy_state.with_mut(|s| {
                                s.visibility = v;
                                s.has_unsaved_changes = true;
                            });
                        },
                    }

                    div { class: "canvas-container",
                        onfocusout: move |_| canvas_hint.set(CanvasHint::Off),
                        p { class: "canvas-key-hint", role: "status", "{hint_text}" }
                        MapCanvas {
                            zoom: zoom_val,
                            pan_offset: pan_val,
                            selected_floor: floor_val,
                            show_health_packs: show_hp,
                            map_metadata: metadata_val,
                            current_map_id: map_id_val,
                            // Keys the canvas element itself resolved while it
                            // held focus (`keybindings::canvas_key`). Tab
                            // toggles the left panel and focus stays. Esc first
                            // drops a drawing or selection in progress (the
                            // canvas clears its own arrow and the selection);
                            // with nothing in progress Esc sends focus to the
                            // main landmark. Shift+Tab is left to the browser,
                            // which moves to the previous stop.
                            on_canvas_key: move |key: CanvasKey| match key {
                                CanvasKey::ToggleLeftPanel => {
                                    let now_open = left_open.with_mut(|v| {
                                        *v = !*v;
                                        *v
                                    });
                                    canvas_hint.set(CanvasHint::PanelToggled(now_open));
                                }
                                CanvasKey::CancelInProgress => {
                                    drawing_state.with_mut(|d| {
                                        d.is_drawing = false;
                                        d.drawing_points.clear();
                                    });
                                }
                                CanvasKey::LeaveToShell => focus_main(),
                                CanvasKey::LeaveBackwards => {}
                            },
                            on_canvas_keydown: move |_| {
                                let next = canvas_hint.peek().after_key();
                                if next != *canvas_hint.peek() {
                                    canvas_hint.set(next);
                                }
                            },
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

                    // Bottom panel: element list
                    div {
                        class: if bottom_collapsed { "element-list-wrapper collapsed" } else { "element-list-wrapper" },

                        button {
                            class: "panel-collapse-btn panel-collapse-bottom",
                            r#type: "button",
                            title: "Toggle elements panel ([ while the map has focus)",
                            aria_label: panel_button_label("elements", !bottom_collapsed),
                            aria_expanded: if bottom_collapsed { "false" } else { "true" },
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
                                                span { style: "color: var(--text-3);", "{label}" }
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
                            r#type: "button",
                            title: "Toggle panel (] while the map has focus)",
                            aria_label: panel_button_label("properties", !right_collapsed),
                            aria_expanded: if right_collapsed { "false" } else { "true" },
                            onclick: move |_| right_open.with_mut(|v| *v = !*v),
                            if right_collapsed { "<" } else { ">" }
                        }
                        span { class: "sidebar-title", "Properties" }
                    }
                    div { class: "sidebar-content",
                        PropertiesPanel {
                            element: selected_element_obj,
                            phases: phases_val.clone(),
                            on_label_change: move |(id, label): (Uuid, Option<String>)| {
                                update_element_field(strategy_state, undo_manager, id, move |e| {
                                    e.label = label;
                                });
                            },
                            on_color_change: move |(id, color): (Uuid, Color)| {
                                update_element_field(strategy_state, undo_manager, id, move |e| {
                                    e.color = color;
                                });
                            },
                            on_hero_change: move |(id, hero): (Uuid, Option<HeroId>)| {
                                update_element_field(strategy_state, undo_manager, id, move |e| {
                                    e.hero_id = hero;
                                });
                            },
                            on_phase_change: move |(id, phase): (Uuid, Option<Uuid>)| {
                                update_element_field(strategy_state, undo_manager, id, move |e| {
                                    e.phase_id = phase;
                                });
                            },
                            on_move_up: move |id: Uuid| {
                                strategy_state.with_mut(|s| {
                                    if let Some(i) = s.elements.iter().position(|e| e.id == id)
                                        && i > 0
                                    {
                                        s.elements.swap(i, i - 1);
                                        s.has_unsaved_changes = true;
                                    }
                                });
                            },
                            on_move_down: move |id: Uuid| {
                                strategy_state.with_mut(|s| {
                                    if let Some(i) = s.elements.iter().position(|e| e.id == id)
                                        && i + 1 < s.elements.len()
                                    {
                                        s.elements.swap(i, i + 1);
                                        s.has_unsaved_changes = true;
                                    }
                                });
                            },
                            on_delete: move |id: Uuid| {
                                let removed = strategy_state.with_mut(|s| s.remove_element(id));
                                if let Some((idx, elem)) = removed {
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::RemoveElement {
                                            element: elem,
                                            index: idx,
                                        });
                                    });
                                }
                            },
                        }
                        Timeline {
                            phases: phases_val,
                            selected_phase: sel_phase,
                            playback_state: playback_val,
                            on_select_phase: move |p: Option<Uuid>| {
                                strategy_state.with_mut(|s| s.selected_phase = p);
                            },
                            on_add_phase: move |name: String| {
                                let id = strategy_state.with_mut(|s| s.add_phase(name));
                                let phase = strategy_state
                                    .read()
                                    .phases
                                    .iter()
                                    .find(|p| p.id == id)
                                    .cloned();
                                if let Some(phase) = phase {
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::AddPhase { phase });
                                    });
                                }
                            },
                            on_delete_phase: move |id: Uuid| {
                                let removed = strategy_state.with_mut(|s| s.remove_phase(id));
                                if let Some((index, phase)) = removed {
                                    undo_manager.with_mut(|u| {
                                        u.push(UndoableAction::RemovePhase { phase, index });
                                    });
                                }
                            },
                            on_play_pause: move |_| {
                                strategy_state.with_mut(|s| {
                                    s.playback_state = match s.playback_state {
                                        PlaybackState::Playing => PlaybackState::Paused,
                                        _ => PlaybackState::Playing,
                                    };
                                });
                            },
                            on_next_phase: move |_| {
                                strategy_state.with_mut(|s| s.next_phase());
                            },
                            on_prev_phase: move |_| {
                                strategy_state.with_mut(|s| s.prev_phase());
                            },
                            on_first_phase: move |_| {
                                strategy_state.with_mut(|s| {
                                    if let Some(p) = s.phases.first() {
                                        s.selected_phase = Some(p.id);
                                    }
                                });
                            },
                            on_last_phase: move |_| {
                                strategy_state.with_mut(|s| {
                                    if let Some(p) = s.phases.last() {
                                        s.selected_phase = Some(p.id);
                                    }
                                });
                            },
                        }
                    }
                }
            }

            // ===========================================================
            // Status bar
            // ===========================================================
            div { class: "editor-statusbar", inert: behind_picker(picker_open),
                if can_change_map {
                    button {
                        id: EDITOR_MAP_BUTTON_ID,
                        class: "status-item status-map status-map-link",
                        r#type: "button",
                        title: "Change map",
                        onclick: move |_| show_map_picker.set(true),
                        "{map_label}"
                    }
                } else {
                    // Not a control: the map of a saved strategy cannot be
                    // changed, and the reason is in the text itself. It keeps
                    // the id (and takes script focus) so focus still has a
                    // place to return to when the picker closes.
                    span {
                        id: EDITOR_MAP_BUTTON_ID,
                        class: "status-item status-map",
                        tabindex: "-1",
                        "{map_label}"
                    }
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
            // A modal dialog: named by its heading, focus moves in on open
            // and stays in (the keydown listener above), Esc or Close or a
            // click outside closes it and focus returns to the Map button.
            // The editor behind it and the shell chrome are inert.
            if picker_open {
                div {
                    class: "map-picker-overlay",
                    onclick: move |_| close_picker(),

                    div {
                        id: MAP_PICKER_ID,
                        class: "map-picker-dialog",
                        role: "dialog",
                        aria_modal: "true",
                        aria_labelledby: MAP_PICKER_TITLE_ID,
                        onclick: move |evt| evt.stop_propagation(),

                        div { class: "map-picker-header",
                            h3 { id: MAP_PICKER_TITLE_ID,
                                if canvas_state.read().current_map.is_some() {
                                    "Change Map"
                                } else {
                                    "Select a Map"
                                }
                            }
                            button {
                                id: MAP_PICKER_CLOSE_ID,
                                class: "map-picker-close",
                                r#type: "button",
                                aria_label: "Close map picker",
                                onclick: move |_| close_picker(),
                                "X"
                            }
                        }
                        div { class: "map-picker-content",
                            if let Some(message) = map_picker_error.read().as_ref() {
                                p { class: "map-picker-error", "{message}" }
                            }
                            for mode in PICKER_MODE_ORDER {
                                {
                                    let maps: Vec<_> = maps_for_mode(*mode).collect();
                                    let mode_label = mode.to_string();
                                    rsx! {
                                        if !maps.is_empty() {
                                            div { class: "map-picker-mode",
                                                h4 { class: "map-picker-mode-title", "{mode_label}" }
                                                div { class: "map-picker-grid",
                                                    for map in maps {
                                                        {
                                                            let map_id = map.id;
                                                            let selected = current_pick == Some(map_id);
                                                            let item_class = if selected {
                                                                "map-picker-item selected"
                                                            } else {
                                                                "map-picker-item"
                                                            };
                                                            rsx! {
                                                                button {
                                                                    id: if selected { Some(MAP_PICKER_SELECTED_ID) } else { None },
                                                                    class: "{item_class}",
                                                                    r#type: "button",
                                                                    aria_current: if selected { Some("true") } else { None },
                                                                    onclick: move |_| {
                                                                        let (saved, map_saved) = {
                                                                            let s = strategy_state.peek();
                                                                            (s.strategy_id.is_some(), s.map_saved)
                                                                        };
                                                                        let allowed = map_change_allowed(
                                                                            saved,
                                                                            *save_in_progress.peek(),
                                                                            map_is_kept(
                                                                                saved,
                                                                                map_saved,
                                                                                canvas_state.peek().current_map.is_some(),
                                                                            ),
                                                                        );
                                                                        if allowed {
                                                                            canvas_state.with_mut(|c| {
                                                                                c.select_map(map_id.to_string(), None);
                                                                            });
                                                                            // The map is sent only on create, so it
                                                                            // is an unsaved change only before that.
                                                                            if !saved {
                                                                                strategy_state.with_mut(|s| {
                                                                                    s.has_unsaved_changes = true;
                                                                                });
                                                                            }
                                                                        }
                                                                        map_picker_error.set(None);
                                                                        close_picker();
                                                                    },
                                                                    "{map.name}"
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
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

/// Apply a single-field edit to an element and record it for undo.
/// Shared by the PropertiesPanel callbacks (label/color/hero/phase).
fn update_element_field(
    mut strategy_state: Signal<StrategyState>,
    mut undo_manager: Signal<UndoManager>,
    id: Uuid,
    mutate: impl FnOnce(&mut StrategyElement),
) {
    let before = strategy_state
        .read()
        .elements
        .iter()
        .find(|e| e.id == id)
        .cloned();
    if let Some(before) = before {
        let mut after = before.clone();
        mutate(&mut after);
        if after == before {
            return;
        }
        strategy_state.with_mut(|s| s.update_element(id, after.clone()));
        undo_manager.with_mut(|u| {
            u.push(UndoableAction::UpdateElement { id, before, after });
        });
    }
}

/// Seconds a phase shows during playback before the next one.
const PLAYBACK_PHASE_MS: u32 = 3_000;

async fn playback_pause(ms: u32) {
    #[cfg(feature = "web")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(feature = "web"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(u64::from(ms))).await;
    }
}

/// Everything a save needs. Copy, so the Save shortcut and the toolbar button
/// can both hold one.
#[derive(Clone, Copy)]
struct SaveContext {
    strategy_state: Signal<StrategyState>,
    canvas_state: Signal<CanvasState>,
    save_in_progress: Signal<bool>,
    show_map_picker: Signal<bool>,
    map_picker_error: Signal<Option<String>>,
    toast: crate::components::ToastState,
    navigator: dioxus::router::Navigator,
    /// Mounted at `/strategy/editor` (no strategy yet). After the first save
    /// the editor moves to the saved strategy's own route.
    opened_as_new: bool,
}

/// Persist the strategy to the API. Shared by the Save keybinding and the
/// Toolbar's save button. No-op while a save is already in flight.
///
/// A new strategy is created with its drawing in one request. The dirty flag
/// is cleared only when the saved snapshot still matches the editor.
fn save_strategy(ctx: SaveContext) {
    let SaveContext {
        mut strategy_state,
        canvas_state,
        mut save_in_progress,
        mut show_map_picker,
        mut map_picker_error,
        mut toast,
        navigator,
        opened_as_new,
    } = ctx;
    if *save_in_progress.peek() {
        return;
    }

    // Snapshot current state for the async save
    let snapshot = strategy_state.read().clone();
    let current_map = canvas_state.read().current_map.clone();
    let sub_map_id = canvas_state.read().selected_sub_map.clone();

    let create_map_id = if snapshot.strategy_id.is_none() {
        match map_id_for_create(current_map.as_deref()) {
            Ok(id) => Some(id),
            Err(msg) => {
                map_picker_error.set(Some(msg.to_string()));
                show_map_picker.set(true);
                toast.show(Toast::error(msg));
                return;
            }
        }
    } else {
        None
    };

    save_in_progress.set(true);

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
            client
                .put_json::<_, Strategy>(&format!("/api/strategy/strategies/{id}"), &body)
                .await
        } else {
            let Some(map_id) = create_map_id else {
                save_in_progress.set(false);
                return;
            };
            let game_mode = game_mode_api_str(game_mode_for_map(&map_id)).to_string();
            let body = CreateStrategyRequest {
                name: snapshot.name.clone(),
                description: snapshot.description.clone(),
                map_id,
                sub_map_id,
                game_mode,
                team_id: None,
                visibility: visibility_to_str(snapshot.visibility).to_string(),
                elements: snapshot.elements.clone(),
                phases: snapshot.phases.clone(),
            };
            client
                .post_json::<_, Strategy>("/api/strategy/strategies", &body)
                .await
        };

        match result {
            Ok(saved) => {
                let id = saved.id.clone();
                let map_saved = !saved.map_id.trim().is_empty();
                let clean = strategy_state.with_mut(|s| {
                    s.map_saved = map_saved;
                    s.mark_saved(&snapshot, saved.id, saved.owner_id)
                });
                tracing::info!("Strategy saved successfully");
                toast.show(Toast::success("Strategy saved."));
                if opened_as_new && clean {
                    // Leave `/strategy/editor` so a reload or a second save
                    // works on this strategy instead of creating another.
                    // The new route's editor loads from the server, which
                    // has no team, so the team goes across by hand.
                    strategy_state.peek().hand_off_team();
                    navigator.replace(crate::routes::Route::StrategyEditor { id });
                }
            }
            Err(e) => {
                tracing::error!("Failed to save strategy: {e}");
                toast.show(Toast::error_from("Failed to save strategy", &e));
            }
        }
        save_in_progress.set(false);
    });
}

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
        UndoableAction::AssignHeroToSlot { slot, previous, .. } => {
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
            strategy_state.with_mut(|s| s.set_team_composition(previous));
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
        UndoableAction::AssignHeroToSlot { slot, hero_id, .. } => {
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
            strategy_state.with_mut(|s| s.set_team_composition(Vec::new()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review 819: leaving the editor with an unsaved drawing asks first.
    ///
    /// The guard is a no-op off wasm, so this test cannot run it: the
    /// wiring rests on these source pins alone. How `mark_saved` clears the
    /// flag is covered in `state/editor.rs`; the prompt itself needs a
    /// browser pass (see the PR's QA steps). The pins: one guard call, in
    /// `EditorLayout`, ahead of anything that could leave the component; it
    /// reads the same `has_unsaved` as the toolbar and the "Unsaved
    /// changes" line; the save path still calls `mark_saved` and only
    /// replaces the route when that save left the editor clean.
    #[test]
    fn the_leave_guard_follows_the_unsaved_drawing() {
        let src = include_str!("editor.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let flag = "let has_unsaved = strategy_state.read().has_unsaved_changes;";
        let guard = "use_unsaved_guard(has_unsaved);";
        assert_eq!(prod.matches(guard).count(), 1, "one guard, on the page");
        let layout = prod
            .split("fn EditorLayout(")
            .nth(1)
            .expect("EditorLayout is in this file");
        let before_guard = layout.split(guard).next().unwrap();
        assert!(
            before_guard.len() < layout.len(),
            "EditorLayout owns the guard"
        );
        assert!(
            !before_guard.contains("return") && !before_guard.contains('?'),
            "no way out of the component ahead of the guard"
        );
        // One binding, read just before the guard, reused below.
        assert_eq!(prod.matches("let has_unsaved =").count(), 1);
        assert!(
            before_guard.trim_end().ends_with(flag),
            "the guard reads the flag bound right above it"
        );
        assert!(prod.contains("has_unsaved_changes: has_unsaved,"));
        assert!(prod.contains("} else if has_unsaved {"));
        assert!(prod.contains("s.mark_saved(&snapshot, saved.id, saved.owner_id)"));
        // A first save only moves to the new route when nothing was edited
        // during it; a replace skips the guard and would drop that edit.
        assert!(prod.contains("if opened_as_new && clean {"));
    }

    /// Vision blocking 2: Esc closes the picker, Tab stays inside it, and
    /// no other key (Space, Delete, a tool letter) reaches the editor.
    /// Vision nit 14: Ctrl+S and Cmd+S are held back from the browser.
    #[test]
    fn picker_keys_close_on_escape_trap_tab_and_block_save() {
        use keybindings::Keybinding;
        assert_eq!(
            picker_key(&Keybinding::new("Escape")),
            Some(PickerKey::Close)
        );
        assert_eq!(
            picker_key(&Keybinding::new("Tab")),
            Some(PickerKey::KeepFocusInside)
        );
        assert_eq!(
            picker_key(&Keybinding::new("Tab").with_shift()),
            Some(PickerKey::KeepFocusInside)
        );
        for save in [
            Keybinding::new("s").with_ctrl(),
            Keybinding::new("S").with_ctrl().with_shift(),
            Keybinding::new("s").with_meta(),
        ] {
            assert_eq!(
                picker_key(&save),
                Some(PickerKey::BlockBrowserSave),
                "{save:?}"
            );
        }
        for key in [" ", "Enter", "Delete", "v", "s", "ArrowLeft", "]"] {
            assert_eq!(picker_key(&Keybinding::new(key)), None, "{key:?}");
        }
    }

    /// Vision nit 17: the current map is found however it was stored (slug,
    /// display name, extra spaces), so it is marked and takes focus; an
    /// unknown id marks nothing and focus falls back to Close.
    #[test]
    fn the_picker_finds_the_current_map_however_it_was_stored() {
        let first = &crate::state::maps::PICKABLE_MAPS[0];
        assert_eq!(picker_current_id(Some(first.id)), Some(first.id));
        assert_eq!(
            picker_current_id(Some(&format!("  {}  ", first.id))),
            Some(first.id)
        );
        for map in crate::state::maps::PICKABLE_MAPS {
            assert_eq!(
                picker_current_id(Some(map.name)),
                Some(map.id),
                "{} finds its map by display name",
                map.name
            );
        }
        assert_eq!(picker_current_id(Some("not-a-map")), None);
        assert_eq!(picker_current_id(Some("")), None);
        assert_eq!(picker_current_id(None), None);
    }

    /// Vision nit 4: the panel buttons are named by what they do, not "<".
    #[test]
    fn panel_buttons_have_real_names() {
        assert_eq!(panel_button_label("team", true), "Hide team panel");
        assert_eq!(panel_button_label("team", false), "Show team panel");
        assert_eq!(
            panel_button_label("properties", false),
            "Show properties panel"
        );
        assert_eq!(behind_picker(true), Some("true"));
        assert_eq!(behind_picker(false), None);
    }

    /// This file without its tests, so a pin cannot match itself.
    fn editor_source() -> &'static str {
        let src = include_str!("editor.rs");
        src.split("#[cfg(test)]").next().unwrap_or(src)
    }

    /// Issue 817: the update body is exactly what the server's update
    /// handler reads. When team format, composition or map join it, they
    /// must also join `StrategyState::mark_saved` and the stopgap (team
    /// edits not dirty, map fixed after the first save) can go.
    #[test]
    fn the_update_body_sends_only_what_the_api_stores() {
        let body = UpdateStrategyRequest {
            name: Some("n".into()),
            description: Some(None),
            visibility: Some("private".into()),
            elements: Some(Vec::new()),
            phases: Some(Vec::new()),
        };
        let json = serde_json::to_value(&body).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["description", "elements", "name", "phases", "visibility"]
        );
    }

    /// Issue 817, the other half: the server's update body. If the server
    /// starts reading a map or a team field, this fails along with the
    /// stopgap it no longer needs.
    #[test]
    fn the_server_update_body_reads_only_those_five_fields() {
        let route = include_str!("../../../../server/src/routes/strategy.rs");
        let body = route
            .split("struct UpdateStrategyRequest {")
            .nth(1)
            .expect("server UpdateStrategyRequest");
        let body = &body[..body.find('}').expect("end of UpdateStrategyRequest")];
        let mut fields: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#') && !l.starts_with("//") && l.contains(':'))
            .map(|l| l.split(':').next().unwrap_or("").trim())
            .collect();
        fields.sort();
        assert_eq!(
            fields,
            ["description", "elements", "name", "phases", "visibility"]
        );
    }

    /// The source between `start` and the next `end` after it.
    fn slice<'a>(src: &'a str, start: &str, end: &str) -> &'a str {
        let from = src.find(start).unwrap_or_else(|| panic!("`{start}`"));
        let rest = &src[from..];
        let to = rest
            .find(end)
            .unwrap_or_else(|| panic!("`{end}` after `{start}`"));
        &rest[..to]
    }

    /// Issue 817: nothing in the editor page marks the strategy unsaved for
    /// a team edit. Team state changes only through the `StrategyState`
    /// methods, which leave the flag alone, and no team handler or team
    /// undo or redo arm sets the flag itself.
    #[test]
    fn the_editor_page_never_writes_team_state_directly() {
        let src = editor_source();
        // Any binding: `s.`, `strategy_state.write().`, and so on. A
        // comparison (`==`) is a read, not a write.
        for field in [".team_format =", ".team_composition ="] {
            for (at, _) in src.match_indices(field) {
                assert!(
                    src[at + field.len()..].starts_with('='),
                    "editor.rs writes `{field}` directly"
                );
            }
        }
        for direct in [
            "team_composition.clear()",
            "team_composition.push(",
            "team_composition.retain(",
            "team_composition.insert(",
            "team_composition.remove(",
            "team_composition.truncate(",
        ] {
            assert!(!src.contains(direct), "editor.rs still has `{direct}`");
        }
        let panel = slice(src, "TeamPanel {", "HeroPicker {");
        for handler in [
            "on_format_change:",
            "on_assign:",
            "on_clear_slot:",
            "on_clear_all:",
        ] {
            assert!(panel.contains(handler), "TeamPanel has no `{handler}`");
        }
        assert!(
            !panel.contains("has_unsaved_changes"),
            "a team handler marks the strategy unsaved"
        );
        let undo = slice(src, "fn apply_undo(", "fn apply_redo(");
        let redo = slice(src, "fn apply_redo(", "\n}\n");
        for (name, body) in [("undo", undo), ("redo", redo)] {
            let arms = &body[body
                .find("UndoableAction::AssignHeroToSlot")
                .unwrap_or_else(|| panic!("{name} has no team arms"))..];
            assert!(arms.contains("UndoableAction::ClearSlot"));
            assert!(arms.contains("UndoableAction::ClearAllSlots"));
            assert!(
                !arms.contains("has_unsaved_changes"),
                "a team {name} arm marks the strategy unsaved"
            );
        }
    }

    /// Issue 817: Change map is offered only while the map can still be
    /// sent, the picker re-checks before it switches and marks the strategy
    /// unsaved only before the first save, and the fixed map is focusable
    /// text with the same id, so focus can return to it.
    #[test]
    fn change_map_is_offered_only_while_the_map_can_be_saved() {
        let src = editor_source();
        let bar = slice(src, "if can_change_map {", "span { class: \"status-item\",");
        let (offered, fixed) = bar.split_once("} else {").expect("fixed branch");
        assert!(offered.contains("title: \"Change map\""));
        assert!(offered.contains("onclick: move |_| show_map_picker.set(true),"));
        assert!(offered.contains("\"{map_label}\""));
        assert!(!fixed.contains("onclick"), "the fixed map is not a control");
        assert!(!fixed.contains("button {"));
        assert!(fixed.contains("span {"));
        assert!(fixed.contains("id: EDITOR_MAP_BUTTON_ID,"));
        assert!(fixed.contains("tabindex: \"-1\","));
        assert!(fixed.contains("\"{map_label}\""));
        assert!(fixed.contains("status-map"));
        assert!(offered.contains("status-map"));

        let pick = slice(src, "let allowed = map_change_allowed(", "close_picker();");
        assert!(pick.contains("map_is_kept("));
        let (_, guarded) = pick.split_once("if allowed {").expect("picker re-check");
        assert!(guarded.contains("c.select_map(map_id.to_string(), None);"));
        let (_, dirty) = guarded
            .split_once("if !saved {")
            .expect("unsaved only before save");
        assert!(dirty.contains("s.has_unsaved_changes = true;"));
        assert!(
            !guarded
                .split_once("if !saved {")
                .unwrap()
                .0
                .contains("has_unsaved_changes"),
            "the pick marks a saved strategy unsaved"
        );
        assert!(src.contains("map_is_kept(saved, ss.map_saved, cs.current_map.is_some())"));
    }

    /// Issue 817: the note beside the map name says why it is fixed, or that
    /// a map picked for a saved strategy with none stored is not kept.
    #[test]
    fn the_map_note_matches_what_is_kept() {
        assert_eq!(map_status_note(true, false, true), Some(MAP_FIXED_NOTE));
        assert_eq!(map_status_note(false, false, true), Some(MAP_FIXED_NOTE));
        // Saved, no stored map, one picked: shown but never sent.
        assert_eq!(map_status_note(true, true, true), Some(MAP_NOT_SAVED_NOTE));
        // Saved, nothing picked yet, or a new strategy: no note.
        assert_eq!(map_status_note(true, true, false), None);
        assert_eq!(map_status_note(false, true, true), None);
        assert_eq!(map_status_note(false, true, false), None);
        assert_eq!(MAP_FIXED_NOTE, "fixed once saved");
        assert_eq!(MAP_NOT_SAVED_NOTE, "not saved");
    }

    /// Issue 817: the first save of a new strategy moves to the strategy's
    /// own route, whose editor loads from the server (no team). The team is
    /// handed across right before the move and taken when the editor mounts,
    /// and the saved map state comes from the server's answer.
    #[test]
    fn the_team_survives_the_move_after_the_first_save() {
        let src = editor_source();
        let saved = slice(src, "Ok(saved) => {", "Err(e) => {");
        assert!(saved.contains("s.map_saved = map_saved;"));
        // Only the move hands the team across: outside this block every save
        // would stash it, and reopening the strategy later would bring it back.
        let (before, moving) = saved
            .split_once("if opened_as_new && clean {")
            .expect("first-save block");
        assert!(
            !before.contains("hand_off_team()"),
            "hand-off outside the move"
        );
        let moving = &moving[..moving.find("\n                }\n").expect("block end")];
        assert_eq!(saved.matches("hand_off_team()").count(), 1);
        let hand_off = moving
            .find("hand_off_team();")
            .expect("hand-off in the block");
        let replace = moving
            .find("navigator.replace(crate::routes::Route::StrategyEditor { id });")
            .expect("replace");
        assert!(hand_off < replace);
        let init = slice(
            src,
            "let mut strategy_state = use_signal(",
            "let mut undo_manager",
        );
        assert!(init.contains("state.take_team_hand_off();"));
    }
}
