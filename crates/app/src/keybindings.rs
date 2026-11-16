//! Configurable keybinding system for the strategy editor.
//!
//! Maps keyboard events to `EditorAction`s. Provides a thread-local global
//! instance via `with_keybindings` so any component can resolve a
//! `web_sys::KeyboardEvent` to an action without threading props.
//!
//! Two layers:
//! - [`KeybindingManager`]: editor-wide shortcuts, resolved by the editor's
//!   document keydown listener. Tab is never one of them, so Tab moves focus
//!   as on every other page (skip link, site nav, the editor's own buttons).
//! - [`canvas_key`]: keys that only act while focus is inside the map canvas.
//!   Tab toggles the left panel there. Esc and Shift+Tab leave the canvas.

use std::collections::HashMap;

/// Actions that can be triggered by keybindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditorAction {
    // Tool selection
    SelectTool,
    PanTool,
    MarkerTool,
    RouteTool,
    AreaTool,
    ArrowTool,
    TextTool,
    EraserTool,

    // Panel toggles
    ToggleLeftPanel,
    ToggleRightPanel,
    ToggleBottomPanel,
    ToggleHealthPacks,

    // Element actions
    Delete,

    // Zoom
    ZoomIn,
    ZoomOut,
    ZoomReset,

    // Playback
    PlayPause,
    NextPhase,
    PrevPhase,
    FirstPhase,
    LastPhase,

    // History
    Undo,
    Redo,

    // File
    Save,
}

/// Modifier key state extracted from a keyboard event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn shift() -> Self {
        Self {
            shift: true,
            ..Default::default()
        }
    }

    pub fn ctrl() -> Self {
        Self {
            ctrl: true,
            ..Default::default()
        }
    }

    pub fn ctrl_shift() -> Self {
        Self {
            ctrl: true,
            shift: true,
            ..Default::default()
        }
    }

    pub fn from_keyboard_event(event: &web_sys::KeyboardEvent) -> Self {
        Self {
            shift: event.shift_key(),
            ctrl: event.ctrl_key(),
            alt: event.alt_key(),
            meta: event.meta_key(),
        }
    }
}

/// A single key + modifiers combination.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Keybinding {
    /// Lowercase key name (e.g. `"v"`, `"escape"`, `"arrowright"`).
    pub key: String,
    pub modifiers: Modifiers,
}

impl Keybinding {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into().to_lowercase(),
            modifiers: Modifiers::none(),
        }
    }

    pub fn with_shift(mut self) -> Self {
        self.modifiers.shift = true;
        self
    }

    pub fn with_ctrl(mut self) -> Self {
        self.modifiers.ctrl = true;
        self
    }

    pub fn with_alt(mut self) -> Self {
        self.modifiers.alt = true;
        self
    }

    pub fn with_meta(mut self) -> Self {
        self.modifiers.meta = true;
        self
    }

    /// Build a `Keybinding` from a browser `KeyboardEvent`.
    pub fn from_keyboard_event(event: &web_sys::KeyboardEvent) -> Self {
        Self {
            key: event.key().to_lowercase(),
            modifiers: Modifiers::from_keyboard_event(event),
        }
    }

    /// Build a `Keybinding` from a Dioxus key event (`evt.key()`,
    /// `evt.modifiers()`). Named keys print as the browser's `key` string
    /// ("Tab", "Escape"), so both constructors agree.
    pub fn from_key(key: &dioxus::prelude::Key, modifiers: dioxus::prelude::Modifiers) -> Self {
        use dioxus::prelude::Modifiers as Held;
        Self {
            key: key.to_string().to_lowercase(),
            modifiers: Modifiers {
                shift: modifiers.contains(Held::SHIFT),
                ctrl: modifiers.contains(Held::CONTROL),
                alt: modifiers.contains(Held::ALT),
                meta: modifiers.contains(Held::META),
            },
        }
    }

    /// Tab with any modifiers. The browser uses it to move focus.
    pub fn is_tab(&self) -> bool {
        self.key == "tab"
    }
}

/// What a key does while keyboard focus is inside the map canvas. Nothing
/// else in the editor may take Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasKey {
    /// Tab: toggle the left panel. Focus stays on the canvas.
    ToggleLeftPanel,
    /// Esc: the editor moves focus out of the canvas, to the main landmark.
    LeaveToShell,
    /// Shift+Tab: the browser moves focus to the previous stop.
    LeaveBackwards,
}

