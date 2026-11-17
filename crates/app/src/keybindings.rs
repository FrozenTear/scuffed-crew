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
//!   Tab toggles the left panel there. Esc cancels a drawing or selection in
//!   progress, and with nothing in progress Esc (or Shift+Tab) leaves.
//!
//! Where a key lands decides whether a shortcut fires ([`KeyTarget`],
//! [`KeybindingManager::action_for_target`]): every shortcut works in the
//! map canvas and on non-interactive parts of the page, none works in a text
//! field, and on a button, link or other control only the Ctrl/Cmd chords
//! (undo, redo, save) work. Space on a focused button only presses it.

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

/// Where a keydown event's target sits, for shortcut routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTarget {
    /// The map canvas (the overlay canvas element) holds focus.
    MapCanvas,
    /// A text field: `input`, `textarea`, `select` or contenteditable.
    TextEntry,
    /// A button, link or other control with keys of its own (see
    /// [`CONTROL_SELECTOR`]).
    Control,
    /// Anything else: the body, `main`, a panel's static content.
    Page,
}

/// Elements whose own keys (Space, Enter, arrows, Home, End, Delete) must
/// reach them: a single-key shortcut never fires on one of these.
pub const CONTROL_SELECTOR: &str = "a[href], button, summary, details, label, \
    audio[controls], video[controls], [role=button], [role=link], [role=tab], \
    [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio], \
    [role=checkbox], [role=radio], [role=switch], [role=option], [role=slider], \
    [role=spinbutton], [role=combobox], [role=listbox], [role=gridcell], \
    [role=treeitem], [role=scrollbar]";

/// Text fields: every key belongs to them.
pub const TEXT_ENTRY_SELECTOR: &str = "input, textarea, select, [contenteditable]:not([contenteditable=false]), \
    [role=textbox], [role=searchbox]";

/// The element that takes the canvas keys.
pub const MAP_CANVAS_SELECTOR: &str = ".map-canvas-overlay";

/// Classify a keydown target. Text fields win over everything (a field
/// inside a control is still a field); then the canvas; then controls.
pub fn key_target(target: Option<&web_sys::Element>) -> KeyTarget {
    let Some(target) = target else {
        return KeyTarget::Page;
    };
    let within = |selector: &str| target.closest(selector).ok().flatten().is_some();
    if within(TEXT_ENTRY_SELECTOR) {
        KeyTarget::TextEntry
    } else if within(MAP_CANVAS_SELECTOR) {
        KeyTarget::MapCanvas
    } else if within(CONTROL_SELECTOR) {
        KeyTarget::Control
    } else {
        KeyTarget::Page
    }
}

/// Whether an editor shortcut may fire on this target. Ctrl/Cmd chords
/// (undo, redo, save) do nothing on a control, so they stay available after
/// a toolbar click; a single key belongs to the focused control.
pub fn shortcut_allowed(target: KeyTarget, binding: &Keybinding) -> bool {
    match target {
        KeyTarget::MapCanvas | KeyTarget::Page => true,
        KeyTarget::TextEntry => false,
        KeyTarget::Control => binding.modifiers.ctrl || binding.modifiers.meta,
    }
}

/// What a key does while keyboard focus is inside the map canvas. Nothing
/// else in the editor may take Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasKey {
    /// Tab: toggle the left panel. Focus stays on the canvas.
    ToggleLeftPanel,
    /// Esc with a drawing or selection in progress: drop it. Focus stays,
    /// so a second Esc leaves.
    CancelInProgress,
    /// Esc with nothing in progress: the editor moves focus out of the
    /// canvas, to the main landmark.
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
/// `in_progress`: a half-drawn shape or a selected element. Esc drops that
/// first; only an Esc with nothing to cancel leaves the canvas.
pub fn canvas_key(binding: &Keybinding, in_progress: bool) -> Option<CanvasKey> {
    let held = binding.modifiers;
    match binding.key.as_str() {
        "tab" if held == Modifiers::none() => Some(CanvasKey::ToggleLeftPanel),
        "tab" if held == Modifiers::shift() => Some(CanvasKey::LeaveBackwards),
        "escape" if held == Modifiers::none() && in_progress => Some(CanvasKey::CancelInProgress),
        "escape" if held == Modifiers::none() => Some(CanvasKey::LeaveToShell),
        _ => None,
    }
}

/// The map's live hint, kept mounted so a screen reader hears each change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CanvasHint {
    /// Focus is outside the map: no text.
    #[default]
    Off,
    /// The map took focus.
    Focused,
    /// Tab toggled the team panel; `true` when it is now shown.
    PanelToggled(bool),
}

