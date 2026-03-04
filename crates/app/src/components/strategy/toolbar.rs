use dioxus::prelude::*;
use scuffed_types::{Color, FloorLevel, Tool, Visibility};

const TOOLBAR_CSS: &str = r#"
    .editor-toolbar {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0 0.75rem;
        height: 44px;
        background: var(--bg-surface);
        border-bottom: 1px solid var(--border);
        flex-shrink: 0;
        overflow-x: auto;
    }

    /* ---- Section / dividers ---- */
    .tb-section {
        display: flex;
        align-items: center;
        gap: 0.25rem;
    }
    .tb-divider {
        width: 1px;
        height: 24px;
        background: var(--border);
        margin: 0 0.35rem;
    }
    .tb-label {
        font-size: 0.65rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.06em;
        margin-right: 0.25rem;
        user-select: none;
    }
    .tb-spacer {
        flex: 1;
    }

    /* ---- Tool buttons ---- */
    .tb-tool-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 30px;
        height: 30px;
        border: 1px solid transparent;
        border-radius: 5px;
        background: none;
        color: var(--text-secondary);
        font-size: 0.85rem;
        cursor: pointer;
        transition: background 0.12s, color 0.12s, border-color 0.12s;
    }
    .tb-tool-btn:hover {
        background: var(--bg-card);
        color: var(--text-bright);
    }
    .tb-tool-btn.active {
        background: var(--accent-soft);
        color: var(--accent-bright);
        border-color: var(--accent);
    }
    .tb-tool-btn:disabled {
        opacity: 0.35;
        cursor: default;
    }

    /* ---- Color swatches ---- */
    .tb-color-btn {
        width: 22px;
        height: 22px;
        border: 2px solid transparent;
        border-radius: 50%;
        cursor: pointer;
        transition: border-color 0.12s, transform 0.12s;
    }
    .tb-color-btn:hover {
        transform: scale(1.15);
    }
    .tb-color-btn.active {
        border-color: var(--accent-bright);
        box-shadow: 0 0 0 2px var(--accent-glow);
    }

    /* ---- Opacity slider ---- */
    .tb-opacity {
        display: flex;
        align-items: center;
        gap: 0.35rem;
    }
    .tb-opacity input[type="range"] {
        width: 60px;
        accent-color: var(--accent);
    }
    .tb-opacity-val {
        font-size: 0.7rem;
        color: var(--text-muted);
        min-width: 2.2em;
        text-align: right;
    }

    /* ---- Zoom display ---- */
    .tb-zoom-display {
        font-size: 0.75rem;
        color: var(--text-muted);
        min-width: 3em;
        text-align: center;
        user-select: none;
    }

    /* ---- Floor selector ---- */
    .tb-floor-select {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 4px;
        color: var(--text-secondary);
        font-size: 0.75rem;
        padding: 0.2rem 0.35rem;
    }

    /* ---- Health pack toggle ---- */
    .tb-hp-toggle {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 30px;
        height: 30px;
        border: 1px solid transparent;
        border-radius: 5px;
        background: none;
        color: var(--text-secondary);
        font-size: 0.9rem;
        cursor: pointer;
        transition: background 0.12s, color 0.12s;
    }
    .tb-hp-toggle.active {
        background: rgba(80, 200, 120, 0.15);
        color: #64c878;
    }
    .tb-hp-toggle:hover {
        background: var(--bg-card);
    }

    /* ---- Strategy name input ---- */
    .tb-name-input {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 5px;
        color: var(--text-bright);
        font-size: 0.8rem;
        padding: 0.3rem 0.5rem;
        width: 160px;
        outline: none;
        transition: border-color 0.15s;
    }
    .tb-name-input:focus {
        border-color: var(--accent);
    }

    /* ---- Save button ---- */
    .tb-save-btn {
        display: flex;
        align-items: center;
        gap: 0.35rem;
        padding: 0.3rem 0.75rem;
        border: none;
        border-radius: 5px;
        background: var(--accent);
        color: #fff;
        font-size: 0.8rem;
        font-weight: 600;
        cursor: pointer;
        transition: opacity 0.15s;
        position: relative;
    }
    .tb-save-btn:hover {
        opacity: 0.9;
    }
    .tb-save-btn:disabled {
        opacity: 0.5;
        cursor: default;
    }
    .tb-unsaved-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: #fbbf24;
    }

    /* ---- Visibility dropdown ---- */
    .tb-visibility-select {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 4px;
        color: var(--text-secondary);
        font-size: 0.75rem;
        padding: 0.2rem 0.35rem;
    }
