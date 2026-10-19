//! Whether this site has a chat relay, from the public health check.
//!
//! `NOSTR_RELAY_URL` is server env, not a `site_settings` column.
//! `strategies_enabled` gates Strategies only. The admin Relay page already
//! reads `configured` from `GET /api/nostr/health`. The public shell uses that
//! same response. `configured: false` hides Chat and DMs. A failed fetch is
//! not that answer: links stay up, the socket stays closed, and the check
//! retries a few times. The client does not open `wss://<host>/relay` until
//! `configured` is true.
//!
//! `tracker_latest` rides on this response for the `/stats` install note.
//! Blank, null, and a missing field are absent. The stats page formats a
//! present value into the same sentence as its pinned fallback.

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use serde::Deserialize;

/// Settled answer of `GET /api/nostr/health` field `configured`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayAvailability {
    /// The health request has not finished.
    Unknown,
    /// `configured` is false.
    Off,
    /// `configured` is true.
    On,
    /// The health request failed. Distinct from [`Self::Off`].
    Failed,
}

impl RelayAvailability {
    /// WebSocket dial and DM sync. Unknown and Failed stay closed so a slow
    /// or failed health check cannot start the reconnect loop.
    pub fn allows_connection(self) -> bool {
        matches!(self, Self::On)
    }

    /// Chat and DM links. They stay up until health says the relay is off,
    /// matching a configured site on the first paint. A fetch error keeps
    /// them, the same as [`Self::Unknown`].
    pub fn shows_chat_links(self) -> bool {
        !matches!(self, Self::Off)
    }
}

#[derive(Clone, Copy)]
pub struct RelayAvailabilityState {
    availability: Memo<RelayAvailability>,
    tracker_latest: Memo<Option<String>>,
}

impl RelayAvailabilityState {
    pub fn current(self) -> RelayAvailability {
        *self.availability.read()
    }

    /// Bare `tracker_latest` from the last successful health response.
    /// `None` while the check is in flight, after a fetch error, or when the
    /// field was missing, null, or blank.
    pub fn tracker_latest(self) -> Option<String> {
        self.tracker_latest.read().clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RelayHealthSnapshot {
    pub configured: bool,
    pub tracker_latest: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct RelayHealthConfigured {
    configured: bool,
    #[serde(default)]
    tracker_latest: Option<String>,
}

/// How many health fetches to try before [`RelayAvailability::Failed`].
pub(crate) const HEALTH_FETCH_ATTEMPTS: u32 = 3;
/// Pause between those tries. Short, so a blip does not sit on the check.
pub(crate) const HEALTH_RETRY_BACKOFF_MS: u32 = 400;

/// Backoff after `failed_attempts` failures. `None` means stop.
pub(crate) fn health_retry_backoff_ms(failed_attempts: u32) -> Option<u32> {
    if failed_attempts < HEALTH_FETCH_ATTEMPTS {
        Some(HEALTH_RETRY_BACKOFF_MS)
    } else {
        None
    }
}

/// Drop null, missing, empty, and whitespace-only `tracker_latest` values.
pub(crate) fn normalize_tracker_latest(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn snapshot_from_body(body: RelayHealthConfigured) -> RelayHealthSnapshot {
    RelayHealthSnapshot {
        configured: body.configured,
        tracker_latest: normalize_tracker_latest(body.tracker_latest.as_deref()),
    }
}

/// Map a health fetch to the gate. `None` is still in flight.
/// A transport error is [`RelayAvailability::Failed`], not off.
pub(crate) fn availability_from_health(
    fetched: Option<&Result<bool, String>>,
) -> RelayAvailability {
    match fetched {
        None => RelayAvailability::Unknown,
        Some(Ok(true)) => RelayAvailability::On,
        Some(Ok(false)) => RelayAvailability::Off,
        Some(Err(_)) => RelayAvailability::Failed,
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

async fn pause_health_retry() {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(HEALTH_RETRY_BACKOFF_MS).await;
    #[cfg(not(target_arch = "wasm32"))]
    std::future::ready(()).await;
}

async fn fetch_relay_health_once() -> Result<RelayHealthConfigured, String> {
    ApiClient::web()
        .fetch::<RelayHealthConfigured>("/api/nostr/health")
        .await
        .map_err(|err| err.to_string())
}

async fn load_relay_health() -> Result<RelayHealthSnapshot, String> {
    let mut failed_attempts = 0u32;
    loop {
        match fetch_relay_health_once().await {
            Ok(body) => return Ok(snapshot_from_body(body)),
            Err(err) => {
                failed_attempts += 1;
                if health_retry_backoff_ms(failed_attempts).is_none() {
                    return Err(err);
                }
                pause_health_retry().await;
            }
        }
    }
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
                return Ok(RelayHealthSnapshot {
                    configured: false,
                    tracker_latest: None,
                });
            }
            load_relay_health().await
        }
    });
    let availability = use_memo(move || {
        if let Some(value) = fixed() {
            return value;
        }
        let read = resource.read();
        let mapped = match read.as_ref() {
            None => None,
            Some(Ok(body)) => Some(Ok(body.configured)),
            Some(Err(err)) => Some(Err(err.clone())),
        };
        availability_from_health(mapped.as_ref())
    });
    let tracker_latest = use_memo(move || {
        if fixed().is_some() {
            return None;
        }
        match resource.read().as_ref() {
            Some(Ok(body)) => body.tracker_latest.clone(),
            _ => None,
        }
    });
    let state = RelayAvailabilityState {
        availability,
        tracker_latest,
    };
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
            RelayAvailability::Failed
        );

        assert!(!RelayAvailability::Unknown.allows_connection());
        assert!(!RelayAvailability::Off.allows_connection());
        assert!(RelayAvailability::On.allows_connection());
        assert!(!RelayAvailability::Failed.allows_connection());

        assert!(RelayAvailability::Unknown.shows_chat_links());
        assert!(!RelayAvailability::Off.shows_chat_links());
        assert!(RelayAvailability::On.shows_chat_links());
        assert!(RelayAvailability::Failed.shows_chat_links());

        assert!(!dm_may_sync(RelayAvailability::Off, true));
        assert!(!dm_may_sync(RelayAvailability::Unknown, true));
        assert!(!dm_may_sync(RelayAvailability::Failed, true));
        assert!(!dm_may_sync(RelayAvailability::On, false));
        assert!(dm_may_sync(RelayAvailability::On, true));
    }

