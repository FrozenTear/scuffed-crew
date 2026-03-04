use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{MeResponse, OrgRole, UserInfo};

#[derive(Clone, Debug, PartialEq)]
pub struct AuthState {
    pub user: Option<UserInfo>,
    pub loading: bool,
}

impl AuthState {
    pub fn new() -> Self {
        Self {
            user: None,
            loading: true,
        }
    }

    pub fn is_logged_in(&self) -> bool {
        self.user.is_some()
    }

    pub fn is_admin(&self) -> bool {
        self.user
            .as_ref()
            .and_then(|u| u.role.as_ref())
            .is_some_and(|r| *r == OrgRole::Admin)
    }

    pub fn is_officer_or_above(&self) -> bool {
        self.user
            .as_ref()
            .and_then(|u| u.role.as_ref())
            .is_some_and(|r| matches!(r, OrgRole::Admin | OrgRole::Officer))
    }
}

pub fn use_auth() -> Signal<AuthState> {
    use_context::<Signal<AuthState>>()
}

fn me_to_user_info(me: &MeResponse) -> UserInfo {
    let role = me.member.as_ref().and_then(|m| match m.org_role.as_str() {
        "admin" => Some(OrgRole::Admin),
        "officer" => Some(OrgRole::Officer),
        "member" => Some(OrgRole::Member),
        _ => None,
    });

    UserInfo {
        id: me.user.id.clone(),
        username: me.member
            .as_ref()
            .map(|m| m.display_name.clone())
            .unwrap_or_else(|| me.user.username.clone()),
        avatar_url: me.user.avatar_url.clone(),
        role,
    }
}

/// Call this once from the root App component to fetch auth on startup.
pub fn use_auth_init() {
    let mut auth = use_auth();

    use_future(move || async move {
        let client = ApiClient::web();
        match client.get_me().await {
            Ok(me) => {
                let user_info = me_to_user_info(&me);
                auth.set(AuthState {
                    user: Some(user_info),
                    loading: false,
                });
            }
            Err(_) => {
                auth.set(AuthState {
                    user: None,
                    loading: false,
                });
            }
        }
    });
}
