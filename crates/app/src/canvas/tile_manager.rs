//! Tile manager for loading and caching map tiles
//!
//! Handles progressive tile loading for large map images using a tile pyramid structure.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use scuffed_types::TilePyramidInfo;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlImageElement;

/// Key for identifying a tile: (floor_id, zoom_level, tile_x, tile_y)
pub type TileKey = (String, u32, u32, u32);

/// Manages tile loading and caching for map rendering
#[derive(Clone)]
pub struct TileManager {
    /// Map ID for constructing tile paths
    map_id: String,
    /// Loaded tiles
    cache: HashMap<TileKey, HtmlImageElement>,
    /// Tiles currently being loaded
    loading: HashSet<TileKey>,
    /// Tiles that have finished loading, waiting to be moved to cache
    pending: Rc<RefCell<Vec<(TileKey, HtmlImageElement)>>>,
    /// Maximum tiles to keep in cache (LRU eviction)
    cache_limit: usize,
    /// LRU order tracking - most recently used at the end
    lru_order: Vec<TileKey>,
    /// Callback to trigger re-render when tile loads
    on_tile_loaded: Option<Rc<dyn Fn()>>,
}

impl std::fmt::Debug for TileManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileManager")
            .field("map_id", &self.map_id)
            .field("cache_len", &self.cache.len())
            .field("loading_len", &self.loading.len())
            .field("cache_limit", &self.cache_limit)
            .field("lru_order_len", &self.lru_order.len())
            .field("on_tile_loaded", &self.on_tile_loaded.as_ref().map(|_| ".."))
            .finish()
    }
}

impl TileManager {
    pub fn new(map_id: String, _tile_size: u32) -> Self {
        Self {
            map_id,
            cache: HashMap::new(),
            loading: HashSet::new(),
            pending: Rc::new(RefCell::new(Vec::new())),
            cache_limit: 256, // Keep up to 256 tiles in memory
            lru_order: Vec::with_capacity(256),
            on_tile_loaded: None,
        }
    }

    /// Set callback for when tiles finish loading
    pub fn set_on_tile_loaded(&mut self, callback: Rc<dyn Fn()>) {
        self.on_tile_loaded = Some(callback);
    }

    /// Change the current map
    pub fn set_map(&mut self, map_id: String) {
        if self.map_id != map_id {
            self.map_id = map_id;
            self.cache.clear();
            self.loading.clear();
            self.pending.borrow_mut().clear();
            self.lru_order.clear();
        }
    }

    /// Get a tile if it's loaded and mark it as recently used
    pub fn get_tile(&mut self, floor: &str, z: u32, x: u32, y: u32) -> Option<&HtmlImageElement> {
        let key = (floor.to_string(), z, x, y);
        if self.cache.contains_key(&key) {
            // Move to end of LRU order (most recently used)
            self.touch_lru(&key);
            self.cache.get(&key)
        } else {
            None
        }
    }

    /// Mark a tile as recently used (move to end of LRU order)
    fn touch_lru(&mut self, key: &TileKey) {
        if let Some(pos) = self.lru_order.iter().position(|k| k == key) {
            self.lru_order.remove(pos);
        }
        self.lru_order.push(key.clone());
    }

    /// Check if a tile is currently loading
    pub fn is_loading(&self, floor: &str, z: u32, x: u32, y: u32) -> bool {
        let key = (floor.to_string(), z, x, y);
        self.loading.contains(&key)
    }

    /// Request a tile to be loaded (async)
    pub fn load_tile(&mut self, floor: &str, z: u32, x: u32, y: u32) {
        let key = (floor.to_string(), z, x, y);

        // Already loaded or loading
        if self.cache.contains_key(&key) || self.loading.contains(&key) {
            return;
        }

        self.loading.insert(key.clone());

        // Evict old tiles if at capacity
        self.evict_if_needed();

        // Construct tile URL
        let url = format!(
            "/assets/maps/{}/floors/{}/{}/{}/{}.webp",
            self.map_id, floor, z, x, y
        );

        // Create image element and start loading
        let img = HtmlImageElement::new().expect("failed to create image");

        let key_clone = key.clone();
        let on_loaded = self.on_tile_loaded.clone();
        let pending = self.pending.clone();
        let img_clone = img.clone();

        let onload = Closure::<dyn Fn()>::new(move || {
            // Store in pending queue - will be processed on next render
            pending.borrow_mut().push((key_clone.clone(), img_clone.clone()));
            if let Some(ref callback) = on_loaded {
                callback();
            }
        });
        img.set_onload(Some(onload.as_ref().unchecked_ref()));
        onload.forget();

        // Handle load errors silently
        let onerror = Closure::<dyn Fn()>::new(move || {
            // Tile doesn't exist or failed to load - that's ok
        });
        img.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        // Start loading
        img.set_src(&url);
    }