    #[test]
    fn health_fetch_retries_a_few_times_then_stops() {
        let mut failures = 0u32;
        let mut pauses = Vec::new();
        loop {
            failures += 1;
            match health_retry_backoff_ms(failures) {
                Some(ms) => pauses.push(ms),
                None => break,
            }
        }
        assert_eq!(failures, HEALTH_FETCH_ATTEMPTS);
        assert_eq!(pauses.len(), (HEALTH_FETCH_ATTEMPTS - 1) as usize);
        assert!(pauses.iter().all(|ms| *ms == HEALTH_RETRY_BACKOFF_MS));
    }

    #[test]
    fn health_json_reads_configured_and_ignores_reachability() {
        let off: RelayHealthConfigured = serde_json::from_str(
            r#"{"configured":false,"reachable":false,"relay_url":null,"forum_backend":"local"}"#,
        )
        .expect("unconfigured health JSON");
        assert!(!off.configured);
        assert_eq!(off.tracker_latest, None);
        let on: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":true,"reachable":false}"#).expect("configured");
        assert!(on.configured);
        assert_eq!(on.tracker_latest, None);
        assert!(
            serde_json::from_str::<RelayHealthConfigured>(r#"{"reachable":true}"#).is_err(),
            "a body without configured is not a relay"
        );
    }

    #[test]
    fn health_json_reads_tracker_latest_full_null_missing_and_empty() {
        let full: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":true,"tracker_latest":"0.5.0-alpha.12"}"#)
                .expect("full tracker_latest");
        assert_eq!(full.tracker_latest.as_deref(), Some("0.5.0-alpha.12"));
        assert_eq!(
            snapshot_from_body(full).tracker_latest.as_deref(),
            Some("0.5.0-alpha.12")
        );

        let null_field: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":true,"tracker_latest":null}"#)
                .expect("null tracker_latest");
        assert_eq!(null_field.tracker_latest, None);
        assert_eq!(snapshot_from_body(null_field).tracker_latest, None);

        let missing: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":false,"reachable":true}"#)
                .expect("missing tracker_latest");
        assert_eq!(missing.tracker_latest, None);
        assert_eq!(snapshot_from_body(missing).tracker_latest, None);

        let empty: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":true,"tracker_latest":""}"#)
                .expect("empty tracker_latest");
        assert_eq!(empty.tracker_latest.as_deref(), Some(""));
        assert_eq!(snapshot_from_body(empty).tracker_latest, None);

        let blank: RelayHealthConfigured =
            serde_json::from_str(r#"{"configured":true,"tracker_latest":"  "}"#)
                .expect("blank tracker_latest");
        assert_eq!(
            normalize_tracker_latest(blank.tracker_latest.as_deref()),
            None
        );
    }
}
