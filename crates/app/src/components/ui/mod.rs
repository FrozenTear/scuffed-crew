pub mod button;
// TODO(task-9): drop this allow once pages/layouts consume the ui re-exports.
#[allow(unused_imports)]
pub use button::{BtnSize, BtnVariant, Button};

/// Concatenated CSS for every ui/ component, injected once at the app root.
pub fn ui_css() -> String {
    [button::BUTTON_CSS].concat()
}