impl CanvasKey {
    /// Only the panel toggle swallows the key. Shift+Tab has to reach the
    /// browser, or focus would never leave the canvas.
    pub fn prevents_default(self) -> bool {
        matches!(self, Self::ToggleLeftPanel)
    }
}

/// Resolve a key pressed while focus is inside the map canvas. `None` means
/// the canvas has no say (Ctrl+Tab, Alt+Tab and every non-Tab key but Esc).
pub fn canvas_key(binding: &Keybinding) -> Option<CanvasKey> {
    let held = binding.modifiers;
    match binding.key.as_str() {
        "tab" if held == Modifiers::none() => Some(CanvasKey::ToggleLeftPanel),
        "tab" if held == Modifiers::shift() => Some(CanvasKey::LeaveBackwards),
        "escape" if held == Modifiers::none() => Some(CanvasKey::LeaveToShell),
        _ => None,
    }
}

/// Maps `Keybinding`s to `EditorAction`s with default and custom bindings.
#[derive(Debug, Clone)]
pub struct KeybindingManager {
    bindings: HashMap<Keybinding, EditorAction>,
    /// Reverse lookup for UI display (e.g. tooltips).
    action_keys: HashMap<EditorAction, Vec<Keybinding>>,
}

impl Default for KeybindingManager {
    fn default() -> Self {
        Self::new()
    }
}

impl KeybindingManager {
    pub fn new() -> Self {
        let mut manager = Self {
            bindings: HashMap::new(),
            action_keys: HashMap::new(),
        };
        manager.load_defaults();
        manager
    }

    /// Load the default keybinding set.
    fn load_defaults(&mut self) {
        let defaults: Vec<(Keybinding, EditorAction)> = vec![
            // ---- Tool selection ----
            (Keybinding::new("v"), EditorAction::SelectTool),
            (Keybinding::new("1"), EditorAction::SelectTool),
            (Keybinding::new("h"), EditorAction::PanTool),
            (Keybinding::new("2"), EditorAction::PanTool),
            (Keybinding::new("m"), EditorAction::MarkerTool),
            (Keybinding::new("3"), EditorAction::MarkerTool),
            (Keybinding::new("r"), EditorAction::RouteTool),
            (Keybinding::new("4"), EditorAction::RouteTool),
            (Keybinding::new("a"), EditorAction::AreaTool),
            (Keybinding::new("5"), EditorAction::AreaTool),
            (Keybinding::new("w"), EditorAction::ArrowTool),
            (Keybinding::new("6"), EditorAction::ArrowTool),
            (Keybinding::new("t"), EditorAction::TextTool),
            (Keybinding::new("7"), EditorAction::TextTool),
            (Keybinding::new("e"), EditorAction::EraserTool),
            (Keybinding::new("8"), EditorAction::EraserTool),
            // ---- Panel toggles ----
            // The left panel is Tab, but only inside the map canvas: see
            // `canvas_key`. Shift+Tab leaves the canvas, so the right panel
            // moved to `]`.
            (Keybinding::new("]"), EditorAction::ToggleRightPanel),
            (Keybinding::new("["), EditorAction::ToggleBottomPanel),
            (Keybinding::new("\\"), EditorAction::ToggleHealthPacks),
            // ---- Element actions ----
            (Keybinding::new("delete"), EditorAction::Delete),
            (Keybinding::new("backspace"), EditorAction::Delete),
            // ---- Zoom ----
            (Keybinding::new("+"), EditorAction::ZoomIn),
            (Keybinding::new("="), EditorAction::ZoomIn),
            (Keybinding::new("-"), EditorAction::ZoomOut),
            (Keybinding::new("_"), EditorAction::ZoomOut),
            (Keybinding::new("0"), EditorAction::ZoomReset),
            // ---- Playback ----
            (Keybinding::new("k"), EditorAction::PlayPause),
            (Keybinding::new(" "), EditorAction::PlayPause),
            (Keybinding::new("l"), EditorAction::NextPhase),
            (Keybinding::new("arrowright"), EditorAction::NextPhase),
            (Keybinding::new("j"), EditorAction::PrevPhase),
            (Keybinding::new("arrowleft"), EditorAction::PrevPhase),
            (Keybinding::new("home"), EditorAction::FirstPhase),
            (Keybinding::new("end"), EditorAction::LastPhase),
            // ---- History ----
            (Keybinding::new("z").with_ctrl(), EditorAction::Undo),
            (
                Keybinding::new("z").with_ctrl().with_shift(),
                EditorAction::Redo,
            ),
            (Keybinding::new("y").with_ctrl(), EditorAction::Redo),
            // ---- File ----
            (Keybinding::new("s").with_ctrl(), EditorAction::Save),
        ];

        for (binding, action) in defaults {
            self.bind(binding, action);
        }
    }

