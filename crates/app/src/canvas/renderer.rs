//! Canvas2D drawing primitives
//!
//! Wrapper around CanvasRenderingContext2d providing higher-level drawing operations
//! for strategy visualization.

use scuffed_types::{Color, Position};
use web_sys::CanvasRenderingContext2d;

use super::layers::LayerManager;

/// Canvas renderer for strategy visualization
#[derive(Debug, Clone)]
pub struct CanvasRenderer {
    ctx: CanvasRenderingContext2d,
    width: f64,
    height: f64,
    zoom: f64,
    pan: Position,
    layers: LayerManager,
}

impl CanvasRenderer {
    pub fn new(ctx: CanvasRenderingContext2d, width: f64, height: f64) -> Self {
        Self {
            ctx,
            width,
            height,
            zoom: 1.0,
            pan: Position::new(0.0, 0.0),
            layers: LayerManager::new(),
        }
    }

    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom = zoom.clamp(0.03, 4.0);
    }

    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    pub fn set_pan(&mut self, pan: Position) {
        self.pan = pan;
    }

    pub fn pan(&self) -> Position {
        self.pan
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn height(&self) -> f64 {
        self.height
    }

    pub fn ctx(&self) -> &CanvasRenderingContext2d {
        &self.ctx
    }

    pub fn clear(&self) {
        self.ctx.set_fill_style_str("#1a1a2e");
        self.ctx.fill_rect(0.0, 0.0, self.width, self.height);
    }

    pub fn begin_frame(&self) {
        self.ctx.save();
        // Canvas 2D ops only fail on context loss; a dropped frame beats a WASM panic.
        let _ = self.ctx.translate(self.pan.x, self.pan.y);
        let _ = self.ctx.scale(self.zoom, self.zoom);
    }

    pub fn end_frame(&self) {
        self.ctx.restore();
    }

    pub fn draw_grid(&self, cell_size: f64, color: &str) {
        self.ctx.set_stroke_style_str(color);
        self.ctx.set_line_width(0.5);

        let num_cols = (self.width / self.zoom / cell_size).ceil() as i32 + 1;
        let num_rows = (self.height / self.zoom / cell_size).ceil() as i32 + 1;

        let start_x = (-self.pan.x / self.zoom / cell_size).floor() as i32;
        let start_y = (-self.pan.y / self.zoom / cell_size).floor() as i32;

        for i in start_x..(start_x + num_cols) {
            let x = i as f64 * cell_size;
            self.ctx.begin_path();
            self.ctx.move_to(x, start_y as f64 * cell_size);
            self.ctx.line_to(x, (start_y + num_rows) as f64 * cell_size);
            self.ctx.stroke();
        }

        for i in start_y..(start_y + num_rows) {
            let y = i as f64 * cell_size;
            self.ctx.begin_path();
            self.ctx.move_to(start_x as f64 * cell_size, y);
            self.ctx.line_to((start_x + num_cols) as f64 * cell_size, y);
            self.ctx.stroke();
        }
    }

    pub fn draw_circle(&self, pos: &Position, radius: f64, fill: &Color, stroke: Option<&Color>) {
        self.ctx.set_fill_style_str(&fill.to_css());
        self.ctx.begin_path();
        let _ = self
            .ctx
            .arc(pos.x, pos.y, radius, 0.0, std::f64::consts::PI * 2.0);
        self.ctx.fill();

        if let Some(stroke_color) = stroke {
            self.ctx.set_stroke_style_str(&stroke_color.to_css());
            self.ctx.set_line_width(2.0);
            self.ctx.stroke();
        }
    }

    pub fn draw_line(&self, from: &Position, to: &Position, color: &Color, width: f64) {
        self.ctx.set_stroke_style_str(&color.to_css());
        self.ctx.set_line_width(width);
        self.ctx.set_line_cap("round");

        self.ctx.begin_path();
        self.ctx.move_to(from.x, from.y);
        self.ctx.line_to(to.x, to.y);
        self.ctx.stroke();
    }

    pub fn draw_path(&self, points: &[Position], color: &Color, width: f64, closed: bool) {
        if points.len() < 2 {
            return;
        }

        self.ctx.set_stroke_style_str(&color.to_css());
        self.ctx.set_line_width(width);
        self.ctx.set_line_cap("round");
        self.ctx.set_line_join("round");

        self.ctx.begin_path();
        self.ctx.move_to(points[0].x, points[0].y);

        for point in &points[1..] {
            self.ctx.line_to(point.x, point.y);
        }

        if closed {
            self.ctx.close_path();
        }

        self.ctx.stroke();
    }

    pub fn draw_polygon(&self, points: &[Position], fill: &Color, stroke: Option<&Color>) {
        if points.len() < 3 {
            return;
        }

        self.ctx.begin_path();
        self.ctx.move_to(points[0].x, points[0].y);

        for point in &points[1..] {
            self.ctx.line_to(point.x, point.y);
        }

        self.ctx.close_path();

        self.ctx.set_fill_style_str(&fill.to_css());
        self.ctx.fill();

        if let Some(stroke_color) = stroke {
            self.ctx.set_stroke_style_str(&stroke_color.to_css());
            self.ctx.set_line_width(2.0);
            self.ctx.stroke();
        }
    }

    pub fn draw_text(&self, text: &str, pos: &Position, color: &Color, font_size: f64) {
        self.ctx.set_fill_style_str(&color.to_css());
        self.ctx.set_font(&format!("{}px sans-serif", font_size));
        let _ = self.ctx.fill_text(text, pos.x, pos.y);
    }

    pub fn draw_arrowhead(&self, from: &Position, to: &Position, color: &Color, size: f64) {
        let angle = (to.y - from.y).atan2(to.x - from.x);

        self.ctx.set_fill_style_str(&color.to_css());
        self.ctx.begin_path();
        self.ctx.move_to(to.x, to.y);
        self.ctx.line_to(
            to.x - size * (angle - 0.5).cos(),
            to.y - size * (angle - 0.5).sin(),
        );
        self.ctx.line_to(
            to.x - size * (angle + 0.5).cos(),
            to.y - size * (angle + 0.5).sin(),
        );
        self.ctx.close_path();
        self.ctx.fill();
    }

    /// Convert screen coordinates to canvas coordinates
    pub fn screen_to_canvas(&self, screen_pos: &Position) -> Position {
        Position::new(
            (screen_pos.x - self.pan.x) / self.zoom,
            (screen_pos.y - self.pan.y) / self.zoom,
        )
    }

    /// Convert canvas coordinates to screen coordinates
    pub fn canvas_to_screen(&self, canvas_pos: &Position) -> Position {
        Position::new(
            canvas_pos.x * self.zoom + self.pan.x,
            canvas_pos.y * self.zoom + self.pan.y,
        )
    }

    pub fn layers(&self) -> &LayerManager {
        &self.layers
    }

    pub fn layers_mut(&mut self) -> &mut LayerManager {
        &mut self.layers
    }
}
