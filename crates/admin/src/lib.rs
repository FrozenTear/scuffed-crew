pub mod api;
pub mod app;
pub mod components;
pub mod guards;
pub mod pages;
pub mod state;

pub use app::AdminApp;

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    tracing::info!("Scuffed Crew admin panel initialized");
    leptos::mount::mount_to_body(AdminApp);
}
