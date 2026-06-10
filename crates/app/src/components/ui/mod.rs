pub mod button;
pub mod card;
pub mod feedback;
pub mod field;
pub mod label;
pub mod pill;
pub mod shell;
// TODO(task-9): drop this allow once pages/layouts consume the ui re-exports.
#[allow(unused_imports)]
pub use button::{BtnSize, BtnVariant, Button};
#[allow(unused_imports)]
pub use card::Card;
#[allow(unused_imports)]
pub use feedback::{EmptyState, Spinner};
#[allow(unused_imports)]
pub use field::{Input, Select, Textarea};
#[allow(unused_imports)]
pub use label::Label;
#[allow(unused_imports)]
pub use pill::{Pill, PillTone};
#[allow(unused_imports)]
pub use shell::PageShell;

/// Concatenated CSS for every ui/ component, injected once at the app root.
pub fn ui_css() -> String {
    [
        button::BUTTON_CSS,
        card::CARD_CSS,
        pill::PILL_CSS,
        label::LABEL_CSS,
        shell::SHELL_CSS,
        field::FIELD_CSS,
        feedback::FEEDBACK_CSS,
    ]
    .concat()
}
