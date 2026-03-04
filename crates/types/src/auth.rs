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
}
