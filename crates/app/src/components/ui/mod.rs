pub mod button;
pub mod card;
pub mod feedback;
pub mod field;
pub mod label;
pub mod pill;
pub mod shell;
#[allow(unused_imports)]
pub use button::BtnSize;
pub use button::{BtnVariant, Button};
pub use card::Card;
pub use feedback::EmptyState;
#[allow(unused_imports)]
pub use feedback::Spinner;
pub use field::Textarea;
#[allow(unused_imports)]
pub use field::{Input, Select};
#[allow(unused_imports)]
pub use label::Label;
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
