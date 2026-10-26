//! Error text for the strategy loaders (APP9).
//!
//! Errors are `ClientError` Display strings, as kept by the resources in
//! `meta.rs` and `editor.rs`.

use dioxus::prelude::UseResourceState;

use crate::components::is_http_status;

/// What a failed load shows and whether a Retry can help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub message: String,
    pub retry: bool,
}

/// Competitive Meta: a 404 means the strategies gate has Meta switched off,
/// so a Retry cannot help. Every other failure is shown with a Retry.
pub fn meta_load_error(err: &str) -> LoadError {
    if is_http_status(err, 404) {
        LoadError {
            message: "Competitive Meta isn't available right now.".to_string(),
            retry: false,
        }
    } else {
        LoadError {
            message: format!("Couldn't load meta data. {err}"),
            retry: true,
        }
    }
}

/// A Retry is in flight while the resource is pending. The old value (the
/// error) stays in the slot until the refetch resolves.
pub fn is_retrying(state: UseResourceState) -> bool {
    state == UseResourceState::Pending
}

/// Strategy editor: 404 and 403 mean the strategy is gone or private, so a
/// Retry will not help. Anything else (500, network, bad JSON) can be retried.
pub fn strategy_load_error(err: &str) -> LoadError {
    if is_http_status(err, 404) || is_http_status(err, 403) {
        LoadError {
            message: "Strategy not found or access denied.".to_string(),
            retry: false,
        }
    } else {
        LoadError {
            message: format!("Couldn't load this strategy. {err}"),
            retry: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_api_client::ClientError;

    fn http(status: u16, body: &str) -> String {
        ClientError::http(status, body, None).to_string()
    }

    #[test]
    fn missing_or_forbidden_strategy_is_not_retried() {
        for err in [
            http(404, r#"{"error":"Not found"}"#),
            http(403, r#"{"error":"Forbidden"}"#),
            http(404, "<html></html>"),
            http(403, ""),
        ] {
            let shown = strategy_load_error(&err);
            assert_eq!(shown.message, "Strategy not found or access denied.");
            assert!(!shown.retry, "{err}");
        }
    }

    #[test]
    fn server_and_network_failures_offer_retry() {
        for err in [
            http(500, r#"{"error":"Internal server error"}"#),
            http(502, ""),
            http(401, r#"{"error":"Unauthorized"}"#),
            ClientError::Network("offline".to_string()).to_string(),
            ClientError::Deserialize("bad json".to_string()).to_string(),
        ] {
            let shown = strategy_load_error(&err);
            assert!(shown.retry, "{err}");
            assert!(shown.message.starts_with("Couldn't load this strategy."));
            assert!(!shown.message.contains("not found or access denied"));
            assert!(shown.message.contains(&err));
        }
    }

    #[test]
    fn a_500_body_mentioning_404_is_still_retryable() {
        let err = http(500, r#"{"error":"upstream said 404"}"#);
        assert!(strategy_load_error(&err).retry);
    }

    #[test]
    fn meta_failures_show_the_error_with_retry() {
        for err in [
            http(500, r#"{"error":"Internal server error"}"#),
            http(502, ""),
            ClientError::Network("offline".to_string()).to_string(),
        ] {
            let shown = meta_load_error(&err);
            assert!(shown.retry);
            assert_eq!(shown.message, format!("Couldn't load meta data. {err}"));
            assert!(!shown.message.contains("Loading"));
        }
    }

    #[test]
    fn a_meta_404_is_unavailable_without_retry() {
        for err in [http(404, r#"{"error":"Not found"}"#), http(404, "")] {
            let shown = meta_load_error(&err);
            assert!(!shown.retry, "{err}");
            assert_eq!(shown.message, "Competitive Meta isn't available right now.");
        }
        let err = http(500, r#"{"error":"upstream said 404"}"#);
        assert!(meta_load_error(&err).retry);
    }

    #[test]
    fn only_a_pending_resource_is_retrying() {
        assert!(is_retrying(UseResourceState::Pending));
        assert!(!is_retrying(UseResourceState::Ready));
        assert!(!is_retrying(UseResourceState::Stopped));
        assert!(!is_retrying(UseResourceState::Paused));
    }
}
