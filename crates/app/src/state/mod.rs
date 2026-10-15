pub mod auth;
#[cfg(feature = "web")]
pub mod collab;
pub mod editor;
pub mod maps;
#[cfg(feature = "web")]
pub mod nostr;
pub mod relay;
pub mod site_settings;
pub mod undo;

pub use auth::*;
#[cfg(test)]
pub use relay::set_test_relay_availability;
pub use relay::{RelayAvailability, provide_relay_availability, use_relay_availability_state};
#[cfg(test)]
pub use site_settings::set_test_embedded_json;
pub use site_settings::{
    document_title, loaded_site_settings, provide_site_settings, use_site_settings,
};
