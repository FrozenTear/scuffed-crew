pub mod env_flags;
pub mod types;

pub use env_flags::is_production_env;

#[cfg(feature = "crypto")]
pub mod crypto;

#[cfg(feature = "crypto")]
pub mod nip49;

#[cfg(feature = "server")]
pub mod password;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

pub use types::*;
