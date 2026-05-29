//! Layer visibility and opacity management
//!
//! Provides a layered rendering stack for the canvas editor, with per-layer
//! visibility and opacity controls.

use scuffed_types::Bounds;

/// Types of rendering layers
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerType {
    /// Map background image
    Background = 0,
    /// Route/path drawings
    Routes = 1,
    /// Area/zone highlights
    Areas = 2,
    /// Player markers and icons
    Markers = 3,
    /// Text labels
    Text = 4,
    /// Collaborator cursors
    Cursors = 5,
}

/// A layer in the canvas rendering stack
#[derive(Debug, Clone)]
pub struct Layer {
    pub layer_type: LayerType,
    pub visible: bool,
    pub opacity: f32,
    pub bounds: Bounds,
}

impl Layer {
    pub fn new(layer_type: LayerType) -> Self {
        Self {
            layer_type,
            visible: true,
            opacity: 1.0,
            bounds: Bounds::new(0.0, 0.0, 1920.0, 1080.0),
        }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
}

/// Layer manager for organizing canvas content
#[derive(Debug, Clone)]
pub struct LayerManager {
    layers: Vec<Layer>,
}

impl LayerManager {
    pub fn new() -> Self {
        Self {
            layers: vec![
                Layer::new(LayerType::Background),
                Layer::new(LayerType::Areas).with_opacity(0.6),
                Layer::new(LayerType::Routes),
                Layer::new(LayerType::Markers),
                Layer::new(LayerType::Text),
                Layer::new(LayerType::Cursors),
            ],
        }
    }

    pub fn get_layer(&self, layer_type: LayerType) -> Option<&Layer> {
        self.layers.iter().find(|l| l.layer_type == layer_type)
    }

    pub fn get_layer_mut(&mut self, layer_type: LayerType) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.layer_type == layer_type)
    }

    pub fn set_visibility(&mut self, layer_type: LayerType, visible: bool) {
        if let Some(layer) = self.get_layer_mut(layer_type) {
            layer.visible = visible;
        }
    }

    pub fn set_opacity(&mut self, layer_type: LayerType, opacity: f32) {
        if let Some(layer) = self.get_layer_mut(layer_type) {
            layer.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    pub fn is_visible(&self, layer_type: LayerType) -> bool {
        self.get_layer(layer_type).is_some_and(|l| l.visible)
    }

    pub fn opacity(&self, layer_type: LayerType) -> f32 {
        self.get_layer(layer_type).map_or(1.0, |l| l.opacity)
    }

    pub fn visible_layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().filter(|l| l.visible)
    }

    pub fn all_layers(&self) -> &[Layer] {
        &self.layers
    }
}

impl Default for LayerManager {
    fn default() -> Self {
        Self::new()
    }
}
