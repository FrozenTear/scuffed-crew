use serde::{Deserialize, Serialize};

use crate::org::OrgRole;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Discord,
    Google,
}

impl std::fmt::Display for AuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthProvider::Discord => write!(f, "discord"),
            AuthProvider::Google => write!(f, "google"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub role: Option<OrgRole>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeResponse {
    pub user: MeUser,
    pub member: Option<MeMember>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeUser {
    pub id: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeMember {
    pub id: String,
    pub org_role: String,
    pub display_name: String,
    #[serde(default)]
    pub nostr_pubkey: Option<String>,
    #[serde(default)]
    pub nostr_key_mode: Option<String>,
}

/// Public first-boot / login capability flags.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetupStatusResponse {
    pub needs_setup: bool,
    pub local_login: bool,
}

/// Which auth methods the SPA should show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthProvidersResponse {
    pub local: bool,
    pub discord: bool,
    pub google: bool,
    /// Local self-registration is open (recruitment_open in site settings).
    #[serde(default)]
    pub register: bool,
    /// Minimum age the registrant must confirm (site settings, default 16).
    #[serde(default = "default_min_age")]
    pub min_age: u32,
}

fn default_min_age() -> u32 {
    16
}

/// POST /api/auth/local/register — privacy-first: no email collected.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    /// Must be true: registrant confirms they meet the org's minimum age.
    #[serde(default)]
    pub confirm_min_age: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    /// Optional org display name for first-boot settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    /// Homepage preset id (`neutral`, `competitive`, …). Applied once on setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage_preset: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalLoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}