    /// Process any tiles that finished loading
    /// Call this before rendering to update the cache
    pub fn process_pending(&mut self) {
        let pending_tiles: Vec<_> = self.pending.borrow_mut().drain(..).collect();
        for (key, img) in pending_tiles {
            self.loading.remove(&key);
            self.cache.insert(key.clone(), img);
            // Add to LRU order (newly loaded tiles are most recently used)
            self.lru_order.push(key);
        }
    }

    /// Evict least recently used tiles if cache is at capacity
    fn evict_if_needed(&mut self) {
        while self.cache.len() >= self.cache_limit && !self.lru_order.is_empty() {
            // Remove least recently used tile (front of the list)
            let key = self.lru_order.remove(0);
            self.cache.remove(&key);
        }
    }

    /// Clear all cached tiles
    pub fn clear(&mut self) {
        self.cache.clear();
        self.loading.clear();
        self.lru_order.clear();
    }

    /// Clear tiles for a specific floor
    pub fn clear_floor(&mut self, floor: &str) {
        self.cache.retain(|k, _| k.0 != floor);
        self.loading.retain(|k| k.0 != floor);
        self.lru_order.retain(|k| k.0 != floor);
    }

    /// Calculate zoom level from canvas zoom factor
    ///
    /// Canvas zoom 1.0 = max detail (max_zoom)
    /// Canvas zoom 0.5 = one level down, etc.
    pub fn zoom_to_level(&self, canvas_zoom: f64, max_zoom: u32) -> u32 {
        // Use round() instead of floor() for smoother transitions
        // This prevents jumping to high-detail tiles too early
        let level_offset = (-canvas_zoom.log2()).round() as i32;
        let level = max_zoom as i32 - level_offset;
        level.clamp(0, max_zoom as i32) as u32
    }

    /// Calculate which tiles are visible in the current viewport
    ///
    /// Returns Vec<(z, x, y)> of visible tile coordinates
    pub fn visible_tiles(
        &self,
        viewport_x: f64,
        viewport_y: f64,
        viewport_width: f64,
        viewport_height: f64,
        canvas_zoom: f64,
        pyramid: &TilePyramidInfo,
    ) -> Vec<(u32, u32, u32)> {
        let zoom_level = self.zoom_to_level(canvas_zoom, pyramid.max_zoom);
        let (tiles_x, tiles_y) = pyramid.tiles_at_zoom(zoom_level);

        // Calculate scale factor for this zoom level
        let scale = 1 << (pyramid.max_zoom - zoom_level);
        let scaled_tile_size = (pyramid.tile_size * scale) as f64;

        // Calculate visible tile range
        let start_x = (viewport_x / scaled_tile_size).floor() as i32;
        let start_y = (viewport_y / scaled_tile_size).floor() as i32;
        let end_x = ((viewport_x + viewport_width) / scaled_tile_size).ceil() as i32;
        let end_y = ((viewport_y + viewport_height) / scaled_tile_size).ceil() as i32;

        let mut tiles = Vec::new();

        for y in start_y.max(0)..=end_y.min(tiles_y as i32 - 1) {
            for x in start_x.max(0)..=end_x.min(tiles_x as i32 - 1) {
                tiles.push((zoom_level, x as u32, y as u32));
            }
        }

        tiles
    }

    /// Get tile position and size for rendering
    ///
    /// Returns (x, y, width, height) in canvas coordinates
    pub fn tile_rect(
        &self,
        z: u32,
        x: u32,
        y: u32,
        pyramid: &TilePyramidInfo,
    ) -> (f64, f64, f64, f64) {
        let scale = 1 << (pyramid.max_zoom - z);
        let tile_world_size = (pyramid.tile_size * scale) as f64;

        let px = x as f64 * tile_world_size;
        let py = y as f64 * tile_world_size;

        (px, py, tile_world_size, tile_world_size)
    }

    /// Get cache stats for debugging
    pub fn stats(&self) -> (usize, usize) {
        (self.cache.len(), self.loading.len())
    }
}

/// Shared tile manager that can be used across render cycles
pub type SharedTileManager = Rc<RefCell<TileManager>>;

/// Create a new shared tile manager
pub fn create_tile_manager(map_id: String, tile_size: u32) -> SharedTileManager {
    Rc::new(RefCell::new(TileManager::new(map_id, tile_size)))
}
