//! How a [`scuffed_api_client::ClientError`] reads on the page, and pins how
//! an API 404 turns into a variant and a user-facing message.
//!
//! Unknown `/api/*` paths respond with HTTP 404, `Content-Type: application/json`,
//! and `{"error":"Not found"}` instead of the site HTML page.

use scuffed_api_client::ClientError;

/// What a person sees when a request never got an answer from the server
/// (offline, the server is down, a blocked request).
pub const NETWORK_ERROR_MESSAGE: &str =
    "Could not reach the server. Check your connection and try again.";

/// The text a page shows for a failed request.
///
/// A network failure is always [`NETWORK_ERROR_MESSAGE`]: the raw text on
/// [`ClientError::Network`] is the browser's `JsValue` debug string, which
/// Chromium fills with the whole wasm stack trace (QA 706 FIX5: 1,106
/// characters in the settings Save bar). That detail is for the console
/// only ([`crate::util::log_client_error`]). A server answer keeps its reason.
pub fn user_message(err: &ClientError) -> String {
    match err {
        ClientError::Network(_) => NETWORK_ERROR_MESSAGE.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{NETWORK_ERROR_MESSAGE, user_message};
    use scuffed_api_client::ClientError;

    /// The response the API client reads for an unknown `/api/*` path.
    struct ApiNotFound {
        status: u16,
        content_type: &'static str,
        body: &'static str,
    }

    fn map_not_found(response: ApiNotFound) -> ClientError {
        assert_eq!(response.status, 404);
        assert_eq!(response.content_type, "application/json");
        // `ClientError::http` is what the web and native fetch paths call for
        // every status >= 400. The JSON body is kept on the Http variant.
        // Display reads the `error` field. Content-Type is not stored.
        ClientError::http(response.status, response.body, None)
    }

    #[test]
    fn http_404_json_not_found_is_the_http_variant() {
        let response = ApiNotFound {
            status: 404,
            content_type: "application/json",
            body: r#"{"error":"Not found"}"#,
        };
        let err = map_not_found(response);

        match &err {
            ClientError::Http {
                status,
                body,
                retry_after,
            } => {
                assert_eq!(*status, 404);
                assert_eq!(body, r#"{"error":"Not found"}"#);
                assert_eq!(*retry_after, None);
            }
            ClientError::Network(message) => {
                panic!("404 JSON must not become Network: {message}")
            }
            ClientError::Deserialize(message) => {
                panic!("404 JSON must not become Deserialize: {message}")
            }
        }

        assert_eq!(err.http_status(), Some(404));
        assert_eq!(err.http_body(), Some(r#"{"error":"Not found"}"#));
        assert_eq!(err.to_string(), "HTTP error 404: Not found");
        assert!(!err.to_string().contains('<'));
        assert!(!err.to_string().contains('\u{2014}'));

        // The old miss was the site HTML page, which has no JSON `error` field.
        let html_page = ClientError::http(
            404,
            "<!DOCTYPE html><html><title>Not found</title></html>",
            None,
        );
        assert_eq!(html_page.to_string(), "HTTP error: 404");
        assert_ne!(html_page.to_string(), err.to_string());

        // PR 377 also answers a wrong method with JSON, not an HTML page.
        let method_not_allowed = ClientError::http(405, r#"{"error":"Method not allowed"}"#, None);
        match &method_not_allowed {
            ClientError::Http {
                status,
                body,
                retry_after,
            } => {
                assert_eq!(*status, 405);
                assert_eq!(body, r#"{"error":"Method not allowed"}"#);
                assert_eq!(*retry_after, None);
            }
            ClientError::Network(message) => {
                panic!("405 JSON must not become Network: {message}")
            }
            ClientError::Deserialize(message) => {
                panic!("405 JSON must not become Deserialize: {message}")
            }
        }
        assert_eq!(
            method_not_allowed.to_string(),
            "HTTP error 405: Method not allowed"
        );
    }

    /// Chromium's `ClientError::Network` text for a server that cannot be
    /// reached: the `JsValue` debug string with the whole wasm stack and
    /// asset URLs, over a thousand characters with no spaces in the frames.
    fn chromium_failed_fetch() -> String {
        let frame = "    at imports.wbg.__wbg_fetch_f1856afdb49415d1 \
                     (http://127.0.0.1:3815/assets/scuffed-app-dxh4f1e2d3c4b5a6.js:1:61234)\n";
        format!(
            "JsValue(TypeError: Failed to fetch\nTypeError: Failed to fetch\n{})",
            frame.repeat(12)
        )
    }

    /// QA 706 FIX5: a network failure reads as one short plain sentence,
    /// never the raw `JsValue` or its stack.
    #[test]
    fn a_network_failure_reads_as_one_short_sentence() {
        let raw = chromium_failed_fetch();
        assert!(raw.len() > 1000, "{}", raw.len());
        let err = ClientError::Network(raw.clone());
        let shown = user_message(&err);
        assert_eq!(
            shown,
            "Could not reach the server. Check your connection and try again."
        );
        assert_eq!(shown, NETWORK_ERROR_MESSAGE);
        assert!(shown.len() < 80, "{shown}");
        for leak in [
            "JsValue",
            "TypeError",
            "Failed to fetch",
            "http://",
            ".js:",
            "\n",
            "Network error",
            "\u{2014}",
        ] {
            assert!(!shown.contains(leak), "{leak} in {shown}");
        }
        // The raw detail is still on the error, for the console.
        assert!(err.to_string().contains("Failed to fetch"));

        // Firefox's short text, the other transport failures and an empty
        // text read the same.
        for raw in [
            "JsValue(TypeError: NetworkError when attempting to fetch resource.)",
            "No window",
            "Response cast failed",
            "",
        ] {
            assert_eq!(
                user_message(&ClientError::Network(raw.into())),
                NETWORK_ERROR_MESSAGE
            );
        }
    }

    /// A server answer keeps its reason; only the network case is replaced.
    #[test]
    fn a_server_answer_keeps_its_reason() {
        let http = ClientError::http(400, r#"{"error":"name too long"}"#, None);
        assert_eq!(user_message(&http), "HTTP error 400: name too long");
        assert_eq!(user_message(&http), http.to_string());
        let parse = ClientError::Deserialize("expected value".into());
        assert_eq!(user_message(&parse), parse.to_string());
    }

    /// The settings Save bar builds its failure line from [`user_message`]
    /// and logs the raw error to the console beside it.
    #[test]
    fn the_settings_save_bar_uses_the_short_message() {
        use crate::components::SaveState;
        let err = ClientError::Network(chromium_failed_fetch());
        assert_eq!(
            SaveState::failed("Could not save settings", user_message(&err)).text(),
            "Could not save settings. Could not reach the server. Check your connection and try again."
        );

        let prod = include_str!("pages/admin/settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("prod part");
        assert!(prod.contains(
            "crate::util::log_client_error(\"settings save\", &e);\n                    save_state.set(crate::components::SaveState::failed(\n                        \"Could not save settings\",\n                        crate::client_error::user_message(&e),\n                    ));"
        ));
    }
}
