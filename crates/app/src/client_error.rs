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

/// What a person sees when the server answered with something this page
/// could not read (an old cached page after a deploy, a proxy login page).
pub const UNREADABLE_ANSWER_MESSAGE: &str =
    "The server's answer could not be read. Reload the page and try again.";

/// The text a page shows for a failed request. This is the one place a
/// [`ClientError`] becomes page text: toasts ([`with_lead`],
/// `Toast::error_from`), save lines (`SaveState::failed`) and load lines
/// all go through it.
///
/// A network failure is always [`NETWORK_ERROR_MESSAGE`]: the raw text on
/// [`ClientError::Network`] is the browser's `JsValue` debug string, which
/// Chromium fills with the whole wasm stack trace (QA 706 FIX5: 1,106
/// characters in the settings Save bar). A body the client could not parse
/// is [`UNREADABLE_ANSWER_MESSAGE`]: serde's text names fields, lines and
/// columns, which mean nothing on the page. That detail is for the console
/// only ([`crate::util::log_client_error`]). A server answer keeps its reason.
pub fn user_message(err: &ClientError) -> String {
    match err {
        ClientError::Network(_) => NETWORK_ERROR_MESSAGE.to_string(),
        ClientError::Deserialize(_) => UNREADABLE_ANSWER_MESSAGE.to_string(),
        ClientError::Http { .. } => err.to_string(),
    }
}

/// [`user_message`] for a load that keeps its failure as text: the raw
/// error goes to the console first ([`crate::util::log_client_error`]), so
/// the detail the page no longer shows is still there for whoever debugs it.
pub fn logged_message(what: &str, err: &ClientError) -> String {
    crate::util::log_client_error(what, err);
    user_message(err)
}

/// `lead` and [`user_message`] as one line: "Failed to save game: Could not
/// reach the server. ..." A lead that already ends a sentence ("Could not
/// delete the article.") is followed by a space, any other by ": ".
pub fn with_lead(lead: &str, err: &ClientError) -> String {
    let message = user_message(err);
    let lead = lead.trim_end();
    if lead.is_empty() {
        message
    } else if lead.ends_with(['.', '!', '?', ':']) {
        format!("{lead} {message}")
    } else {
        format!("{lead}: {message}")
    }
}

#[cfg(test)]
mod tests {
    use super::{NETWORK_ERROR_MESSAGE, UNREADABLE_ANSWER_MESSAGE, user_message, with_lead};
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