"#;

/// Tool metadata: (Tool variant, tooltip label, display icon).
const TOOLS: [(Tool, &str, &str); 8] = [
    (Tool::Select, "Select (V)", "\u{238b}"),
    (Tool::Pan, "Pan (H)", "\u{270b}"),
    (Tool::PlayerMarker, "Player Marker (M)", "\u{25cf}"),
    (Tool::Route, "Draw Route (R)", "\u{2571}"),
    (Tool::Area, "Draw Area (A)", "\u{25a2}"),
    (Tool::Arrow, "Arrow (W)", "\u{27a4}"),
    (Tool::Text, "Text (T)", "T"),
    (Tool::Eraser, "Eraser (E)", "\u{2715}"),
];

/// Color palette: (Color, label).
const COLORS: [(Color, &str); 7] = [
    (Color::BLUE_TEAM, "Blue Team"),
    (Color::RED_TEAM, "Red Team"),
    (Color::TANK, "Tank"),
    (Color::DAMAGE, "Damage"),
    (Color::SUPPORT, "Support"),
    (Color::WHITE, "White"),
    (Color::BLACK, "Black"),
];

#[component]
pub fn Toolbar(
    // ---- Tool ----
    active_tool: Tool,
    on_tool_change: EventHandler<Tool>,

    // ---- Color ----
    draw_color: Color,
    on_color_change: EventHandler<Color>,

    // ---- Fill ----
    fill_opacity: f32,
    on_fill_opacity_change: EventHandler<f32>,

    // ---- Undo / Redo ----
    can_undo: bool,
    can_redo: bool,
    on_undo: EventHandler<()>,
    on_redo: EventHandler<()>,

    // ---- Zoom ----
    zoom: f64,
    on_zoom_in: EventHandler<()>,
    on_zoom_out: EventHandler<()>,
    on_zoom_reset: EventHandler<()>,

    // ---- Floors ----
    floors: Vec<FloorLevel>,
    selected_floor: Option<String>,
    on_floor_change: EventHandler<String>,

    // ---- Health packs ----
    show_health_packs: bool,
    on_toggle_health_packs: EventHandler<()>,

    // ---- Strategy name ----
    strategy_name: String,
    on_name_change: EventHandler<String>,

    // ---- Save ----
    has_unsaved_changes: bool,
    on_save: EventHandler<()>,
    saving: bool,

    // ---- Visibility ----
    visibility: Visibility,
    on_visibility_change: EventHandler<Visibility>,
) -> Element {
    let opacity_pct = (fill_opacity * 100.0) as i32;
    let zoom_pct = format!("{:.0}%", zoom * 100.0);

    rsx! {
        style { {TOOLBAR_CSS} }
        div { class: "editor-toolbar",

            // ===== Tools =====
            div { class: "tb-section",
                span { class: "tb-label", "Tools" }
                {TOOLS.iter().map(|(tool, tip, icon)| {
                    let tool = *tool;
                    let active_cls = if tool == active_tool { "tb-tool-btn active" } else { "tb-tool-btn" };
                    rsx! {
                        button {
                            class: "{active_cls}",
                            title: "{tip}",
                            onclick: move |_| on_tool_change.call(tool),
                            "{icon}"
                        }
                    }
                })}
            }

            div { class: "tb-divider" }

            // ===== Colors =====
            div { class: "tb-section",
                span { class: "tb-label", "Color" }
                {COLORS.iter().map(|(color, name)| {
                    let color = *color;
                    let css = color.to_css();
                    let active_cls = if color == draw_color { "tb-color-btn active" } else { "tb-color-btn" };
                    rsx! {
                        button {
                            class: "{active_cls}",
                            title: "{name}",
                            style: "background-color: {css};",
                            onclick: move |_| on_color_change.call(color),
                        }
                    }
                })}
            }

            // ===== Fill opacity =====
            div { class: "tb-section",
                span { class: "tb-label", "Fill" }
                div { class: "tb-opacity",
                    input {
                        r#type: "range",
                        min: "0",
                        max: "100",
                        value: "{opacity_pct}",
                        title: "Fill Opacity",
                        oninput: move |e: Event<FormData>| {
                            let v: f32 = e.value().parse().unwrap_or(25.0);
                            on_fill_opacity_change.call(v / 100.0);
                        },
                    }
                    span { class: "tb-opacity-val", "{opacity_pct}%" }
                }
            }

            div { class: "tb-divider" }

            // ===== Undo / Redo =====
            div { class: "tb-section",
                button {
                    class: "tb-tool-btn",
                    title: "Undo (Ctrl+Z)",
                    disabled: !can_undo,
                    onclick: move |_| on_undo.call(()),
                    "\u{21b6}"
                }
                button {
                    class: "tb-tool-btn",
                    title: "Redo (Ctrl+Shift+Z)",
                    disabled: !can_redo,
                    onclick: move |_| on_redo.call(()),
                    "\u{21b7}"
                }
            }

            // ===== Zoom =====
            div { class: "tb-section",
                button {
                    class: "tb-tool-btn",
                    title: "Zoom In",
                    onclick: move |_| on_zoom_in.call(()),
                    "+"
                }
                span { class: "tb-zoom-display", "{zoom_pct}" }
                button {
                    class: "tb-tool-btn",
                    title: "Zoom Out",
                    onclick: move |_| on_zoom_out.call(()),
                    "-"
                }
                button {
                    class: "tb-tool-btn",
                    title: "Reset View",
                    onclick: move |_| on_zoom_reset.call(()),
                    "R"
                }
            }

            // ===== Floor Selector (only when multiple floors) =====
            if floors.len() > 1 {
                div { class: "tb-section",
                    select {
                        class: "tb-floor-select",
                        value: selected_floor.clone().unwrap_or_default(),
                        onchange: move |e: Event<FormData>| {
                            on_floor_change.call(e.value());
                        },
                        {floors.iter().map(|floor| {
                            let fid = floor.id.clone();
                            let fname = floor.name.clone();
                            let is_selected = selected_floor.as_deref() == Some(fid.as_str());
                            rsx! {
                                option {
                                    value: "{fid}",
                                    selected: is_selected,
                                    "{fname}"
                                }
                            }
                        })}
                    }
                }
            }

            // ===== Health Pack Toggle =====
            {
                let hp_cls = if show_health_packs { "tb-hp-toggle active" } else { "tb-hp-toggle" };
                rsx! {
                    button {
                        class: "{hp_cls}",
                        title: "Toggle Health Packs",
                        onclick: move |_| on_toggle_health_packs.call(()),
                        "+"
                    }
                }
            }

            div { class: "tb-spacer" }

            // ===== Strategy Name =====
            div { class: "tb-section",
                input {
                    r#type: "text",
                    class: "tb-name-input",
                    placeholder: "Strategy name...",
                    value: "{strategy_name}",
                    oninput: move |e: Event<FormData>| {
                        on_name_change.call(e.value());
                    },
                }
            }

            // ===== Save =====
            div { class: "tb-section",
                button {
                    class: "tb-save-btn",
                    disabled: saving,
                    onclick: move |_| on_save.call(()),
                    if has_unsaved_changes {
                        span { class: "tb-unsaved-dot" }
                    }
                    if saving {
                        "Saving..."
                    } else {
                        "Save"
                    }
                }
            }

            // ===== Visibility =====
            div { class: "tb-section",
                select {
                    class: "tb-visibility-select",
                    value: match visibility {
                        Visibility::Private => "private",
                        Visibility::Unlisted => "unlisted",
                        Visibility::Public => "public",
                    },
                    onchange: move |e: Event<FormData>| {
                        let vis = match e.value().as_str() {
                            "unlisted" => Visibility::Unlisted,
                            "public" => Visibility::Public,
                            _ => Visibility::Private,
                        };
                        on_visibility_change.call(vis);
                    },
                    option { value: "private", selected: visibility == Visibility::Private, "Private" }
                    option { value: "unlisted", selected: visibility == Visibility::Unlisted, "Unlisted" }
                    option { value: "public", selected: visibility == Visibility::Public, "Public" }
                }
            }
        }
    }
}
