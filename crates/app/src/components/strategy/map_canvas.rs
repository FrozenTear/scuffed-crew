//! Core map canvas component with three-layer rendering architecture.
//!
//! Uses a layered canvas system for performance:
//! - Layer 1 (Background): Map tiles, health packs -- rarely changes
//! - Layer 2 (Elements): Routes, markers, areas -- redraws on element edits
//! - Layer 3 (Overlay): Cursors, selection, drawing preview -- highest frequency
//!
//! Ported from Leptos to Dioxus 0.7.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::*;
use js_sys::Array;
use uuid::Uuid;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use scuffed_types::strategy::{
    Color, ElementType, HealthPack, HealthPackSize, MapMetadata, Position, StrategyElement, Tool,
};

use crate::canvas::tile_manager::{SharedTileManager, create_tile_manager};
use crate::theme::tokens::{
    CANVAS_BADGE_BG, CANVAS_BG, CANVAS_GRID_LOADING, CANVAS_MARKER_BORDER, CANVAS_SELECTION_COLOR,
    CANVAS_TEXT_LOADING, CANVAS_TILE_PLACEHOLDER, CANVAS_WHITE, HP_GLOW, HP_LARGE_FILL,
    HP_LARGE_STROKE, HP_SMALL_FILL, HP_SMALL_STROKE, STRATEGY_ACCENT,
};

// =============================================================================
// CSS
// =============================================================================

pub const MAP_CANVAS_CSS: &str = r#"
    .map-canvas-wrapper {
        position: relative;
        width: 100%;
        height: 100%;
        overflow: hidden;
        background: var(--bg);
    }
    .map-canvas {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
    }
    .map-canvas-background {
        pointer-events: none;
        z-index: 1;
    }
    .map-canvas-elements {
        pointer-events: none;
        z-index: 2;
    }
    .map-canvas-overlay {
        z-index: 3;
    }
    .map-selector-overlay {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
        display: flex;
        align-items: center;
        justify-content: center;
        background: var(--overlay);
        z-index: 10;
    }
    .map-selector-content {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 2rem;
        max-width: 600px;
        width: 90%;
        max-height: 70vh;
        overflow-y: auto;
    }
    .map-selector-content h2 {
        font-family: var(--font-head);
        font-size: 1.5rem;
        color: var(--text);
        margin-bottom: 0.5rem;
    }
    .map-selector-content p {
        color: var(--text-2);
        margin-bottom: 1.5rem;
    }
    .map-selector-modes {
        display: flex;
        flex-direction: column;
        gap: 0.5rem;
    }
    .map-selector-mode-group {
        border: 1px solid var(--border);
        border-radius: 8px;
        overflow: hidden;
    }
    .map-selector-mode-title {
        padding: 0.6rem 1rem;
        background: var(--surface-2);
        color: var(--text);
        font-weight: 600;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: space-between;
    }
    .map-count {
        font-size: 0.8rem;
        color: var(--text-3);
        background: var(--surface);
        padding: 0.1rem 0.5rem;
        border-radius: 10px;
    }
    .map-selector-grid {
        display: flex;
        flex-direction: column;
    }
    .map-selector-btn,
    .map-selector-btn-submap {
        display: block;
        width: 100%;
        padding: 0.5rem 1rem;
        text-align: left;
        background: transparent;
        border: none;
        color: var(--text-2);
        cursor: pointer;
        transition: background 0.15s, color 0.15s;
    }
    .map-selector-btn:hover,
    .map-selector-btn-submap:hover {
        background: var(--surface-2);
        color: var(--text);
    }
    .map-selector-btn-submap {
        padding-left: 2rem;
        font-size: 0.9rem;
    }
    .map-selector-submap-group {
        border: none;
    }
    .map-selector-expand-hint {
        margin-left: 0.5rem;
        font-size: 0.8rem;
        color: var(--text-3);
    }
    .map-selector-submaps {
        padding-left: 0.5rem;
    }
"#;

// =============================================================================
// Hero image cache
// =============================================================================

/// Hero portrait image cache (Rc<RefCell<>> because HtmlImageElement is !Send)
type HeroImageCache = Rc<RefCell<HashMap<String, HtmlImageElement>>>;

fn create_hero_image_cache() -> HeroImageCache {
    Rc::new(RefCell::new(HashMap::new()))
}

// =============================================================================
// Canvas helper: get context from MountedData
// =============================================================================

fn get_canvas_ctx(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas
        .get_context("2d")
        .ok()?
        .and_then(|ctx| ctx.dyn_into::<CanvasRenderingContext2d>().ok())
}

// =============================================================================
// Component
// =============================================================================