impl CanvasHint {
    pub fn text(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Focused => "Tab toggles the team panel. Esc or Shift+Tab leaves the map.",
            Self::PanelToggled(true) => "Team panel shown. Esc or Shift+Tab leaves the map.",
            Self::PanelToggled(false) => "Team panel hidden. Esc or Shift+Tab leaves the map.",
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

    /// The editor-wide action for a key that landed on `target`, or `None`
    /// when the target keeps the key (see [`shortcut_allowed`]).
    pub fn action_for_target(
        &self,
        binding: &Keybinding,
        target: KeyTarget,
    ) -> Option<EditorAction> {
        if !shortcut_allowed(target, binding) {
            return None;
        }
        self.action_for_binding(binding)
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

/// Resolve a document keydown to an editor action, honouring where it
/// landed: nothing on a text field, only Ctrl/Cmd chords on a control.
pub fn from_keyboard_event(event: &web_sys::KeyboardEvent) -> Option<EditorAction> {
    use wasm_bindgen::JsCast;
    let target = event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
    let binding = Keybinding::from_keyboard_event(event);
    with_keybindings(|kb| kb.action_for_target(&binding, key_target(target.as_ref())))
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
        for in_progress in [false, true] {
            let key = canvas_key(&Keybinding::from_key(&Key::Tab, Held::empty()), in_progress);
            assert_eq!(key, Some(CanvasKey::ToggleLeftPanel));
        }
        // Focus stays on the canvas, so the key is swallowed.
        assert!(CanvasKey::ToggleLeftPanel.prevents_default());
    }

    #[test]
    fn escape_inside_the_canvas_exits() {
        let key = canvas_key(&Keybinding::from_key(&Key::Escape, Held::empty()), false);
        assert_eq!(key, Some(CanvasKey::LeaveToShell));
        assert!(!CanvasKey::CancelInProgress.prevents_default());
    }

    #[test]
    fn shift_tab_inside_the_canvas_exits() {
        let key = canvas_key(&Keybinding::from_key(&Key::Tab, Held::SHIFT), true);
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
            assert_eq!(canvas_key(&tab, false), expected, "{tab:?}");
            assert_eq!(canvas_key(&tab, true), expected, "{tab:?}");

            let escape = Keybinding {
                key: "escape".to_string(),
                modifiers,
            };
            let plain = modifiers == Modifiers::none();
            assert_eq!(
                canvas_key(&escape, false),
                plain.then_some(CanvasKey::LeaveToShell),
                "{escape:?}"
            );
            assert_eq!(
                canvas_key(&escape, true),
                plain.then_some(CanvasKey::CancelInProgress),
                "{escape:?}"
            );
        }
        for key in ["v", " ", "[", "]", "delete", "arrowleft", "s"] {
            assert_eq!(canvas_key(&Keybinding::new(key), false), None, "{key}");
            assert_eq!(canvas_key(&Keybinding::new(key), true), None, "{key}");
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
                .find("keybindings::canvas_key(&binding, in_progress)")
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
            editor.contains("p { class: \"canvas-key-hint\", role: \"status\", \"{hint_text}\" }")
        );
        assert_eq!(
            CanvasHint::Focused.text(),
            "Tab toggles the team panel. Esc or Shift+Tab leaves the map."
        );
    }

    /// Every default binding as (binding, action), so the routing tests walk
    /// all 38 of them.
    fn all_defaults() -> Vec<(Keybinding, EditorAction)> {
        let manager = KeybindingManager::new();
        let mut all: Vec<_> = manager
            .bindings
            .iter()
            .map(|(binding, action)| (binding.clone(), *action))
            .collect();
        all.sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
        assert_eq!(all.len(), 38);
        all
    }

    fn is_chord(binding: &Keybinding) -> bool {
        binding.modifiers.ctrl || binding.modifiers.meta
    }

    /// Vision blocking 1 on PR 693: a single-key shortcut never fires on a
    /// focused button or link (or any other control). Space only presses the
    /// button; Delete, arrows, Home and End stay with the control.
    #[test]
    fn single_key_shortcuts_are_ignored_on_a_focused_button_or_link() {
        let manager = KeybindingManager::new();
        let mut single = 0;
        for (binding, action) in all_defaults() {
            if is_chord(&binding) {
                continue;
            }
            single += 1;
            assert_eq!(
                manager.action_for_target(&binding, KeyTarget::Control),
                None,
                "{binding:?} ({action:?}) fired on a control"
            );
        }
        assert_eq!(single, 34, "every single-key binding was checked");
        for key in [
            " ",
            "delete",
            "backspace",
            "arrowleft",
            "arrowright",
            "home",
            "end",
            "k",
        ] {
            assert_eq!(
                manager.action_for_target(&Keybinding::new(key), KeyTarget::Control),
                None,
                "{key:?}"
            );
        }
    }

    /// The same shortcuts all work while the map canvas holds focus, and on
    /// the non-interactive page (body, main, panel text).
    #[test]
    fn every_shortcut_is_active_in_the_canvas_and_on_the_page() {
        let manager = KeybindingManager::new();
        for (binding, action) in all_defaults() {
            for target in [KeyTarget::MapCanvas, KeyTarget::Page] {
                assert_eq!(
                    manager.action_for_target(&binding, target),
                    Some(action),
                    "{binding:?} on {target:?}"
                );
            }
        }
    }

    /// Ctrl/Cmd chords (undo, redo, save) mean nothing to a button or link,
    /// so they keep working after a toolbar click. A text field keeps every
    /// key, chords too (its own undo).
    #[test]
    fn chords_work_on_controls_and_nothing_works_in_a_text_field() {
        let manager = KeybindingManager::new();
        let mut chords = 0;
        for (binding, action) in all_defaults() {
            assert_eq!(
                manager.action_for_target(&binding, KeyTarget::TextEntry),
                None,
                "{binding:?} in a text field"
            );
            if is_chord(&binding) {
                chords += 1;
                assert_eq!(
                    manager.action_for_target(&binding, KeyTarget::Control),
                    Some(action),
                    "{binding:?} on a control"
                );
            }
        }
        assert_eq!(chords, 4);
        let save_cmd = Keybinding::new("s").with_meta();
        assert!(shortcut_allowed(KeyTarget::Control, &save_cmd));
        // Tab is never an editor shortcut, wherever it lands.
        for target in [KeyTarget::MapCanvas, KeyTarget::Page, KeyTarget::Control] {
            assert_eq!(
                manager.action_for_target(&Keybinding::new("tab"), target),
                None
            );
        }
    }

    /// The selectors name the controls Vision listed, and the classifier
    /// checks text fields first, then the canvas, then controls.
    #[test]
    fn the_target_selectors_cover_buttons_links_and_fields() {
        for part in [
            "a[href]",
            "button",
            "summary",
            "[role=button]",
            "[role=link]",
            "[role=tab]",
            "[role=menuitem]",
            "[role=checkbox]",
            "[role=option]",
        ] {
            assert!(CONTROL_SELECTOR.contains(part), "{part}");
        }
        for part in ["input", "textarea", "select", "[contenteditable]"] {
            assert!(TEXT_ENTRY_SELECTOR.contains(part), "{part}");
        }
        assert!(!CONTROL_SELECTOR.contains(MAP_CANVAS_SELECTOR));
        let src = include_str!("keybindings.rs");
        let body = &src[src.find("pub fn key_target(").expect("classifier")..];
        let text = body.find("within(TEXT_ENTRY_SELECTOR)").expect("text");
        let canvas = body.find("within(MAP_CANVAS_SELECTOR)").expect("canvas");
        let control = body.find("within(CONTROL_SELECTOR)").expect("control");
        assert!(text < canvas && canvas < control);
        // The document listener resolves through the target-aware lookup.
        let resolve = &src[src.find("pub fn from_keyboard_event(").expect("resolver")..];
        assert!(resolve.contains("kb.action_for_target(&binding, key_target(target.as_ref()))"));
    }

    /// Vision notes 5 and 10: Esc in the canvas cancels first, then leaves.
    #[test]
    fn escape_cancels_what_is_in_progress_before_leaving() {
        let esc = Keybinding::from_key(&Key::Escape, Held::empty());
        assert_eq!(canvas_key(&esc, true), Some(CanvasKey::CancelInProgress));
        // After the cancel nothing is in progress: the second Esc leaves.
        assert_eq!(canvas_key(&esc, false), Some(CanvasKey::LeaveToShell));
        // Shift+Tab leaves at once, even mid-drawing.
        let back = Keybinding::from_key(&Key::Tab, Held::SHIFT);
        assert_eq!(canvas_key(&back, true), Some(CanvasKey::LeaveBackwards));
    }

    #[test]
    fn the_hint_text_follows_the_panel() {
        assert_eq!(CanvasHint::default(), CanvasHint::Off);
        assert_eq!(CanvasHint::Off.text(), "");
        assert!(
            CanvasHint::PanelToggled(true)
                .text()
                .starts_with("Team panel shown.")
        );
        assert!(
            CanvasHint::PanelToggled(false)
                .text()
                .starts_with("Team panel hidden.")
        );
        for hint in [
            CanvasHint::Focused,
            CanvasHint::PanelToggled(true),
            CanvasHint::PanelToggled(false),
        ] {
            assert!(
                hint.text().ends_with("Esc or Shift+Tab leaves the map."),
                "{hint:?}"
            );
        }
    }

    /// The web canvas takes focus by click, not by Tab. As a Tab stop it
    /// would swallow every further Tab and cut off the controls after it.
    #[test]
    fn the_map_canvas_is_not_a_tab_stop() {
        // Vision nit 5: the desktop canvas follows the same rule as web.
        for canvas in [
            include_str!("components/strategy/map_canvas.rs"),
            include_str!("components/strategy/desktop_map_canvas.rs"),
        ] {
            let production = canvas.split("mod tests").next().expect("production half");
            assert_eq!(production.matches("tabindex:").count(), 1);
            assert!(production.contains("tabindex: \"-1\","));
            // Its Esc goes through the editor's `focus_element`, which works
            // in the desktop webview too (`layouts::focus_script`).
            assert!(production.contains("on_canvas_key.call(canvas_key);"));
            assert!(production.contains("Esc or Shift+Tab leaves the map."));
        }
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