    /// Add or replace a keybinding.
    pub fn bind(&mut self, binding: Keybinding, action: EditorAction) {
        self.bindings.insert(binding.clone(), action);
        self.action_keys.entry(action).or_default().push(binding);
    }

    /// Remove a keybinding.
    pub fn unbind(&mut self, binding: &Keybinding) {
        if let Some(action) = self.bindings.remove(binding)
            && let Some(keys) = self.action_keys.get_mut(&action)
        {
            keys.retain(|k| k != binding);
        }
    }

    /// Look up the action for a raw keyboard event.
    pub fn action_for(&self, event: &web_sys::KeyboardEvent) -> Option<EditorAction> {
        self.action_for_binding(&Keybinding::from_keyboard_event(event))
    }

    /// Look up the editor-wide action for a key. Tab never has one, whatever
    /// was bound: outside the map canvas it belongs to the browser.
    pub fn action_for_binding(&self, binding: &Keybinding) -> Option<EditorAction> {
        if binding.is_tab() {
            return None;
        }
        self.bindings.get(binding).copied()
    }

    /// Human-readable string for the first keybinding of an action.
    pub fn display_for(&self, action: EditorAction) -> Option<String> {
        self.action_keys
            .get(&action)
            .and_then(|keys| keys.first().map(format_keybinding))
    }

    /// All display strings for an action.
    pub fn all_display_for(&self, action: EditorAction) -> Vec<String> {
        self.action_keys
            .get(&action)
            .map(|keys| keys.iter().map(format_keybinding).collect())
            .unwrap_or_default()
    }
}

/// Format a keybinding for display in tooltips / UI.
fn format_keybinding(binding: &Keybinding) -> String {
    let mut parts = Vec::new();

    if binding.modifiers.ctrl {
        parts.push("Ctrl");
    }
    if binding.modifiers.alt {
        parts.push("Alt");
    }
    if binding.modifiers.shift {
        parts.push("Shift");
    }
    if binding.modifiers.meta {
        parts.push("Cmd");
    }

    let key_display = match binding.key.as_str() {
        " " => "Space",
        "tab" => "Tab",
        "escape" => "Esc",
        "delete" => "Del",
        "backspace" => "Backspace",
        "arrowleft" => "\u{2190}",
        "arrowright" => "\u{2192}",
        "arrowup" => "\u{2191}",
        "arrowdown" => "\u{2193}",
        "home" => "Home",
        "end" => "End",
        k => k,
    };

    parts.push(key_display);
    parts.join("+")
}

// ---------------------------------------------------------------------------
// Thread-local global instance (WASM is single-threaded)
// ---------------------------------------------------------------------------

thread_local! {
    static MANAGER: std::cell::RefCell<KeybindingManager> =
        std::cell::RefCell::new(KeybindingManager::new());
}

/// Run a closure with a reference to the global `KeybindingManager`.
pub fn with_keybindings<F, T>(f: F) -> T
where
    F: FnOnce(&KeybindingManager) -> T,
{
    MANAGER.with(|m| f(&m.borrow()))
}

