//! Desktop map canvas component using JS bridge via document::eval().
//!
//! On desktop (Dioxus + Wry), web_sys Canvas2D APIs are unavailable because Rust
//! runs natively. This component serializes render state to JSON and delegates all
//! Canvas2D drawing to the injected `desktop_canvas.js` module.

use dioxus::prelude::*;
use uuid::Uuid;

use scuffed_types::strategy::{Color, ElementType, MapMetadata, Position, StrategyElement, Tool};

// =============================================================================
// Serialization helpers for the JS bridge
// =============================================================================

fn color_to_json(c: &Color) -> String {
    format!(r#"{{"r":{},"g":{},"b":{}}}"#, c.r, c.g, c.b)
}

fn position_to_json(p: &Position) -> String {
    format!(r#"{{"x":{},"y":{}}}"#, p.x, p.y)
}

fn element_type_to_json(et: &ElementType) -> String {
    match et {
        ElementType::PlayerMarker => r#""PlayerMarker""#.to_string(),
        ElementType::Route { points } => {
            let pts: Vec<String> = points.iter().map(|p| position_to_json(p)).collect();
            format!(r#"{{"Route":{{"points":[{}]}}}}"#, pts.join(","))
        }
        ElementType::Area { points } => {
            let pts: Vec<String> = points.iter().map(|p| position_to_json(p)).collect();
            format!(r#"{{"Area":{{"points":[{}]}}}}"#, pts.join(","))
        }
        ElementType::Arrow { end } => {
            format!(r#"{{"Arrow":{{"end":{}}}}}"#, position_to_json(end))
        }
        ElementType::Text { content, font_size } => {
            let escaped = content.replace('\\', "\\\\").replace('"', "\\\"");
            format!(
                r#"{{"Text":{{"content":"{}","font_size":{}}}}}"#,
                escaped, font_size
            )
        }
        ElementType::Icon { icon_type } => {
            format!(r#"{{"Icon":{{"icon_type":"{}"}}}}"#, icon_type.emoji())
        }
        ElementType::Drawing {
            points,
            stroke_width,
        } => {
            let pts: Vec<String> = points.iter().map(|p| position_to_json(p)).collect();
            format!(
                r#"{{"Drawing":{{"points":[{}],"stroke_width":{}}}}}"#,
                pts.join(","),
                stroke_width
            )
        }
        ElementType::Ability { ability_id } => {
            let escaped = ability_id.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"{{"Ability":{{"ability_id":"{}"}}}}"#, escaped)
        }
    }
}

fn element_to_json(el: &StrategyElement) -> String {
    let hero = match &el.hero_id {
        Some(h) => format!(r#""{}""#, h),
        None => "null".to_string(),
    };
    let label = match &el.label {
        Some(l) => {
            let escaped = l.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#""{}""#, escaped)
        }
        None => "null".to_string(),
    };
    let phase = match &el.phase_id {
        Some(p) => format!(r#""{}""#, p),
        None => "null".to_string(),
    };
    format!(
        r#"{{"id":"{}","position":{},"color":{},"element_type":{},"hero_id":{},"label":{},"phase_id":{}}}"#,
        el.id,
        position_to_json(&el.position),
        color_to_json(&el.color),
        element_type_to_json(&el.element_type),
        hero,
        label,
        phase
    )
}

fn metadata_to_json(meta: &MapMetadata) -> String {
    serde_json::to_string(meta).unwrap_or_else(|_| "null".to_string())
}

// =============================================================================
// Component
// =============================================================================

#[component]
pub fn DesktopMapCanvas(
    zoom: f64,
    pan_offset: Position,
    selected_floor: Option<String>,
    show_health_packs: bool,
    map_metadata: Option<MapMetadata>,
    current_map_id: Option<String>,
    active_tool: Tool,
    draw_color: Color,
    fill_opacity: f32,
    selected_hero: Option<String>,
    is_drawing: bool,
    drawing_points: Vec<Position>,
    elements: Vec<StrategyElement>,
    selected_element: Option<Uuid>,
    selected_phase: Option<Uuid>,
    on_element_add: EventHandler<StrategyElement>,
    on_element_select: EventHandler<Option<Uuid>>,
    on_element_move: EventHandler<(Uuid, Position)>,
    on_pan_change: EventHandler<Position>,
    on_zoom_change: EventHandler<f64>,
    on_drawing_start: EventHandler<Position>,
    on_drawing_continue: EventHandler<Position>,
    on_drawing_finish: EventHandler<()>,
    on_arrow_create: EventHandler<(Position, Position)>,
    on_text_create: EventHandler<(Position, String)>,
    on_erase_at: EventHandler<Position>,
    on_element_drag_end: EventHandler<(Uuid, Position, Position)>,
) -> Element {
    let mut is_panning = use_signal(|| false);
    let mut is_dragging = use_signal(|| false);
    let mut drag_offset = use_signal(|| Position::new(0.0, 0.0));
    let mut drag_start_pos = use_signal(|| Option::<(Uuid, Position)>::None);
    let mut last_mouse_pos = use_signal(|| Position::new(0.0, 0.0));
    let mut arrow_start = use_signal(|| Option::<Position>::None);
    let mut arrow_end = use_signal(|| Option::<Position>::None);
    let mut local_pan = use_signal(|| Position::new(0.0, 0.0));

    // Canvas rect for mouse coordinate conversion — updated on mount
    let mut canvas_rect = use_signal(|| (0.0f64, 0.0f64, 1920.0f64, 1080.0f64));

    // Track current map for JS init
    let mut current_loaded_map: Signal<Option<String>> = use_signal(|| None);

    // =========================================================================
    // Initialize JS module and map on change
    // =========================================================================
    {
        let map_id = current_map_id.clone();
        use_effect(move || {
            let map_id_val = map_id.clone();
            if map_id_val == *current_loaded_map.peek() {
                return;
            }
            *current_loaded_map.write() = map_id_val.clone();

            let init_id = map_id_val
                .as_deref()
                .map(|s| format!(r#""{}""#, s))
                .unwrap_or_else(|| "null".to_string());
            document::eval(&format!("if(window.owCanvas)window.owCanvas.setMap({init_id})"));
        });
    }

    // =========================================================================
    // Background rendering effect
    // =========================================================================
    {
        let map_id = current_map_id.clone();
        let metadata = map_metadata.clone();
        let floor = selected_floor.clone();

        use_effect(move || {
            let zoom_val = zoom;
            let panning = *is_panning.read();
            let panning_local = *local_pan.read();
            let pan = if panning { panning_local } else { pan_offset };
            let meta_json = metadata
                .as_ref()
                .map(|m| metadata_to_json(m))
                .unwrap_or_else(|| "null".to_string());
            let floor_json = floor
                .as_ref()
                .map(|f| format!(r#""{}""#, f))
                .unwrap_or_else(|| "null".to_string());
            let map_json = map_id
                .as_ref()
                .map(|m| format!(r#""{}""#, m))
                .unwrap_or_else(|| "null".to_string());

            let js = format!(
                "if(window.owCanvas)window.owCanvas.renderBackground({{zoom:{},pan:{},mapId:{},metadata:{},selectedFloor:{},showHealthPacks:{}}})",
                zoom_val,
                position_to_json(&pan),
                map_json,
                meta_json,
                floor_json,
                show_health_packs
            );
            document::eval(&js);
        });
    }

    // =========================================================================
    // Elements rendering effect
    // =========================================================================
    {
        let elements_render = elements.clone();

        use_effect(move || {
            let zoom_val = zoom;
            let panning = *is_panning.read();
            let panning_local = *local_pan.read();
            let pan = if panning { panning_local } else { pan_offset };
            let phase_json = selected_phase
                .map(|p| format!(r#""{}""#, p))
                .unwrap_or_else(|| "null".to_string());

            let els: Vec<String> = elements_render.iter().map(|e| element_to_json(e)).collect();

            let js = format!(
                "if(window.owCanvas)window.owCanvas.renderElements({{zoom:{},pan:{},elements:[{}],selectedPhase:{},fillOpacity:{}}})",
                zoom_val,
                position_to_json(&pan),
                els.join(","),
                phase_json,
                fill_opacity
            );
            document::eval(&js);
        });
    }

    // =========================================================================
    // Overlay rendering effect
    // =========================================================================
    {
        let elements_overlay = elements.clone();

        use_effect(move || {
            let zoom_val = zoom;
            let panning = *is_panning.read();
            let panning_local = *local_pan.read();
            let pan = if panning { panning_local } else { pan_offset };
            let arrow_s = *arrow_start.read();
            let arrow_e = *arrow_end.read();
            let sel_id = selected_element;
            let phase_json = selected_phase
                .map(|p| format!(r#""{}""#, p))
                .unwrap_or_else(|| "null".to_string());

            let pts: Vec<String> = drawing_points.iter().map(|p| position_to_json(p)).collect();
            let arrow_s_json = arrow_s
                .map(|p| position_to_json(&p))
                .unwrap_or_else(|| "null".to_string());
            let arrow_e_json = arrow_e
                .map(|p| position_to_json(&p))
                .unwrap_or_else(|| "null".to_string());
            let sel_json = sel_id
                .map(|s| format!(r#""{}""#, s))
                .unwrap_or_else(|| "null".to_string());

            let els: Vec<String> = elements_overlay.iter().map(|e| element_to_json(e)).collect();

            let js = format!(
                "if(window.owCanvas)window.owCanvas.renderOverlay({{zoom:{},pan:{},drawColor:{},isDrawing:{},drawingPoints:[{}],arrowStart:{},arrowEnd:{},selectedElement:{},selectedPhase:{},elements:[{}]}})",
                zoom_val,
                position_to_json(&pan),
                color_to_json(&draw_color),
                is_drawing,
                pts.join(","),
                arrow_s_json,
                arrow_e_json,
                sel_json,
                phase_json,
                els.join(",")
            );
            document::eval(&js);
        });
    }

    // =========================================================================
    // Mouse position helper
    // =========================================================================
    let get_canvas_pos = move |client_x: f64, client_y: f64| -> Position {
        let (rect_left, rect_top, rect_width, rect_height) = *canvas_rect.read();
        let scale_x = 1920.0 / rect_width;
        let scale_y = 1080.0 / rect_height;
        let pan_val = pan_offset;
        let x = ((client_x - rect_left) * scale_x - pan_val.x) / zoom;
        let y = ((client_y - rect_top) * scale_y - pan_val.y) / zoom;
        Position::new(x, y)
    };

    // =========================================================================
    // Cursor style
    // =========================================================================
    let cursor_style = {
        let is_dragging_val = is_dragging;
        let is_panning_val = is_panning;
        move || -> &'static str {
            if *is_dragging_val.read() {
                return "grabbing";
            }
            match active_tool {
                Tool::Select => {
                    if selected_element.is_some() {
                        "move"
                    } else {
                        "default"
                    }
                }
                Tool::Pan => {
                    if *is_panning_val.read() {
                        "grabbing"
                    } else {
                        "grab"
                    }
                }
                Tool::PlayerMarker | Tool::Route | Tool::Area | Tool::Arrow => "crosshair",
                Tool::Text => "text",
                Tool::Eraser => "crosshair",
            }
        }
    };

    let elements_1 = elements.clone();
    let elements_2 = elements.clone();
    let current_map_id_rsx = current_map_id;

    // =========================================================================
    // Render
    // =========================================================================
    rsx! {
        div { class: "map-canvas-wrapper",

            canvas {
                id: "owc-bg",
                width: "1920",
                height: "1080",
                class: "map-canvas map-canvas-background",
            }

            canvas {
                id: "owc-el",
                width: "1920",
                height: "1080",
                class: "map-canvas map-canvas-elements",
            }

            canvas {
                id: "owc-ov",
                width: "1920",
                height: "1080",
                class: "map-canvas map-canvas-overlay",
                style: "cursor: {cursor_style()};",
                tabindex: 0,
                onmounted: move |evt| {
                    let data = evt.data();
                    spawn(async move {
                        if let Ok(rect) = data.get_client_rect().await {
                            canvas_rect.set((
                                rect.origin.x,
                                rect.origin.y,
                                rect.size.width,
                                rect.size.height,
                            ));
                        }
                    });
                },

                onmousedown: move |evt| {
                    let pos = get_canvas_pos(
                        evt.client_coordinates().x,
                        evt.client_coordinates().y,
                    );

                    match active_tool {
                        Tool::Pan => {
                            is_panning.set(true);
                            local_pan.set(pan_offset);
                            last_mouse_pos.set(Position::new(
                                evt.client_coordinates().x,
                                evt.client_coordinates().y,
                            ));
                        }
                        Tool::PlayerMarker => {
                            let mut elem = StrategyElement::new(
                                ElementType::PlayerMarker,
                                pos,
                            ).with_color(draw_color);
                            if let Some(ref hero) = selected_hero {
                                elem = elem.with_hero(hero.clone());
                            }
                            on_element_add.call(elem);
                        }
                        Tool::Route | Tool::Area => {
                            on_drawing_start.call(pos);
                        }
                        Tool::Arrow => {
                            arrow_start.set(Some(pos));
                        }
                        Tool::Select => {
                            if let Some(sel_id) = selected_element {
                                if let Some(element) = elements_1.iter().find(|e| e.id == sel_id) {
                                    if crate::state::editor::is_position_near_element(pos, element, 30.0) {
                                        is_dragging.set(true);
                                        drag_start_pos.set(Some((sel_id, element.position)));
                                        drag_offset.set(Position::new(
                                            element.position.x - pos.x,
                                            element.position.y - pos.y,
                                        ));
                                        return;
                                    }
                                }
                            }
                            let found = elements_2
                                .iter()
                                .rev()
                                .find(|e| crate::state::editor::is_position_near_element(pos, e, 30.0))
                                .map(|e| e.id);
                            on_element_select.call(found);
                        }
                        Tool::Eraser => {
                            on_erase_at.call(pos);
                        }
                        Tool::Text => {
                            // Desktop text input via eval prompt
                            let pos_clone = pos;
                            spawn(async move {
                                let result = document::eval(
                                    "window.prompt('Enter text:')"
                                ).await;
                                if let Ok(serde_json::Value::String(text)) = result {
                                    if !text.is_empty() {
                                        on_text_create.call((pos_clone, text));
                                    }
                                }
                            });
                        }
                    }
                },

                onmousemove: move |evt| {
                    let pos = get_canvas_pos(
                        evt.client_coordinates().x,
                        evt.client_coordinates().y,
                    );

                    if *is_panning.read() {
                        let last = *last_mouse_pos.read();
                        let dx = evt.client_coordinates().x - last.x;
                        let dy = evt.client_coordinates().y - last.y;

                        local_pan.with_mut(|p| {
                            p.x += dx;
                            p.y += dy;
                        });

                        last_mouse_pos.set(Position::new(
                            evt.client_coordinates().x,
                            evt.client_coordinates().y,
                        ));
                    } else if *is_dragging.read() {
                        if let Some(sel_id) = selected_element {
                            let offset = *drag_offset.read();
                            let new_pos = Position::new(pos.x + offset.x, pos.y + offset.y);
                            on_element_move.call((sel_id, new_pos));
                        }
                    } else if is_drawing {
                        on_drawing_continue.call(pos);
                    } else if arrow_start.read().is_some() {
                        arrow_end.set(Some(pos));
                    }
                },

                onmouseup: move |evt| {
                    let pos = get_canvas_pos(
                        evt.client_coordinates().x,
                        evt.client_coordinates().y,
                    );

                    if *is_panning.read() {
                        on_pan_change.call(*local_pan.read());
                        is_panning.set(false);
                    }

                    if *is_dragging.read() {
                        is_dragging.set(false);
                        if let Some((id, start_pos)) = drag_start_pos.take() {
                            on_element_drag_end.call((id, start_pos, pos));
                        }
                    }

                    match active_tool {
                        Tool::Route | Tool::Area => {
                            on_drawing_finish.call(());
                        }
                        Tool::Arrow => {
                            let start_val = *arrow_start.read();
                            if let Some(start) = start_val {
                                on_arrow_create.call((start, pos));
                                arrow_start.set(None);
                                arrow_end.set(None);
                            }
                        }
                        _ => {}
                    }
                },

                onwheel: move |evt| {
                    evt.prevent_default();

                    let (rect_left, rect_top, rect_width, rect_height) = *canvas_rect.read();
                    let scale_x = 1920.0 / rect_width;
                    let scale_y = 1080.0 / rect_height;

                    let mouse_x = (evt.data().client_coordinates().x - rect_left) * scale_x;
                    let mouse_y = (evt.data().client_coordinates().y - rect_top) * scale_y;

                    let delta_y = match evt.data().delta() {
                        dioxus::html::geometry::WheelDelta::Pixels(v) => v.y,
                        dioxus::html::geometry::WheelDelta::Lines(v) => v.y * 20.0,
                        dioxus::html::geometry::WheelDelta::Pages(v) => v.y * 100.0,
                    };
                    let zoom_in = delta_y < 0.0;
                    let factor = if zoom_in { 1.1 } else { 1.0 / 1.1 };
                    let new_zoom = (zoom * factor).clamp(0.03, 4.0);

                    let new_pan_x = mouse_x - (mouse_x - pan_offset.x) * (new_zoom / zoom);
                    let new_pan_y = mouse_y - (mouse_y - pan_offset.y) * (new_zoom / zoom);

                    on_pan_change.call(Position::new(new_pan_x, new_pan_y));
                    on_zoom_change.call(new_zoom);
                },

                oncontextmenu: move |evt| {
                    evt.prevent_default();
                },

                ondragover: move |evt| {
                    evt.prevent_default();
                },

                ondrop: move |evt| {
                    evt.prevent_default();
                },

                onkeydown: move |evt: Event<KeyboardData>| {
                    let key = evt.data().key();
                    match key {
                        Key::Delete | Key::Backspace => {
                            if selected_element.is_some() {
                                on_element_select.call(None);
                            }
                        }
                        Key::Escape => {
                            on_element_select.call(None);
                            arrow_start.set(None);
                            arrow_end.set(None);
                        }
                        _ => {}
                    }
                },
            }

            if current_map_id_rsx.is_none() {
                div { class: "map-selector-overlay",
                    div { class: "map-selector-content",
                        h2 { "Select a Map" }
                        p { "Choose a map to start creating your strategy" }
                        div { class: "map-selector-modes",
                            p {
                                style: "color: var(--text-muted); text-align: center; padding: 2rem;",
                                "Use the status bar or toolbar to select a map."
                            }
                        }
                    }
                }
            }
        }
    }
}
