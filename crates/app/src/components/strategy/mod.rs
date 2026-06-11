#[cfg(feature = "web")]
pub mod connection_status;
#[cfg(feature = "desktop")]
pub mod desktop_map_canvas;
pub mod hero_picker;
#[cfg(feature = "web")]
pub mod map_canvas;
pub mod map_canvas_css;
pub mod properties_panel;
pub mod team_panel;
pub mod timeline;
pub mod toolbar;

#[cfg(feature = "desktop")]
pub use desktop_map_canvas::DesktopMapCanvas as MapCanvas;
pub use hero_picker::{HeroPicker, HeroWinRate};
#[cfg(feature = "web")]
pub use map_canvas::MapCanvas;
pub use map_canvas_css::MAP_CANVAS_CSS;
pub use properties_panel::PropertiesPanel;
pub use team_panel::TeamPanel;
pub use timeline::Timeline;
pub use toolbar::Toolbar;
