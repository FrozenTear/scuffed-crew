//! Configurable keybinding system for the strategy editor.
//!
//! Maps keyboard events to `EditorAction`s. Provides a thread-local global
//! instance via `with_keybindings` so any component can resolve a
//! `web_sys::KeyboardEvent` to an action without threading props.

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
            (Keybinding::new("tab"), EditorAction::ToggleLeftPanel),
            (
                Keybinding::new("tab").with_shift(),
                EditorAction::ToggleRightPanel,
            ),
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
        let binding = Keybinding::from_keyboard_event(event);
        self.bindings.get(&binding).copied()
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