    /// A server answer keeps its reason; the network and unreadable-answer
    /// cases are replaced.
    #[test]
    fn a_server_answer_keeps_its_reason() {
        let http = ClientError::http(400, r#"{"error":"name too long"}"#, None);
        assert_eq!(user_message(&http), "HTTP error 400: name too long");
        assert_eq!(user_message(&http), http.to_string());
        // Issue 726: serde's text (field names, line and column) is console
        // detail, not page text. The raw text stays on the error.
        let parse =
            ClientError::Deserialize("missing field `display_name` at line 1 column 212".into());
        assert_eq!(user_message(&parse), UNREADABLE_ANSWER_MESSAGE);
        assert!(parse.to_string().contains("missing field"));
        for leak in ["missing field", "line 1", "column", "`", "\u{2014}"] {
            assert!(!user_message(&parse).contains(leak), "{leak}");
        }
        assert!(UNREADABLE_ANSWER_MESSAGE.len() < 80);
    }

    /// A lead and the short message read as one line, with one separator.
    #[test]
    fn a_lead_joins_the_short_message() {
        let net = ClientError::Network(chromium_failed_fetch());
        assert_eq!(
            with_lead("Failed to save game", &net),
            "Failed to save game: Could not reach the server. Check your connection and try again."
        );
        assert_eq!(
            with_lead("Could not delete the article.", &net),
            "Could not delete the article. Could not reach the server. Check your connection and try again."
        );
        let http = ClientError::http(409, r#"{"error":"name taken"}"#, None);
        assert_eq!(
            with_lead("Failed to save team", &http),
            "Failed to save team: HTTP error 409: name taken"
        );
        assert_eq!(with_lead("", &net), NETWORK_ERROR_MESSAGE);
    }

    /// Issue 726: a toast for a failed request never carries the browser's
    /// raw error. The settings Save toast is the one QA saw at 1,106
    /// characters.
    #[test]
    fn the_settings_save_toast_is_short() {
        use crate::components::{Toast, toast::ToastLevel};
        let toast = Toast::error_from(
            "Failed to save settings",
            &ClientError::Network(chromium_failed_fetch()),
        );
        assert!(toast.level == ToastLevel::Error);
        assert_eq!(
            toast.message,
            "Failed to save settings: Could not reach the server. Check your connection and try again."
        );
        assert!(toast.message.len() < 100, "{}", toast.message);
        for leak in ["JsValue", "TypeError", "Failed to fetch", "http://", ".js:"] {
            assert!(!toast.message.contains(leak), "{leak}");
        }

        let prod = include_str!("pages/admin/settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("prod part");
        assert!(prod.contains("toast.show(Toast::error_from(\"Failed to save settings\", &e));"));
        assert!(!prod.contains("Failed to save settings: {e}"));
    }

    /// The settings Save bar builds its failure line from the error itself
    /// (`SaveState::failed` maps it through [`user_message`]) and logs the
    /// raw error to the console beside it.
    #[test]
    fn the_settings_save_bar_uses_the_short_message() {
        use crate::components::SaveState;
        let err = ClientError::Network(chromium_failed_fetch());
        assert_eq!(
            SaveState::failed("Could not save settings", &err).text(),
            "Could not save settings. Could not reach the server. Check your connection and try again."
        );

        let prod = include_str!("pages/admin/settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("prod part");
        assert!(prod.contains(
            "crate::util::log_client_error(\"settings save\", &e);\n                    save_state.set(crate::components::SaveState::failed(\n                        \"Could not save settings\",\n                        &e,\n                    ));"
        ));
    }

    /// Display text of an error binding the scan looks for: inline format
    /// arguments and `.to_string()` on the names the crate gives errors.
    const SHOWN: [&str; 7] = [
        "{e}",
        "{err}",
        "{error}",
        "{other}",
        "{e:?}",
        "{err:?}",
        "{error:?}",
    ];
    const NAMES: [&str; 4] = ["e", "err", "error", "other"];

    /// Lines that format an error binding which is not a [`ClientError`], or
    /// that keep it off the page. `(file, part of the line, why)`. Every
    /// entry must still match, so the list cannot go stale.
    const NOT_PAGE_TEXT: &[(&str, &str, &str)] = &[
        (
            "util.rs",
            "is_cancelled_fetch_text(&err.to_string()",
            "abort check, not shown",
        ),
        (
            "util.rs",
            "console::warn_1(&format!(\"{what}: {err}\")",
            "the console log",
        ),
        (
            "components/chat/chat_widget.rs",
            "serde_json::to_string(event).map_err(|e| e.to_string())",
            "serde error",
        ),
        (
            "components/chat/chat_widget.rs",
            "tracing::info!(\"Gift wrap decrypt skipped: {e}\")",
            "log",
        ),
        (
            "components/dm/types.rs",
            "format!(\"Invalid npub: {e}\")",
            "bech32 error",
        ),
        (
            "pages/apply.rs",
            "other => (PillTone::Neutral, other.to_string())",
            "status name",
        ),
        (
            "pages/scrims.rs",
            "other => Some((other.to_string(), PillTone::Neutral))",
            "status name",
        ),
        (
            "pages/identity.rs",
            "getPublicKey call failed: {e:?}",
            "NIP-07 extension error",
        ),
        (
            "pages/identity.rs",
            "getPublicKey rejected: {e:?}",
            "NIP-07 extension error",
        ),
        (
            "pages/identity.rs",
            "signEvent call failed: {e:?}",
            "NIP-07 extension error",
        ),
        (
            "pages/identity.rs",
            "signEvent rejected: {e:?}",
            "NIP-07 extension error",
        ),
        (
            "pages/identity.rs",
            "format!(\"Parse error: {e}\")",
            "serde_wasm_bindgen error",
        ),
        (
            "pages/login.rs",
            "signEvent call failed: {e:?}",
            "NIP-07 extension error",
        ),
        (
            "pages/login.rs",
            "signEvent rejected: {e:?}",
            "NIP-07 extension error",
        ),
        (
            "pages/login.rs",
            "format!(\"Parse error: {e}\")",
            "serde_wasm_bindgen error",
        ),
        (
            "pages/stat_reports.rs",
            "serde_json::from_value(value.clone()).map_err(|err| err.to_string())",
            "serde error",
        ),
        (
            "pages/stat_reports.rs",
            "serde_json::from_value(report.clone()).map_err(|err| err.to_string())",
            "serde error",
        ),
        (
            "pages/stat_reports.rs",
            "serde_json::from_value(report).map_err(|err| err.to_string())",
            "serde error",
        ),
        (
            "pages/stats_tokens.rs",
            "form_error.set(error.to_string());",
            "label check copy",
        ),
        (
            "pages/admin/articles.rs",
            "format!(\"Upload failed: HTTP {other}\")",
            "status code",
        ),
        (
            "pages/admin/members.rs",
            "format!(\"Upload failed: HTTP {other}\")",
            "status code",
        ),
        (
            "pages/admin/members.rs",
            "err.to_string().contains(OFFICER_RANK_ERROR)",
            "match, not shown",
        ),
        (
            "pages/admin/moderation.rs",
            "other.to_string().trim_matches",
            "JSON value",
        ),
        (
            "pages/admin/brand_settings.rs",
            "serde_json::from_str(&json).map_err(|err| err.to_string())",
            "test fixture serde error",
        ),
        (
            "pages/admin/patch_notes.rs",
            "Hero changes JSON is invalid: {e}",
            "serde error on typed JSON",
        ),
        (
            "pages/changelog/data.rs",
            "panic!(\"test source: {e}\")",
            "panic",
        ),
        ("pages/changelog/parse.rs", "changelog: {e}", "toml error"),
        (
            "pages/strategy/editor.rs",
            "tracing::error!(\"Failed to save strategy: {e}\")",
            "log",
        ),
        (
            "state/nostr.rs",
            "warn_console(&format!(\"Failed to parse relay message: {err}\"))",
            "console",
        ),
        (
            "state/nostr.rs",
            "tracing::error!(\"NIP-42 auth failed: {e}\")",
            "log",
        ),
        (
            "state/nostr.rs",
            "serde_json::to_string(&body).map_err(|e| e.to_string())",
            "serde error",
        ),
        (
            "state/nostr.rs",
            "Request::new_with_str_and_init(endpoint, &opts).map_err(|e| format!(\"{e:?}\"))",
            "web_sys fetch, logged",
        ),
        (
            "state/nostr.rs",
            ".map_err(|e| format!(\"{e:?}\"))?;",
            "web_sys fetch, logged",
        ),
        (
            "state/nostr.rs",
            "resp.json().map_err(|e| format!(\"{e:?}\"))",
            "web_sys fetch, logged",
        ),
        (
            "state/nostr.rs",
            "serde_wasm_bindgen::from_value(json).map_err(|e| e.to_string())",
            "serde error",
        ),
        (
            "state/nostr.rs",
            ".map_err(|e| e.to_string())?;",
            "serde error",
        ),
        (
            "state/site_settings.rs",
            "ignored invalid #sc-settings JSON: {err}",
            "log",
        ),
        (
            "state/site_settings.rs",
            "serde_json::from_str(&json).map_err(|err| err.to_string())",
            "test fixture serde error",
        ),
    ];

    fn shows_an_error_binding(line: &str) -> bool {
        if SHOWN.iter().any(|shown| line.contains(shown)) {
            return true;
        }
        NAMES.iter().any(|name| {
            let call = format!("{name}.to_string()");
            line.match_indices(&call).any(|(at, _)| {
                !line[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
            })
        })
    }

    /// The code before a file's test module.
    fn prod_part(text: &str) -> &str {
        let mut from = 0;
        while let Some(at) = text[from..].find("#[cfg(test)]") {
            let at = from + at;
            let rest = text[at + "#[cfg(test)]".len()..].trim_start();
            let rest = rest
                .strip_prefix("pub(crate) ")
                .or_else(|| rest.strip_prefix("pub "))
                .unwrap_or(rest);
            if rest.starts_with("mod ") {
                return &text[..at];
            }
            from = at + 1;
        }
        text
    }

    fn collect(
        src: &std::path::Path,
        dir: &std::path::Path,
        hits: &mut Vec<String>,
        used: &mut [bool],
    ) {
        let entries = std::fs::read_dir(dir).unwrap_or_else(|why| panic!("read {dir:?}: {why}"));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect(src, &path, hits, used);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(src)
                .expect("relative")
                .to_string_lossy()
                .replace('\\', "/");
            // This file is the one place a ClientError becomes page text.
            if rel == "client_error.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read rs");
            for (idx, line) in prod_part(&text).lines().enumerate() {
                if !shows_an_error_binding(line) {
                    continue;
                }
                match NOT_PAGE_TEXT
                    .iter()
                    .position(|(file, part, _)| *file == rel && line.contains(part))
                {
                    Some(at) => used[at] = true,
                    None => hits.push(format!("{rel}:{}: {}", idx + 1, line.trim())),
                }
            }
        }
    }

    /// Issues 726 and 728: every toast and inline line that shows a
    /// [`ClientError`] goes through [`user_message`] (directly, or via
    /// [`with_lead`], `Toast::error_from`, `SaveState::failed`,
    /// [`super::logged_message`] or the `ApiResource` error). Formatting an
    /// error binding with `{e}` or `.to_string()` anywhere else in the app is
    /// refused, so the raw `JsValue` stack cannot come back. A binding that
    /// is not a ClientError goes in [`NOT_PAGE_TEXT`] with its reason.
    #[test]
    fn no_client_error_is_formatted_into_page_text() {
        let src = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut hits = Vec::new();
        let mut used = vec![false; NOT_PAGE_TEXT.len()];
        collect(src, src, &mut hits, &mut used);
        assert!(
            hits.is_empty(),
            "format the error with crate::client_error::user_message (or with_lead, \
             Toast::error_from, SaveState::failed), not its Display:\n{}",
            hits.join("\n")
        );
        let stale: Vec<_> = NOT_PAGE_TEXT
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|((file, part, _), _)| format!("{file}: {part}"))
            .collect();
        assert!(stale.is_empty(), "stale entries:\n{}", stale.join("\n"));

        // The scan itself: these shapes are what it refuses.
        for bad in [
            "toast.show(Toast::error(format!(\"Failed to save game: {e}\")));",
            "toast.show(Toast::error(e.to_string()));",
            "p { class: \"fetch-error\", \"Couldn't load totals. {err}\" }",
            ".map_err(|e| e.to_string())",
            "other => DmLoadFailure::Other(other.to_string()),",
        ] {
            assert!(shows_an_error_binding(bad), "{bad}");
        }
        for fine in [
            "toast.show(Toast::error_from(\"Failed to save game\", &e));",
            "p { class: \"fetch-error\", \"Couldn't load totals. {reason}\" }",
            "value.to_string()",
            "self.e.to_string()",
        ] {
            assert!(!shows_an_error_binding(fine), "{fine}");
        }
    }
}
