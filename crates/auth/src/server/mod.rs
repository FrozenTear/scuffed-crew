pub mod discord;
pub mod extractor;
pub mod google;
pub mod matrix;
pub mod oauth;
pub mod session;

pub use extractor::{AuthUser, HasAuth};
pub use oauth::{OAuthProvider, ProviderConfig, ProviderRegistry};
pub use session::{
    build_csrf_cookie, build_session_cookie, generate_session_token, validate_csrf_state,
};
