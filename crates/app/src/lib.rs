// Library root — exposes modules for unit testing without requiring the bin
// target (main.rs) to compile. The bin target has WASM/Dioxus platform deps
// that don't run in a standard `cargo test` host environment.

pub mod theme;
