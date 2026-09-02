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

    /// Session user has a recognized org role (recruit+). Logged-in accounts
    /// without a member row — typical after local register — are not members.
    pub fn is_org_member(&self) -> bool {
        self.user.as_ref().and_then(|u| u.role.as_ref()).is_some()
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
        "recruit" => Some(OrgRole::Recruit),
        _ => None,
    });

    UserInfo {
        id: me.user.id.clone(),
        username: me
            .member
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

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: Option<OrgRole>) -> UserInfo {
        UserInfo {
            id: "u1".into(),
            username: "dev".into(),
            avatar_url: None,
            role,
        }
    }

    #[test]
    fn org_member_requires_recognized_role() {
        let loading = AuthState::new();
        assert!(loading.loading);
        assert!(!loading.is_logged_in());
        assert!(!loading.is_org_member());

        let anon = AuthState {
            user: None,
            loading: false,
        };
        assert!(!anon.is_logged_in());
        assert!(!anon.is_org_member());

        let bare = AuthState {
            user: Some(user(None)),
            loading: false,
        };
        assert!(bare.is_logged_in());
        assert!(!bare.is_org_member());

        let member = AuthState {
            user: Some(user(Some(OrgRole::Member))),
            loading: false,
        };
        assert!(member.is_logged_in());
        assert!(member.is_org_member());
    }
}