/// Run a closure with a mutable reference to the global `KeybindingManager`.
pub fn with_keybindings_mut<F, T>(f: F) -> T
where
    F: FnOnce(&mut KeybindingManager) -> T,
{
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

/// Convenience: resolve a keyboard event to an action.
pub fn from_keyboard_event(event: &web_sys::KeyboardEvent) -> Option<EditorAction> {
    with_keybindings(|kb| kb.action_for(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::prelude::{Key, Modifiers as Held};

    fn every_modifier_set() -> Vec<Modifiers> {
        (0..16u8)
            .map(|bits| Modifiers {
                shift: bits & 1 != 0,
                ctrl: bits & 2 != 0,
                alt: bits & 4 != 0,
                meta: bits & 8 != 0,
            })
            .collect()
    }

    /// QA blocking 2 on PR 693: on the editor Tab did nothing, so the skip
    /// link and the site nav could not be reached. Outside the canvas no Tab
    /// resolves to an editor action, so nothing calls `prevent_default`.
    #[test]
    fn tab_outside_the_canvas_is_not_consumed() {
        let manager = KeybindingManager::new();
        for modifiers in every_modifier_set() {
            let binding = Keybinding {
                key: "tab".to_string(),
                modifiers,
            };
            assert_eq!(manager.action_for_binding(&binding), None, "{binding:?}");
        }
    }

    #[test]
    fn no_default_binding_uses_tab_or_escape() {
        let manager = KeybindingManager::new();
        for binding in manager.bindings.keys() {
            assert!(!binding.is_tab(), "{binding:?}");
            assert_ne!(binding.key, "escape", "{binding:?}");
        }
        assert_eq!(manager.display_for(EditorAction::ToggleLeftPanel), None);
    }

    /// A custom binding cannot bring the trap back.
    #[test]
    fn a_custom_tab_binding_still_leaves_tab_to_the_browser() {
        let mut manager = KeybindingManager::new();
        manager.bind(Keybinding::new("tab"), EditorAction::ToggleLeftPanel);
        manager.bind(
            Keybinding::new("tab").with_shift(),
            EditorAction::ToggleRightPanel,
        );
        assert_eq!(manager.action_for_binding(&Keybinding::new("tab")), None);
        assert_eq!(
            manager.action_for_binding(&Keybinding::new("tab").with_shift()),
            None
        );
    }

    #[test]
    fn tab_inside_the_canvas_toggles_the_left_panel() {
        let key = canvas_key(&Keybinding::from_key(&Key::Tab, Held::empty()));
        assert_eq!(key, Some(CanvasKey::ToggleLeftPanel));
        // Focus stays on the canvas, so the key is swallowed.
        assert!(CanvasKey::ToggleLeftPanel.prevents_default());
    }

    #[test]
    fn escape_inside_the_canvas_exits() {
        let key = canvas_key(&Keybinding::from_key(&Key::Escape, Held::empty()));
        assert_eq!(key, Some(CanvasKey::LeaveToShell));
    }

    #[test]
    fn shift_tab_inside_the_canvas_exits() {
        let key = canvas_key(&Keybinding::from_key(&Key::Tab, Held::SHIFT));
        assert_eq!(key, Some(CanvasKey::LeaveBackwards));
        // The browser does the move, so the key must reach it.
        assert!(!CanvasKey::LeaveBackwards.prevents_default());
        assert!(!CanvasKey::LeaveToShell.prevents_default());
    }

    #[test]
    fn the_canvas_takes_no_other_tab_or_escape_chord() {
        for modifiers in every_modifier_set() {
            let tab = Keybinding {
                key: "tab".to_string(),
                modifiers,
            };
            let expected = if modifiers == Modifiers::none() {
                Some(CanvasKey::ToggleLeftPanel)
            } else if modifiers == Modifiers::shift() {
                Some(CanvasKey::LeaveBackwards)
            } else {
                None
            };
            assert_eq!(canvas_key(&tab), expected, "{tab:?}");

            let escape = Keybinding {
                key: "escape".to_string(),
                modifiers,
            };
            let expected = (modifiers == Modifiers::none()).then_some(CanvasKey::LeaveToShell);
            assert_eq!(canvas_key(&escape), expected, "{escape:?}");
        }
        for key in ["v", " ", "[", "]", "delete", "arrowleft", "s"] {
            assert_eq!(canvas_key(&Keybinding::new(key)), None, "{key}");
        }
    }

    #[test]
    fn dioxus_keys_match_the_browser_key_names() {
        assert_eq!(
            Keybinding::from_key(&Key::Tab, Held::empty()),
            Keybinding::new("tab")
        );
        assert_eq!(
            Keybinding::from_key(&Key::Tab, Held::SHIFT),
            Keybinding::new("tab").with_shift()
        );
        assert_eq!(
            Keybinding::from_key(&Key::Escape, Held::empty()),
            Keybinding::new("escape")
        );
        assert_eq!(
            Keybinding::from_key(&Key::Character("S".to_string()), Held::CONTROL),
            Keybinding::new("s").with_ctrl()
        );
    }

    /// The Tab handler sits on the canvas element, not on the document, and
    /// the on-screen help says so.
    #[test]
    fn the_editor_scopes_tab_to_the_canvas_element() {
        let editor = include_str!("pages/strategy/editor.rs");
        assert!(
            !editor.contains("canvas_key("),
            "the editor resolves no Tab"
        );
        assert!(editor.contains("on_canvas_key: move |key: CanvasKey| match key {"));
        for canvas in [
            include_str!("components/strategy/map_canvas.rs"),
            include_str!("components/strategy/desktop_map_canvas.rs"),
        ] {
            assert_eq!(canvas.matches("keybindings::canvas_key(").count(), 1);
            // The one keydown handler in the file is the overlay canvas's.
            assert_eq!(canvas.matches("onkeydown:").count(), 1);
            let at = canvas
                .find("class: \"map-canvas map-canvas-overlay\",")
                .expect("overlay canvas");
            let element = &canvas[at..];
            let handler = element.find("onkeydown:").expect("canvas keydown");
            let resolve = element
                .find("keybindings::canvas_key(&binding)")
                .expect("canvas key routing");
            let closes = element
                .find("\n            }\n")
                .expect("end of the canvas");
            assert!(handler < resolve && resolve < closes);
        }

        assert!(editor.contains("title: \"Toggle panel (Tab while the map has focus)\""));
        assert!(editor.contains("title: \"Toggle panel (])\""));
        assert!(!editor.contains("(Shift+Tab)"));
        assert!(!editor.contains("title: \"Toggle panel (Tab)\""));
        assert!(
            editor.contains("\"Tab toggles the team panel. Esc or Shift+Tab leaves the map.\"")
        );
    }

    /// The web canvas takes focus by click, not by Tab. As a Tab stop it
    /// would swallow every further Tab and cut off the controls after it.
    #[test]
    fn the_map_canvas_is_not_a_tab_stop() {
        let canvas = include_str!("components/strategy/map_canvas.rs");
        let production = canvas.split("mod tests").next().expect("production half");
        assert_eq!(production.matches("tabindex:").count(), 1);
        assert!(production.contains("tabindex: \"-1\","));
    }

    /// The other editor shortcuts are where they were. The right panel moved
    /// from Shift+Tab to `]`.
    #[test]
    fn the_other_shortcuts_are_unchanged() {
        let manager = KeybindingManager::new();
        let plain = |key: &str| manager.action_for_binding(&Keybinding::new(key));
        for (key, action) in [
            ("v", EditorAction::SelectTool),
            ("1", EditorAction::SelectTool),
            ("h", EditorAction::PanTool),
            ("2", EditorAction::PanTool),
            ("m", EditorAction::MarkerTool),
            ("3", EditorAction::MarkerTool),
            ("r", EditorAction::RouteTool),
            ("4", EditorAction::RouteTool),
            ("a", EditorAction::AreaTool),
            ("5", EditorAction::AreaTool),
            ("w", EditorAction::ArrowTool),
            ("6", EditorAction::ArrowTool),
            ("t", EditorAction::TextTool),
            ("7", EditorAction::TextTool),
            ("e", EditorAction::EraserTool),
            ("8", EditorAction::EraserTool),
            ("]", EditorAction::ToggleRightPanel),
            ("[", EditorAction::ToggleBottomPanel),
            ("\\", EditorAction::ToggleHealthPacks),
            ("delete", EditorAction::Delete),
            ("backspace", EditorAction::Delete),
            ("+", EditorAction::ZoomIn),
            ("=", EditorAction::ZoomIn),
            ("-", EditorAction::ZoomOut),
            ("_", EditorAction::ZoomOut),
            ("0", EditorAction::ZoomReset),
            ("k", EditorAction::PlayPause),
            (" ", EditorAction::PlayPause),
            ("l", EditorAction::NextPhase),
            ("arrowright", EditorAction::NextPhase),
            ("j", EditorAction::PrevPhase),
            ("arrowleft", EditorAction::PrevPhase),
            ("home", EditorAction::FirstPhase),
            ("end", EditorAction::LastPhase),
        ] {
            assert_eq!(plain(key), Some(action), "{key}");
        }
        let chord = |binding: Keybinding| manager.action_for_binding(&binding);
        assert_eq!(
            chord(Keybinding::new("z").with_ctrl()),
            Some(EditorAction::Undo)
        );
        assert_eq!(
            chord(Keybinding::new("z").with_ctrl().with_shift()),
            Some(EditorAction::Redo)
        );
        assert_eq!(
            chord(Keybinding::new("y").with_ctrl()),
            Some(EditorAction::Redo)
        );
        assert_eq!(
            chord(Keybinding::new("s").with_ctrl()),
            Some(EditorAction::Save)
        );
        assert_eq!(manager.bindings.len(), 38, "no binding added or dropped");
        assert_eq!(
            manager
                .display_for(EditorAction::ToggleRightPanel)
                .as_deref(),
            Some("]")
        );
    }
}