#[component]
pub fn MapCanvas(
    // Canvas state
    zoom: f64,
    pan_offset: Position,
    selected_floor: Option<String>,
    show_health_packs: bool,
    map_metadata: Option<MapMetadata>,
    current_map_id: Option<String>,
    // Drawing state
    active_tool: Tool,
    draw_color: Color,
    fill_opacity: f32,
    selected_hero: Option<String>,
    is_drawing: bool,
    drawing_points: Vec<Position>,
    // Strategy elements
    elements: Vec<StrategyElement>,
    selected_element: Option<Uuid>,
    selected_phase: Option<Uuid>,
    // Events
    on_element_add: EventHandler<StrategyElement>,
    on_element_select: EventHandler<Option<Uuid>>,
    on_element_move: EventHandler<(Uuid, Position)>,
    on_pan_change: EventHandler<Position>,
    on_zoom_change: EventHandler<f64>,
    // Drawing events
    on_drawing_start: EventHandler<Position>,
    on_drawing_continue: EventHandler<Position>,
    on_drawing_finish: EventHandler<()>,
    on_arrow_create: EventHandler<(Position, Position)>,
    on_text_create: EventHandler<(Position, String)>,
    on_erase_at: EventHandler<Position>,
    on_element_drag_end: EventHandler<(Uuid, Position, Position)>,
) -> Element {
    // =========================================================================
    // Canvas element refs — stored as signals so effects can read them
    // =========================================================================
    let mut bg_canvas: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let mut el_canvas: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let mut ov_canvas: Signal<Option<Rc<MountedData>>> = use_signal(|| None);

    // =========================================================================
    // Local interaction state — kept local for smooth 60fps panning
    // =========================================================================
    let mut is_panning = use_signal(|| false);
    let mut is_dragging = use_signal(|| false);
    let mut drag_offset = use_signal(|| Position::new(0.0, 0.0));
    let mut drag_start_pos = use_signal(|| Option::<(Uuid, Position)>::None);
    let mut last_mouse_pos = use_signal(|| Position::new(0.0, 0.0));
    let mut arrow_start = use_signal(|| Option::<Position>::None);
    let mut arrow_end = use_signal(|| Option::<Position>::None);

    // Local pan offset for smooth dragging — only syncs to parent on mouseup.
    // This prevents 60+ signal propagation cycles per second during panning.
    let mut local_pan = use_signal(|| Position::new(0.0, 0.0));

    // Image version counter — incremented when map image or tiles load.
    // Subscribing to this in effects triggers redraws.
    let image_version = use_signal(|| 0u32);

    // =========================================================================
    // Shared non-reactive state (Rc<RefCell<>>)
    // =========================================================================

    // Tile manager
    let tile_manager: SharedTileManager = use_hook(|| create_tile_manager(String::new(), 256));
    let tile_update_scheduled: Rc<RefCell<bool>> = use_hook(|| Rc::new(RefCell::new(false)));

    // Map fallback image
    let map_image: Rc<RefCell<Option<HtmlImageElement>>> = use_hook(|| Rc::new(RefCell::new(None)));
    let current_loaded_map: Rc<RefCell<Option<String>>> = use_hook(|| Rc::new(RefCell::new(None)));

    // Hero portrait cache
    let hero_image_cache: HeroImageCache = use_hook(create_hero_image_cache);

    // Clone non-Copy props for use in multiple closures
    let current_map_id_1 = current_map_id.clone();
    let current_map_id_2 = current_map_id.clone();
    let current_map_id_rsx = current_map_id;
    let elements_1 = elements.clone();
    let elements_2 = elements.clone();
    let elements_3 = elements.clone();
    let elements_4 = elements;

    // =========================================================================
    // Map loading effect — runs when current_map_id changes
    // =========================================================================
    {
        let tile_manager = tile_manager.clone();
        let map_image = map_image.clone();
        let current_loaded_map = current_loaded_map.clone();
        let tile_update_scheduled = tile_update_scheduled.clone();

        use_effect(move || {
            let map_id_val = current_map_id_1.clone();

            // Only reload if the map actually changed
            if map_id_val == *current_loaded_map.borrow() {
                return;
            }
            *current_loaded_map.borrow_mut() = map_id_val.clone();
            *map_image.borrow_mut() = None;

            if let Some(id) = map_id_val {
                // Update tile manager for new map
                tile_manager.borrow_mut().set_map(id.clone());

                // Set up tile loaded callback with debouncing via requestAnimationFrame
                let tile_update_scheduled_cb = tile_update_scheduled.clone();
                let version_signal = image_version;
                tile_manager
                    .borrow_mut()
                    .set_on_tile_loaded(Rc::new(move || {
                        if !*tile_update_scheduled_cb.borrow() {
                            *tile_update_scheduled_cb.borrow_mut() = true;
                            let scheduled_flag = tile_update_scheduled_cb.clone();
                            let mut vs = version_signal;
                            let callback = Closure::once(Box::new(move || {
                                *scheduled_flag.borrow_mut() = false;
                                vs.with_mut(|v| *v = v.wrapping_add(1));
                            })
                                as Box<dyn FnOnce()>);
                            let window = web_sys::window().expect("no window");
                            let _ =
                                window.request_animation_frame(callback.as_ref().unchecked_ref());
                            callback.forget();
                        }
                    }));

                // Load fallback main.png
                let img = HtmlImageElement::new().expect("failed to create image");
                let img_clone = img.clone();
                let map_image_clone = map_image.clone();
                let mut iv = image_version;

                let onload = Closure::<dyn FnMut()>::new(move || {
                    *map_image_clone.borrow_mut() = Some(img_clone.clone());
                    iv.with_mut(|v| *v = v.wrapping_add(1));
                });
                img.set_onload(Some(onload.as_ref().unchecked_ref()));
                onload.forget();

                let onerror = Closure::<dyn Fn()>::new(move || {
                    tracing::error!("Failed to load map image");
                });
                img.set_onerror(Some(onerror.as_ref().unchecked_ref()));
                onerror.forget();

                img.set_src(&format!("/assets/maps/{}/main.png", id));
            } else {
                // No map selected — clear
                tile_manager.borrow_mut().clear();
            }
        });
    }

    // =========================================================================
    // LAYER 1: Background rendering effect
    // =========================================================================
    {
        let tile_manager = tile_manager.clone();
        let map_image = map_image.clone();

        use_effect(move || {
            // Subscribe to reactive state
            let _version = *image_version.read();
            let zoom_val = zoom;
            let editor_pan = pan_offset;
            let panning = *is_panning.read();
            let panning_local = *local_pan.read();
            let pan = if panning { panning_local } else { editor_pan };
            let show_hp = show_health_packs;
            let metadata = map_metadata.clone();
            let floor = selected_floor.clone();
            let map_id = current_map_id_2.clone();

            // Get canvas element
            let Some(ref mounted) = *bg_canvas.read() else {
                return;
            };
            let Some(raw) = mounted.downcast::<web_sys::Element>() else {
                return;
            };
            let Ok(canvas) = raw.clone().dyn_into::<HtmlCanvasElement>() else {
                return;
            };
            let Some(ctx) = get_canvas_ctx(&canvas) else {
                return;
            };

            // Disable image smoothing to prevent seams between tiles
            ctx.set_image_smoothing_enabled(false);

            // Clear canvas
            ctx.set_fill_style_str(CANVAS_BG);
            ctx.fill_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);

            ctx.save();
            let _ = ctx.translate(pan.x, pan.y);
            let _ = ctx.scale(zoom_val, zoom_val);

            // Draw map tiles or fallback image
            if map_id.is_some() {
                let use_tiles = metadata
                    .as_ref()
                    .and_then(|m| m.tile_pyramid.as_ref())
                    .is_some();

                if use_tiles {
                    render_background_tiles(
                        &ctx,
                        &canvas,
                        &tile_manager,
                        &metadata,
                        &floor,
                        zoom_val,
                        pan,
                        show_hp,
                    );
                } else if let Some(ref img) = *map_image.borrow() {
                    // Fallback: draw full image
                    let _ = ctx.draw_image_with_html_image_element(img, 0.0, 0.0);

                    if show_hp && let Some(ref meta) = metadata {
                        draw_health_packs(&ctx, &meta.health_packs, Some(meta));
                    }
                } else {
                    // Placeholder grid while loading
                    ctx.set_stroke_style_str(CANVAS_GRID_LOADING);
                    ctx.set_line_width(1.0);

                    for x in (0..5000i32).step_by(100) {
                        ctx.begin_path();
                        ctx.move_to(x as f64, 0.0);
                        ctx.line_to(x as f64, 5000.0);
                        ctx.stroke();
                    }
                    for y in (0..5000i32).step_by(100) {
                        ctx.begin_path();
                        ctx.move_to(0.0, y as f64);
                        ctx.line_to(5000.0, y as f64);
                        ctx.stroke();
                    }

                    ctx.set_fill_style_str(CANVAS_TEXT_LOADING);
                    ctx.set_font("24px sans-serif");
                    let _ = ctx.fill_text("Loading map...", 20.0, 40.0);
                }
            }

            ctx.restore();
        });
    }

    // =========================================================================
    // LAYER 2: Elements rendering effect
    // =========================================================================
    {
        let hero_image_cache = hero_image_cache.clone();

        use_effect(move || {
            let zoom_val = zoom;
            let editor_pan = pan_offset;
            let panning = *is_panning.read();
            let panning_local = *local_pan.read();
            let pan = if panning { panning_local } else { editor_pan };
            let fill_op = fill_opacity;
            let all_elements = elements_1.clone();

            // Filter to visible elements (phase filtering)
            let visible: Vec<&StrategyElement> = all_elements
                .iter()
                .filter(|e| e.phase_id.is_none() || e.phase_id == selected_phase)
                .collect();

            let Some(ref mounted) = *el_canvas.read() else {
                return;
            };
            let Some(raw) = mounted.downcast::<web_sys::Element>() else {
                return;
            };
            let Ok(canvas) = raw.clone().dyn_into::<HtmlCanvasElement>() else {
                return;
            };
            let Some(ctx) = get_canvas_ctx(&canvas) else {
                return;
            };

            // Viewport culling bounds
            let viewport_x = -pan.x / zoom_val;
            let viewport_y = -pan.y / zoom_val;
            let viewport_width = canvas.width() as f64 / zoom_val;
            let viewport_height = canvas.height() as f64 / zoom_val;
            let cull_padding = 50.0;

            // Clear (transparent to show layer below)
            ctx.clear_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);

            ctx.save();
            let _ = ctx.translate(pan.x, pan.y);
            let _ = ctx.scale(zoom_val, zoom_val);

            for element in &visible {
                if !is_element_visible(
                    element,
                    viewport_x,
                    viewport_y,
                    viewport_width,
                    viewport_height,
                    cull_padding,
                ) {
                    continue;
                }

                draw_element(&ctx, element, fill_op, &hero_image_cache);

                // Draw element number badge
                if let Some(index) = all_elements.iter().position(|e| e.id == element.id) {
                    draw_element_number(&ctx, element, index + 1);
                }
            }

            ctx.restore();
        });
    }

    // =========================================================================
    // LAYER 3: Overlay rendering effect
    // =========================================================================
    use_effect(move || {
        let zoom_val = zoom;
        let editor_pan = pan_offset;
        let panning = *is_panning.read();
        let panning_local = *local_pan.read();
        let pan = if panning { panning_local } else { editor_pan };
        let color = draw_color;
        let drawing = is_drawing;
        let points = drawing_points.clone();
        let arrow_s = *arrow_start.read();
        let arrow_e = *arrow_end.read();
        let sel_id = selected_element;
        let sel_phase = selected_phase;
        let all_elements = elements_2.clone();

        let Some(ref mounted) = *ov_canvas.read() else {
            return;
        };
        let Some(raw) = mounted.downcast::<web_sys::Element>() else {
            return;
        };
        let Ok(canvas) = raw.clone().dyn_into::<HtmlCanvasElement>() else {
            return;
        };
        let Some(ctx) = get_canvas_ctx(&canvas) else {
            return;
        };

        // Clear (transparent)
        ctx.clear_rect(0.0, 0.0, canvas.width() as f64, canvas.height() as f64);

        ctx.save();
        let _ = ctx.translate(pan.x, pan.y);
        let _ = ctx.scale(zoom_val, zoom_val);

        // Drawing preview
        if drawing && !points.is_empty() {
            ctx.set_stroke_style_str(&color.to_css());
            ctx.set_line_width(3.0);
            ctx.set_line_cap("round");
            ctx.set_line_join("round");
            ctx.begin_path();
            ctx.move_to(points[0].x, points[0].y);
            for point in &points[1..] {
                ctx.line_to(point.x, point.y);
            }
            ctx.stroke();
        }

        // Arrow preview
        if let (Some(start), Some(end)) = (arrow_s, arrow_e) {
            ctx.set_stroke_style_str(&color.to_css());
            ctx.set_line_width(4.0);
            ctx.set_line_cap("round");
            ctx.begin_path();
            ctx.move_to(start.x, start.y);
            ctx.line_to(end.x, end.y);
            ctx.stroke();
            draw_arrowhead(&ctx, &start, &end, &color.to_css());
        }

        // Selection highlight
        if let Some(sel) = sel_id {
            let visible: Vec<&StrategyElement> = all_elements
                .iter()
                .filter(|e| e.phase_id.is_none() || e.phase_id == sel_phase)
                .collect();

            if let Some(element) = visible.iter().find(|e| e.id == sel) {
                ctx.set_stroke_style_str(CANVAS_SELECTION_COLOR);
                ctx.set_line_width(2.0);
                let dash = Array::of2(&4.0.into(), &4.0.into());
                let _ = ctx.set_line_dash(&dash);
                ctx.begin_path();
                let _ = ctx.arc(
                    element.position.x,
                    element.position.y,
                    35.0,
                    0.0,
                    std::f64::consts::PI * 2.0,
                );
                ctx.stroke();
                let _ = ctx.set_line_dash(&Array::new());
            }
        }

        ctx.restore();
    });

    // =========================================================================
    // Mouse position helper
    // =========================================================================
    let get_canvas_pos = move |client_x: f64, client_y: f64| -> Position {
        let Some(ref mounted) = *ov_canvas.read() else {
            return Position::new(0.0, 0.0);
        };
        let Some(raw) = mounted.downcast::<web_sys::Element>() else {
            return Position::new(0.0, 0.0);
        };
        let Ok(canvas) = raw.clone().dyn_into::<HtmlCanvasElement>() else {
            return Position::new(0.0, 0.0);
        };

        let rect = canvas.get_bounding_client_rect();
        let pan_val = pan_offset;

        // Account for canvas display scaling (internal vs displayed size)
        let scale_x = canvas.width() as f64 / rect.width();
        let scale_y = canvas.height() as f64 / rect.height();

        let x = ((client_x - rect.left()) * scale_x - pan_val.x) / zoom;
        let y = ((client_y - rect.top()) * scale_y - pan_val.y) / zoom;
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

    // =========================================================================
    // Render
    // =========================================================================
    rsx! {
        div { class: "map-canvas-wrapper",

            // Layer 1 (bottom): Background — map tiles, health packs
            canvas {
                width: "1920",
                height: "1080",
                class: "map-canvas map-canvas-background",
                onmounted: move |evt| {
                    bg_canvas.set(Some(evt.data()));
                },
            }

            // Layer 2 (middle): Elements — routes, markers, areas
            canvas {
                width: "1920",
                height: "1080",
                class: "map-canvas map-canvas-elements",
                onmounted: move |evt| {
                    el_canvas.set(Some(evt.data()));
                },
            }

            // Layer 3 (top): Overlay — cursors, selection, drawing preview
            // This canvas captures all mouse/keyboard events
            canvas {
                width: "1920",
                height: "1080",
                class: "map-canvas map-canvas-overlay",
                style: "cursor: {cursor_style()};",
                tabindex: 0,
                onmounted: move |evt| {
                    ov_canvas.set(Some(evt.data()));
                },

                // ---- Mouse down ----
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
                            // Check if clicking on already-selected element to start dragging
                            if let Some(sel_id) = selected_element
                                && let Some(element) = elements_4.iter().find(|e| e.id == sel_id)
                                    && crate::state::editor::is_position_near_element(pos, element, 30.0) {
                                        is_dragging.set(true);
                                        drag_start_pos.set(Some((sel_id, element.position)));
                                        drag_offset.set(Position::new(
                                            element.position.x - pos.x,
                                            element.position.y - pos.y,
                                        ));
                                        return;
                                    }
                            // Try to select an element at click position
                            let found = elements_3
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
                            if let Some(window) = web_sys::window()
                                && let Ok(Some(text)) = window.prompt_with_message("Enter text:")
                                    && !text.is_empty() {
                                        on_text_create.call((pos, text));
                                    }
                        }
                    }
                },

                // ---- Mouse move ----
                onmousemove: move |evt| {
                    let pos = get_canvas_pos(
                        evt.client_coordinates().x,
                        evt.client_coordinates().y,
                    );

                    if *is_panning.read() {
                        let last = *last_mouse_pos.read();
                        let dx = evt.client_coordinates().x - last.x;
                        let dy = evt.client_coordinates().y - last.y;

                        // Update local pan for smooth 60fps rendering
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

                // ---- Mouse up ----
                onmouseup: move |evt| {
                    let pos = get_canvas_pos(
                        evt.client_coordinates().x,
                        evt.client_coordinates().y,
                    );

                    if *is_panning.read() {
                        // Sync local pan back to parent state once
                        on_pan_change.call(*local_pan.read());
                        is_panning.set(false);
                    }

                    if *is_dragging.read() {
                        is_dragging.set(false);
                        // Fire drag-end with start and final positions for undo
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

                // ---- Wheel (zoom) ----
                onwheel: move |evt| {
                    evt.prevent_default();

                    let Some(ref mounted) = *ov_canvas.read() else { return };
                    let Some(raw) = mounted.downcast::<web_sys::Element>() else { return };
                    let Ok(canvas) = raw.clone().dyn_into::<HtmlCanvasElement>() else { return };
                    let rect = canvas.get_bounding_client_rect();

                    let scale_x = canvas.width() as f64 / rect.width();
                    let scale_y = canvas.height() as f64 / rect.height();

                    // Use page coordinates for wheel events since Dioxus provides them
                    let mouse_x = (evt.data().client_coordinates().x - rect.left()) * scale_x;
                    let mouse_y = (evt.data().client_coordinates().y - rect.top()) * scale_y;

                    // Zoom towards mouse position — extract delta_y from WheelDelta enum
                    let delta_y = match evt.data().delta() {
                        dioxus::html::geometry::WheelDelta::Pixels(v) => v.y,
                        dioxus::html::geometry::WheelDelta::Lines(v) => v.y * 20.0,
                        dioxus::html::geometry::WheelDelta::Pages(v) => v.y * 100.0,
                    };
                    let zoom_in = delta_y < 0.0;
                    let factor = if zoom_in { 1.1 } else { 1.0 / 1.1 };
                    let new_zoom = (zoom * factor).clamp(0.03, 4.0);

                    // Adjust pan to keep the point under the mouse fixed
                    let new_pan_x = mouse_x - (mouse_x - pan_offset.x) * (new_zoom / zoom);
                    let new_pan_y = mouse_y - (mouse_y - pan_offset.y) * (new_zoom / zoom);

                    on_pan_change.call(Position::new(new_pan_x, new_pan_y));
                    on_zoom_change.call(new_zoom);
                },

                // ---- Context menu (prevent) ----
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                },

                // ---- Drag & Drop (hero portraits from HeroPicker) ----
                ondragover: move |evt| {
                    evt.prevent_default();
                },

                ondrop: move |evt| {
                    evt.prevent_default();
                    // Hero drag-and-drop handled by parent through data transfer
                    // The parent should detect the drop and call on_element_add
                },

                // ---- Canvas keyboard shortcuts ----
                onkeydown: move |evt: Event<KeyboardData>| {
                    let key = evt.data().key();
                    match key {
                        Key::Delete | Key::Backspace
                            if selected_element.is_some() => {
                                // Parent handles deletion through element select -> delete flow
                                on_element_select.call(None);
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

            // Map selector overlay when no map selected
            if current_map_id_rsx.is_none() {
                div { class: "map-selector-overlay",
                    div { class: "map-selector-content",
                        h2 { "Select a Map" }
                        p { "Choose a map to start creating your strategy" }
                        div { class: "map-selector-modes",
                            // Map selection handled by parent component's map picker
                            p {
                                style: "color: var(--text-3); text-align: center; padding: 2rem;",
                                "Use the status bar or toolbar to select a map."
                            }
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// Background tile rendering
// =============================================================================

#[allow(clippy::too_many_arguments)]
fn render_background_tiles(
    ctx: &CanvasRenderingContext2d,
    canvas: &HtmlCanvasElement,
    tile_manager: &SharedTileManager,
    metadata: &Option<MapMetadata>,
    floor: &Option<String>,
    zoom: f64,
    pan: Position,
    show_health_packs: bool,
) {
    let Some(meta) = metadata else {
        return;
    };
    let Some(pyramid) = &meta.tile_pyramid else {
        return;
    };

    // Process any tiles that finished loading
    tile_manager.borrow_mut().process_pending();

    // Determine base floor and selected floor
    let base_floor_id = meta
        .floors
        .iter()
        .find(|f| f.is_default)
        .map(|f| f.id.clone())
        .or_else(|| {
            meta.floors
                .iter()
                .find(|f| f.id == "ground")
                .map(|f| f.id.clone())
        })
        .or_else(|| meta.floors.first().map(|f| f.id.clone()))
        .unwrap_or_else(|| "ground".to_string());

    let selected_floor_id = floor.clone().unwrap_or_else(|| base_floor_id.clone());
    let is_overlay_mode = selected_floor_id != base_floor_id;

    let viewport_x = -pan.x / zoom;
    let viewport_y = -pan.y / zoom;
    let viewport_width = canvas.width() as f64 / zoom;
    let viewport_height = canvas.height() as f64 / zoom;

    // Get visible tiles
    let visible = tile_manager.borrow().visible_tiles(
        viewport_x.max(0.0),
        viewport_y.max(0.0),
        viewport_width,
        viewport_height,
        zoom,
        pyramid,
    );

    // Collect tiles to load
    let (base_tiles_to_load, selected_tiles_to_load): (Vec<_>, Vec<_>) = {
        let mut tm = tile_manager.borrow_mut();
        let base_tiles: Vec<_> = if is_overlay_mode {
            visible
                .iter()
                .filter(|(z, x, y)| {
                    tm.get_tile(&base_floor_id, *z, *x, *y).is_none()
                        && !tm.is_loading(&base_floor_id, *z, *x, *y)
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let selected_tiles: Vec<_> = visible
            .iter()
            .filter(|(z, x, y)| {
                tm.get_tile(&selected_floor_id, *z, *x, *y).is_none()
                    && !tm.is_loading(&selected_floor_id, *z, *x, *y)
            })
            .cloned()
            .collect();

        (base_tiles, selected_tiles)
    };

    // Render tiles
    {
        let mut tm = tile_manager.borrow_mut();

        // First pass: base floor (dimmed in overlay mode)
        if is_overlay_mode {
            for (z, x, y) in &visible {
                let (draw_x, draw_y, draw_w, draw_h) = tm.tile_rect(*z, *x, *y, pyramid);

                if let Some(tile_img) = tm.get_tile(&base_floor_id, *z, *x, *y) {
                    ctx.set_global_alpha(0.4);
                    let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(
                        tile_img, draw_x, draw_y, draw_w, draw_h,
                    );
                    ctx.set_global_alpha(1.0);
                } else {
                    ctx.set_fill_style_str(CANVAS_TILE_PLACEHOLDER);
                    ctx.fill_rect(draw_x, draw_y, draw_w, draw_h);
                }
            }
        }

        // Second pass: selected floor (full opacity)
        for (z, x, y) in &visible {
            let (draw_x, draw_y, draw_w, draw_h) = tm.tile_rect(*z, *x, *y, pyramid);

            if let Some(tile_img) = tm.get_tile(&selected_floor_id, *z, *x, *y) {
                let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(
                    tile_img, draw_x, draw_y, draw_w, draw_h,
                );
            } else if !is_overlay_mode {
                ctx.set_fill_style_str(CANVAS_TILE_PLACEHOLDER);
                ctx.fill_rect(draw_x, draw_y, draw_w, draw_h);
            }
        }
    }

    // Load missing tiles
    if !base_tiles_to_load.is_empty() || !selected_tiles_to_load.is_empty() {
        let mut tm = tile_manager.borrow_mut();
        for (z, x, y) in base_tiles_to_load {
            tm.load_tile(&base_floor_id, z, x, y);
        }
        for (z, x, y) in selected_tiles_to_load {
            tm.load_tile(&selected_floor_id, z, x, y);
        }
    }

    // Health packs overlay
    if show_health_packs {
        draw_health_packs(ctx, &meta.health_packs, Some(meta));
    }
}

// =============================================================================
// Element visibility culling
// =============================================================================

fn is_element_visible(
    element: &StrategyElement,
    viewport_x: f64,
    viewport_y: f64,
    viewport_width: f64,
    viewport_height: f64,
    padding: f64,
) -> bool {
    let (min_x, min_y, max_x, max_y) = match &element.element_type {
        ElementType::PlayerMarker => {
            let radius = 20.0;
            (
                element.position.x - radius,
                element.position.y - radius,
                element.position.x + radius,
                element.position.y + radius,
            )
        }
        ElementType::Route { points }
        | ElementType::Area { points }
        | ElementType::Drawing { points, .. } => {
            if points.is_empty() {
                return true;
            }
            let mut min_x = points[0].x;
            let mut min_y = points[0].y;
            let mut max_x = points[0].x;
            let mut max_y = points[0].y;
            for p in points {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
            (min_x, min_y, max_x, max_y)
        }
        ElementType::Arrow { end } => {
            let min_x = element.position.x.min(end.x);
            let min_y = element.position.y.min(end.y);
            let max_x = element.position.x.max(end.x);
            let max_y = element.position.y.max(end.y);
            (min_x, min_y, max_x, max_y)
        }
        ElementType::Text { .. } | ElementType::Icon { .. } | ElementType::Ability { .. } => {
            return true; // Unknown bounds, don't cull
        }
    };

    let viewport_right = viewport_x + viewport_width + padding;
    let viewport_bottom = viewport_y + viewport_height + padding;
    let viewport_left = viewport_x - padding;
    let viewport_top = viewport_y - padding;

    max_x >= viewport_left
        && min_x <= viewport_right
        && max_y >= viewport_top
        && min_y <= viewport_bottom
}

// =============================================================================
// Element drawing
// =============================================================================

fn draw_element(
    ctx: &CanvasRenderingContext2d,
    element: &StrategyElement,
    fill_opacity: f32,
    hero_cache: &HeroImageCache,
) {
    let color = element.color.to_css();

    match &element.element_type {
        ElementType::PlayerMarker => {
            let radius = 20.0;
            let x = element.position.x;
            let y = element.position.y;

            let mut drew_portrait = false;
            if let Some(ref hero_id) = element.hero_id {
                let cache = hero_cache.borrow();
                if let Some(img) = cache.get(hero_id) {
                    // Circular clipping path for hero portrait
                    ctx.save();
                    ctx.begin_path();
                    let _ = ctx.arc(x, y, radius, 0.0, std::f64::consts::PI * 2.0);
                    ctx.clip();

                    let img_size = radius * 2.0;
                    let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(
                        img,
                        x - radius,
                        y - radius,
                        img_size,
                        img_size,
                    );

                    ctx.restore();
                    drew_portrait = true;
                } else {
                    // Image not in cache — start loading
                    drop(cache);
                    let img = HtmlImageElement::new().expect("failed to create image");
                    let hero_id_clone = hero_id.clone();

                    hero_cache
                        .borrow_mut()
                        .insert(hero_id_clone.clone(), img.clone());

                    let onload = Closure::<dyn Fn()>::new(move || {
                        // Image loaded, will be drawn on next render cycle
                    });
                    img.set_onload(Some(onload.as_ref().unchecked_ref()));
                    onload.forget();

                    img.set_src(&format!("/assets/heroes/{}.png", hero_id_clone));
                }
            }

            // Colored circle fallback or border
            if !drew_portrait {
                ctx.set_fill_style_str(&color);
                ctx.begin_path();
                let _ = ctx.arc(x, y, radius, 0.0, std::f64::consts::PI * 2.0);
                ctx.fill();
            }

            // Border
            if drew_portrait {
                ctx.set_stroke_style_str(&color);
                ctx.set_line_width(3.0);
            } else {
                ctx.set_stroke_style_str(CANVAS_MARKER_BORDER);
                ctx.set_line_width(2.0);
            }
            ctx.begin_path();
            let _ = ctx.arc(x, y, radius, 0.0, std::f64::consts::PI * 2.0);
            ctx.stroke();

            // Label (only for plain markers)
            if !drew_portrait && let Some(ref label) = element.label {
                ctx.set_fill_style_str(CANVAS_WHITE);
                ctx.set_font("14px sans-serif");
                ctx.set_text_align("center");
                let _ = ctx.fill_text(label, x, y + 5.0);
            }
        }
        ElementType::Route { points } => {
            if points.len() < 2 {
                return;
            }

            ctx.set_stroke_style_str(&color);
            ctx.set_line_width(4.0);
            ctx.set_line_cap("round");
            ctx.set_line_join("round");

            ctx.begin_path();
            ctx.move_to(points[0].x, points[0].y);
            for point in &points[1..] {
                ctx.line_to(point.x, point.y);
            }
            ctx.stroke();

            // Arrowhead at end
            if let (Some(second_last), Some(last)) =
                (points.get(points.len().saturating_sub(2)), points.last())
            {
                draw_arrowhead(ctx, second_last, last, &color);
            }
        }
        ElementType::Area { points } => {
            if points.len() < 3 {
                return;
            }

            ctx.set_fill_style_str(&element.color.to_css_alpha(fill_opacity));
            ctx.set_stroke_style_str(&color);
            ctx.set_line_width(2.0);

            ctx.begin_path();
            ctx.move_to(points[0].x, points[0].y);
            for point in &points[1..] {
                ctx.line_to(point.x, point.y);
            }
            ctx.close_path();
            ctx.fill();
            ctx.stroke();
        }
        ElementType::Arrow { end } => {
            ctx.set_stroke_style_str(&color);
            ctx.set_line_width(4.0);
            ctx.set_line_cap("round");

            ctx.begin_path();
            ctx.move_to(element.position.x, element.position.y);
            ctx.line_to(end.x, end.y);
            ctx.stroke();

            draw_arrowhead(ctx, &element.position, end, &color);
        }
        ElementType::Text { content, font_size } => {
            ctx.set_fill_style_str(&color);
            ctx.set_font(&format!("{}px sans-serif", font_size));
            let _ = ctx.fill_text(content, element.position.x, element.position.y);
        }
        ElementType::Icon { icon_type } => {
            ctx.set_font("24px sans-serif");
            let _ = ctx.fill_text(icon_type.emoji(), element.position.x, element.position.y);
        }
        ElementType::Drawing {
            points,
            stroke_width,
        } => {
            if points.len() < 2 {
                return;
            }

            ctx.set_stroke_style_str(&color);
            ctx.set_line_width(*stroke_width as f64);
            ctx.set_line_cap("round");
            ctx.set_line_join("round");

            ctx.begin_path();
            ctx.move_to(points[0].x, points[0].y);
            for point in &points[1..] {
                ctx.line_to(point.x, point.y);
            }
            ctx.stroke();
        }
        ElementType::Ability { ability_id } => {
            ctx.set_stroke_style_str(&color);
            ctx.set_line_width(2.0);
            ctx.begin_path();
            let _ = ctx.arc(
                element.position.x,
                element.position.y,
                30.0,
                0.0,
                std::f64::consts::PI * 2.0,
            );
            ctx.stroke();

            ctx.set_fill_style_str(&color);
            ctx.set_font("12px sans-serif");
            ctx.set_text_align("center");
            let _ = ctx.fill_text(ability_id, element.position.x, element.position.y + 4.0);
        }
    }
}

// =============================================================================
// Element number badge
// =============================================================================

fn draw_element_number(ctx: &CanvasRenderingContext2d, element: &StrategyElement, number: usize) {
    let (badge_x, badge_y) = match &element.element_type {
        ElementType::PlayerMarker => (element.position.x + 18.0, element.position.y - 18.0),
        ElementType::Route { points }
        | ElementType::Area { points }
        | ElementType::Drawing { points, .. } => {
            if let Some(first) = points.first() {
                (first.x - 10.0, first.y - 10.0)
            } else {
                (element.position.x - 10.0, element.position.y - 10.0)
            }
        }
        ElementType::Arrow { .. } => (element.position.x - 10.0, element.position.y - 10.0),
        ElementType::Text { .. } => (element.position.x - 10.0, element.position.y - 20.0),
        _ => (element.position.x - 10.0, element.position.y - 10.0),
    };

    let badge_size = 14.0;
    let text = number.to_string();

    // Badge background
    ctx.set_fill_style_str(CANVAS_BADGE_BG);
    ctx.begin_path();
    let _ = ctx.arc(
        badge_x,
        badge_y,
        badge_size / 2.0 + 2.0,
        0.0,
        std::f64::consts::PI * 2.0,
    );
    ctx.fill();

    // Badge border (strategy accent color)
    ctx.set_stroke_style_str(STRATEGY_ACCENT);
    ctx.set_line_width(1.5);
    ctx.stroke();

    // Number text
    ctx.set_fill_style_str(CANVAS_WHITE);
    ctx.set_font("bold 10px sans-serif");
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    let _ = ctx.fill_text(&text, badge_x, badge_y);

    // Reset
    ctx.set_text_baseline("alphabetic");
}

// =============================================================================
// Arrowhead
// =============================================================================

fn draw_arrowhead(ctx: &CanvasRenderingContext2d, from: &Position, to: &Position, color: &str) {
    let angle = (to.y - from.y).atan2(to.x - from.x);
    let size = 15.0;

    ctx.set_fill_style_str(color);
    ctx.begin_path();
    ctx.move_to(to.x, to.y);
    ctx.line_to(
        to.x - size * (angle - 0.5).cos(),
        to.y - size * (angle - 0.5).sin(),
    );
    ctx.line_to(
        to.x - size * (angle + 0.5).cos(),
        to.y - size * (angle + 0.5).sin(),
    );
    ctx.close_path();
    ctx.fill();
}

// =============================================================================
// Health packs
// =============================================================================

fn draw_health_packs(
    ctx: &CanvasRenderingContext2d,
    health_packs: &[HealthPack],
    metadata: Option<&MapMetadata>,
) {
    for pack in health_packs {
        // Convert world coordinates to pixel coordinates
        let (px, py) = if let Some(meta) = metadata {
            meta.transform.world_to_pixel(pack.x, pack.z)
        } else {
            (pack.x, pack.z)
        };

        let (radius, fill_color, stroke_color) = match pack.size {
            HealthPackSize::Small => (8.0, HP_SMALL_FILL, HP_SMALL_STROKE),
            HealthPackSize::Large => (12.0, HP_LARGE_FILL, HP_LARGE_STROKE),
        };

        // Outer glow
        ctx.set_shadow_color(HP_GLOW);
        ctx.set_shadow_blur(6.0);

        // Circle background
        ctx.set_fill_style_str(fill_color);
        ctx.begin_path();
        let _ = ctx.arc(px, py, radius, 0.0, std::f64::consts::PI * 2.0);
        ctx.fill();

        // Reset shadow
        ctx.set_shadow_blur(0.0);

        // Border
        ctx.set_stroke_style_str(stroke_color);
        ctx.set_line_width(2.0);
        ctx.stroke();

        // Cross symbol
        let cross_size = radius * 0.6;
        ctx.set_stroke_style_str(CANVAS_WHITE);
        ctx.set_line_width(2.5);
        ctx.set_line_cap("round");

        ctx.begin_path();
        ctx.move_to(px - cross_size, py);
        ctx.line_to(px + cross_size, py);
        ctx.stroke();

        ctx.begin_path();
        ctx.move_to(px, py - cross_size);
        ctx.line_to(px, py + cross_size);
        ctx.stroke();
    }
}
