pub mod oauth;
pub mod discord;
pub mod google;
pub mod matrix;
pub mod session;
pub mod extractor;

pub use oauth::{OAuthProvider, ProviderConfig, ProviderRegistry};
pub use session::{build_session_cookie, build_csrf_cookie, validate_csrf_state, generate_session_token};
pub use extractor::{AuthUser, HasAuth};
