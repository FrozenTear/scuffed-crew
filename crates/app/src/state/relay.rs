//! Whether this site has a chat relay, from the public health check.
//!
//! `NOSTR_RELAY_URL` is server env, not a `site_settings` column.
//! `strategies_enabled` gates Strategies only. The admin Relay page already
//! reads `configured` from `GET /api/nostr/health`. The public shell uses that
//! same flag. A missing or failed check is treated as off so the client does
//! not open `wss://<host>/relay` against an HTML page.

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use serde::Deserialize;

/// Settled answer of `GET /api/nostr/health` field `configured`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayAvailability {
    /// The health request has not finished.
    Unknown,
    /// `configured` is false, or the request failed.
    Off,
    /// `configured` is true.
    On,
}

impl RelayAvailability {
    /// WebSocket dial and DM sync. Unknown stays closed so a slow health
    /// check cannot start the reconnect loop.
    pub fn allows_connection(self) -> bool {
        matches!(self, Self::On)
    }

    /// Chat and DM links. They stay up until health says the relay is off,
    /// matching a configured site on the first paint.
    pub fn shows_chat_links(self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Clone, Copy)]
pub struct RelayAvailabilityState {
    availability: Memo<RelayAvailability>,
}

impl RelayAvailabilityState {
    pub fn current(self) -> RelayAvailability {
        *self.availability.read()
    }
}

#[derive(Deserialize)]
struct RelayHealthConfigured {
    configured: bool,
}

/// Map a health fetch to the gate. `None` is still in flight.
pub(crate) fn availability_from_health(
    fetched: Option<&Result<bool, String>>,
) -> RelayAvailability {
    match fetched {
        None => RelayAvailability::Unknown,
        Some(Ok(true)) => RelayAvailability::On,
        Some(Ok(false)) | Some(Err(_)) => RelayAvailability::Off,
    }
}

/// DM sync and the conversation list. Both hit the relay when a session exists.
pub(crate) fn dm_may_sync(availability: RelayAvailability, logged_in: bool) -> bool {
    logged_in && availability.allows_connection()
}

#[cfg(test)]
thread_local! {
    static TEST_FIXED: std::cell::Cell<Option<RelayAvailability>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub fn set_test_relay_availability(value: Option<RelayAvailability>) {
    TEST_FIXED.with(|slot| slot.set(value));
}

#[cfg(test)]
fn test_fixed() -> Option<RelayAvailability> {
    TEST_FIXED.with(|slot| slot.get())
}

async fn load_relay_configured() -> Result<bool, String> {
    ApiClient::web()
        .fetch::<RelayHealthConfigured>("/api/nostr/health")
        .await
        .map(|body| body.configured)
        .map_err(|err| err.to_string())
}

/// Call once from [`crate::layouts::PublicLayout`], above the routed page.
pub fn provide_relay_availability() -> RelayAvailabilityState {
    // Tests set the override before mount. Copy it into a signal during this
    // render so later memo reads do not depend on thread-local state.
    #[cfg(test)]
    let fixed = use_signal(test_fixed);
    #[cfg(not(test))]
    let fixed = use_signal(|| Option::<RelayAvailability>::None);

    let resource = use_resource(move || {
        let skip = fixed().is_some();
        async move {
            if skip {
                return Ok(false);
            }
            load_relay_configured().await
        }
    });
    let availability = use_memo(move || {
        if let Some(value) = fixed() {
            return value;
        }
        availability_from_health(resource.read().as_ref())
    });
    let state = RelayAvailabilityState { availability };
    use_context_provider(|| state);
    state
}

pub fn use_relay_availability_state() -> RelayAvailabilityState {
    use_context::<RelayAvailabilityState>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_flag_gates_connection_and_keeps_links_until_off() {
        assert_eq!(availability_from_health(None), RelayAvailability::Unknown);
        assert_eq!(
            availability_from_health(Some(&Ok(false))),
            RelayAvailability::Off
        );
        assert_eq!(
            availability_from_health(Some(&Ok(true))),
            RelayAvailability::On
        );
        assert_eq!(
            availability_from_health(Some(&Err("offline".into()))),
            RelayAvailability::Off
        );

        assert!(!RelayAvailability::Unknown.allows_connection());
        assert!(!RelayAvailability::Off.allows_connection());
        assert!(RelayAvailability::On.allows_connection());

        assert!(RelayAvailability::Unknown.shows_chat_links());
        assert!(!RelayAvailability::Off.shows_chat_links());
        assert!(RelayAvailability::On.shows_chat_links());

        assert!(!dm_may_sync(RelayAvailability::Off, true));
        assert!(!dm_may_sync(RelayAvailability::Unknown, true));
        assert!(!dm_may_sync(RelayAvailability::On, false));
        assert!(dm_may_sync(RelayAvailability::On, true));
    }

    #[test]
    fn health_json_reads_configured_and_ignores_reachability() {
        let off: RelayHealthConfigured = serde_json::from_str(
            r#"{"configured":false,"reachable":false,"relay_url":null,"forum_backend":"local"}"#,
        )
        .expect("unconfigured health JSON");
        assert!(!off.configured);
        let on: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":true,"reachable":false}"#).expect("configured");
        assert!(on.configured);
        assert!(
            serde_json::from_str::<RelayHealthConfigured>(r#"{"reachable":true}"#).is_err(),
            "a body without configured is not a relay"
        );
    }
}
