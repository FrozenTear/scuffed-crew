//! How a [`scuffed_api_client::ClientError`] reads on the page, and pins how
//! an API 404 turns into a variant and a user-facing message.
//!
//! Unknown `/api/*` paths respond with HTTP 404, `Content-Type: application/json`,
//! and `{"error":"Not found"}` instead of the site HTML page.

use scuffed_api_client::ClientError;

/// The first sentence of [`NETWORK_ERROR_MESSAGE`], for a line that adds
/// its own next step ("Check your connection, then ...").
pub const UNREACHABLE_SENTENCE: &str = "Could not reach the server.";

/// What a person sees when a request never got an answer from the server
/// (offline, the server is down, a blocked request).
pub const NETWORK_ERROR_MESSAGE: &str =
    "Could not reach the server. Check your connection and try again.";

/// What a person sees when the server answered with something this page
/// could not read (an old cached page after a deploy, a proxy login page).
/// It never invites a blind retry: after a write the server may well have
/// done it (the answer was a 2xx), so the reload comes first, and it shows
/// whether the change is there. For a load the reload is the fix as well.
pub const UNREADABLE_ANSWER_MESSAGE: &str =
    "The server's answer could not be read. Reload to see the current state.";

/// What a person sees for a 5xx with no reason in the body: the proxy in
/// front of a stopped or restarting site server answers a bare 502, 503 or
/// 504 (Vision on 748, issue 728's stop-the-server step).
pub const SERVER_TROUBLE_MESSAGE: &str = "The server had a problem. Try again in a moment.";

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
/// only ([`crate::util::log_client_error`]). A server answer keeps its
/// reason, with long dashes made plain ([`plain_punctuation`]). A 5xx with no
/// reason is [`SERVER_TROUBLE_MESSAGE`]. A 4xx with no reason keeps
/// "HTTP error: NNN": `is_http_status` reads 401, 403 and 404 back from this
/// text to pick the sign-in wall and the missing views.
///
/// Pure: it logs nothing. The helpers that put the text on the page
/// ([`with_lead`], [`logged_message`], `SaveState::failed`, ...) log.
pub fn user_message(err: &ClientError) -> String {
    match err {
        ClientError::Network(_) => NETWORK_ERROR_MESSAGE.to_string(),
        ClientError::Deserialize(_) => UNREADABLE_ANSWER_MESSAGE.to_string(),
        ClientError::Http { status, body, .. } => {
            if has_server_reason(*status, body) {
                plain_punctuation(&err.to_string())
            } else if *status >= 500 {
                SERVER_TROUBLE_MESSAGE.to_string()
            } else {
                format!("HTTP error: {status}")
            }
        }
    }
}

/// The body carries something worth showing: the rate-limit copy for a 429,
/// or a non-blank JSON `error` field.
fn has_server_reason(status: u16, body: &str) -> bool {
    scuffed_types::too_many_requests_message(status, body).is_some()
        || serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error")?.as_str().map(|s| !s.trim().is_empty()))
            .unwrap_or(false)
}

/// A dash used as a sentence break in a server reason becomes a full stop
/// and the next word starts a sentence: "already linked \u{2014} remove it
/// first" reads "already linked. Remove it first". The site copy uses no
/// long dashes.
///
/// Which dashes break (Vision nit 2, round 4 on 748):
/// - An em dash (U+2014) always does, spaced or not: it is never a range or
///   a compound, and "linked\u{2014}remove" is the closed style of the same
///   break.
/// - An en dash (U+2013) or a hyphen breaks only with whitespace on both
///   sides ("a \u{2013} b", "a - b"). An unspaced en dash is a range: the
///   server's "username must be 1\u{2013}32 characters" reads "1-32", a plain
///   hyphen, so the page shows no long dash and never "1. 32". An unspaced
///   hyphen ("1-32", "re-link", "-5") is kept as it is.
/// - A long dash with nothing after it ends the sentence ("ends \u{2014}"
///   reads "ends."). A hyphen at the end is kept: the server's "letters,
///   digits, _ and -" names the hyphen itself.
pub fn plain_punctuation(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut capitalize_next = false;
    for (at, &ch) in chars.iter().enumerate() {
        let mut ch = ch;
        if matches!(ch, '\u{2014}' | '\u{2013}' | '-') {
            let space_before = at > 0 && chars[at - 1].is_whitespace();
            let space_after = chars.get(at + 1).is_some_and(|c| c.is_whitespace());
            let nothing_after = chars[at + 1..].iter().all(|c| c.is_whitespace());
            let long = ch != '-';
            let breaks = ch == '\u{2014}'
                || (space_before && space_after)
                || (long && (nothing_after || (at == 0 && space_after)));
            if breaks {
                if !capitalize_next {
                    while out.ends_with(char::is_whitespace) {
                        out.pop();
                    }
                    if !out.ends_with(['.', '!', '?', ':']) && !out.is_empty() {
                        out.push('.');
                    }
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    capitalize_next = true;
                }
                continue;
            }
            if ch == '\u{2013}' {
                ch = '-';
            }
        }
        if capitalize_next {
            if ch.is_whitespace() {
                continue;
            }
            out.extend(ch.to_uppercase());
            capitalize_next = false;
            continue;
        }
        out.push(ch);
    }
    out.trim_end().to_string()
}

/// [`user_message`] for a surface that names its own statuses in its own
/// words and never shows a server reason (poll votes, forum posts, scrims,
/// feed posts): no answer and an unreadable answer read as everywhere else,
/// and any status the surface does not name is [`SERVER_TROUBLE_MESSAGE`].
/// Pure, like [`user_message`]: those surfaces log at the call.
pub fn message_without_reason(err: &ClientError) -> &'static str {
    match err {
        ClientError::Network(_) => NETWORK_ERROR_MESSAGE,
        ClientError::Deserialize(_) => UNREADABLE_ANSWER_MESSAGE,
        ClientError::Http { .. } => SERVER_TROUBLE_MESSAGE,
    }
}

/// [`user_message`] for a load that keeps its failure as text: the raw
/// error goes to the console first ([`crate::util::log_client_error`]), so
/// the detail the page no longer shows is still there for whoever debugs it.
/// `what` may be page text ending in a full stop: the console label drops
/// it ("Loading your choices failed: ...", never "failed.: ...").
pub fn logged_message(what: &str, err: &ClientError) -> String {
    crate::util::log_client_error(what, err);
    user_message(err)
}

/// `lead` and [`user_message`] as one line: "Failed to save game: Could not
/// reach the server. ..." (see [`join_lead`] for the separator). The raw
/// error goes to the console under `lead` (without its trailing `.` or
/// `:`), so a caller does not log it again (Vision nit 8 on 748).
pub fn with_lead(lead: &str, err: &ClientError) -> String {
    join_lead(lead, &logged_message(lead, err))
}

/// [`with_lead`] without the console line, for a toast beside a save line
/// that already logged the same error (`SaveState::failed`): one failure,
/// one console line (QA round 7 on 748).
pub fn with_lead_unlogged(lead: &str, err: &ClientError) -> String {
    join_lead(lead, &user_message(err))
}

/// `lead` and an already mapped `message` as one line. A lead that already
/// ends a sentence ("Could not delete the article.") is followed by a
/// space, a lead ending in ":" by a space, any other by ": ". Either part
/// may be blank. Never "..", ". ." or ": .": a message that is only
/// punctuation is dropped, and a lead's own trailing punctuation is kept.
pub fn join_lead(lead: &str, message: &str) -> String {
    let lead = lead.trim();
    let message = message.trim();
    let message = if message.chars().all(|c| matches!(c, '.' | ':' | ' ')) {
        ""
    } else {
        message
    };
    if lead.is_empty() {
        message.to_string()
    } else if message.is_empty() {
        lead.to_string()
    } else if lead.ends_with(['.', '!', '?', ':']) {
        format!("{lead} {message}")
    } else {
        format!("{lead}: {message}")
    }
}

/// What a failed load says under its own "Couldn't load ..." heading: the
/// mapped reason it kept, or a plain sentence when it kept none.
pub fn reason_or_fallback(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        UNKNOWN_FAILURE_MESSAGE.to_string()
    } else {
        reason.to_string()
    }
}

/// A failure with no reason kept (never from a request: those always map).
pub const UNKNOWN_FAILURE_MESSAGE: &str = "Something went wrong.";

/// `text` ending in a full stop (unless it already ends a sentence), so a
/// next sentence can follow it without "..". Blank stays blank.
pub fn as_sentence(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() || text.ends_with(['.', '!', '?']) {
        text.to_string()
    } else {
        format!("{}.", text.trim_end_matches(':'))
    }
}

/// `first` and `second` as two sentences: `first` gets a full stop when it
/// has none. Blank parts are skipped. Never "..".
pub fn two_sentences(first: &str, second: &str) -> String {
    let first = as_sentence(first);
    let second = second.trim();
    let second = if second.chars().all(|c| matches!(c, '.' | ':' | ' ')) {
        ""
    } else {
        second
    };
    match (first.is_empty(), second.is_empty()) {
        (true, _) => second.to_string(),
        (false, true) => first,
        (false, false) => format!("{first} {second}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NETWORK_ERROR_MESSAGE, SERVER_TROUBLE_MESSAGE, UNREACHABLE_SENTENCE,
        UNREADABLE_ANSWER_MESSAGE, as_sentence, join_lead, logged_message, message_without_reason,
        plain_punctuation, two_sentences, user_message, with_lead, with_lead_unlogged,
    };
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

    /// A server answer keeps its reason; the network, unreadable-answer and
    /// reason-less 5xx cases are replaced.
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

        // The toast logs the raw error (`Toast::error_from` via `with_lead`),
        // so the save line maps the same error without a second log (Vision
        // nit 8 on 748: one console line per failure, not three).
        let prod = include_str!("pages/admin/settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("prod part");
        assert!(prod.contains("toast.show(Toast::error_from(\"Failed to save settings\", &e));"));
        assert!(prod.contains(
            "save_state.set(crate::components::SaveState::failed_text(\n                        \"Could not save settings\",\n                        &crate::client_error::user_message(&e),\n                    ));"
        ));
        assert!(!prod.contains("crate::util::log_client_error(\"settings save\", &e);"));
    }

    /// Vision blocking 1 on 748: production is a bare proxy, so a stopped
    /// site server answers a 502, 503 or 504 with no JSON reason. That reads
    /// as one plain sentence, never "HTTP error: 502". A real reason in a
    /// 5xx body is kept.
    #[test]
    fn a_reasonless_5xx_reads_as_a_plain_sentence() {
        for status in [500, 502, 503, 504] {
            for body in [
                "",
                "<html><body>502 Bad Gateway</body></html>",
                "{}",
                r#"{"error":"  "}"#,
            ] {
                let err = ClientError::http(status, body, None);
                assert_eq!(
                    user_message(&err),
                    SERVER_TROUBLE_MESSAGE,
                    "{status} {body}"
                );
                assert!(!user_message(&err).contains("HTTP"), "{status}");
            }
        }
        assert_eq!(
            SERVER_TROUBLE_MESSAGE,
            "The server had a problem. Try again in a moment."
        );
        let real = ClientError::http(500, r#"{"error":"database error"}"#, None);
        assert_eq!(user_message(&real), "HTTP error 500: database error");
        let busy = ClientError::http(503, r#"{"error":"hero_icons_disabled"}"#, None);
        assert_eq!(user_message(&busy), "HTTP error 503: hero_icons_disabled");
        // A reason-less 4xx keeps its status: `is_http_status` reads 401,
        // 403 and 404 back from this text to pick the sign-in wall and the
        // missing views.
        for status in [400, 401, 403, 404, 409, 413] {
            let err = ClientError::http(status, "", None);
            assert_eq!(user_message(&err), format!("HTTP error: {status}"));
        }
        for status in [401, 403, 404] {
            let shown = user_message(&ClientError::http(status, "", None));
            assert!(crate::components::is_http_status(&shown, status), "{shown}");
            assert!(!crate::components::is_http_status(
                SERVER_TROUBLE_MESSAGE,
                status
            ));
        }
        // A 429 keeps the rate-limit copy with or without a body.
        assert_eq!(
            user_message(&ClientError::http(429, "", None)),
            "Too many requests. Try again later."
        );
        assert_eq!(message_without_reason(&real), SERVER_TROUBLE_MESSAGE);
        assert_eq!(
            message_without_reason(&ClientError::Network("x".into())),
            NETWORK_ERROR_MESSAGE
        );
        assert_eq!(
            message_without_reason(&ClientError::Deserialize("x".into())),
            UNREADABLE_ANSWER_MESSAGE
        );
        assert!(NETWORK_ERROR_MESSAGE.starts_with(UNREACHABLE_SENTENCE));
    }

    /// Vision nit 10 on 748: an unreadable answer after a write (a 2xx) may
    /// mean the change went through, so the copy never invites a blind
    /// retry; the reload shows the current state, for a load as well.
    #[test]
    fn an_unreadable_answer_never_invites_a_retry() {
        let text = user_message(&ClientError::Deserialize("expected value".into()));
        assert_eq!(
            text,
            "The server's answer could not be read. Reload to see the current state."
        );
        assert!(!text.to_lowercase().contains("try again"), "{text}");
        assert!(
            text.starts_with("The server's answer could not be read. Reload"),
            "{text}"
        );
    }

    /// Vision nit 12 on 748: a long dash in a server reason (the key import
    /// 409) reads as two plain sentences.
    #[test]
    fn a_long_dash_in_a_server_reason_is_made_plain() {
        let err = ClientError::http(
            409,
            r#"{"error":"This key is already linked \u2014 remove it from the other account first"}"#,
            None,
        );
        let shown = user_message(&err);
        assert_eq!(
            shown,
            "HTTP error 409: This key is already linked. Remove it from the other account first"
        );
        assert!(
            !shown.contains('\u{2014}') && !shown.contains('\u{2013}'),
            "{shown}"
        );
        assert_eq!(plain_punctuation("a \u{2014} b"), "a. B");
        assert_eq!(plain_punctuation("done. \u{2014} next"), "done. Next");
        assert_eq!(plain_punctuation("ends \u{2014}"), "ends.");
        assert_eq!(plain_punctuation("no dash"), "no dash");
    }

    /// Vision nit 2, round 4 on 748: only a spaced dash breaks a sentence.
    /// The setup and register 400 ("username must be 1\u{2013}32
    /// characters") keeps its range as a plain "1-32", never "1. 32". Setup
    /// reads it through `user_message`; the register form's own reason
    /// (`body_error_or` in `pages/login.rs`) through `plain_punctuation`,
    /// pinned below and in `login.rs`.
    #[test]
    fn the_username_length_reason_keeps_its_range() {
        let err = ClientError::http(
            400,
            r#"{"error":"username must be 1\u201332 characters"}"#,
            None,
        );
        let shown = user_message(&err);
        assert_eq!(shown, "HTTP error 400: username must be 1-32 characters");
        assert!(!shown.contains("1. 32"), "{shown}");
        assert_eq!(
            plain_punctuation("username must be 1\u{2013}32 characters"),
            "username must be 1-32 characters"
        );
        assert!(
            !shown.contains('\u{2014}') && !shown.contains('\u{2013}'),
            "{shown}"
        );
        // The register form's other reason names the hyphen itself, at the end.
        let err = ClientError::http(
            400,
            r#"{"error":"username may only contain letters, digits, _ and -"}"#,
            None,
        );
        assert_eq!(
            user_message(&err),
            "HTTP error 400: username may only contain letters, digits, _ and -"
        );
    }

    #[test]
    fn a_spaced_dash_breaks_the_sentence_and_an_unspaced_one_is_a_range() {
        // Spaced em dash and spaced en dash: two sentences.
        assert_eq!(plain_punctuation("a \u{2014} b"), "a. B");
        assert_eq!(plain_punctuation("a \u{2013} b"), "a. B");
        assert_eq!(plain_punctuation("a - b"), "a. B");
        // Any whitespace counts, not only a plain space.
        assert_eq!(plain_punctuation("a\u{a0}\u{2013}\tb"), "a. B");
        // Unspaced en dash: a range, made a plain hyphen.
        assert_eq!(plain_punctuation("a\u{2013}b"), "a-b");
        assert_eq!(plain_punctuation("1\u{2013}32"), "1-32");
        // Half-spaced en dash is not a break either.
        assert_eq!(plain_punctuation("1 \u{2013}32"), "1 -32");
        // Unspaced hyphen: a compound, a range or a sign, kept.
        assert_eq!(plain_punctuation("re-link 1-32 -5"), "re-link 1-32 -5");
        assert_eq!(plain_punctuation("_ and -"), "_ and -");
        // Unspaced em dash: still a break (never a range).
        assert_eq!(
            plain_punctuation("already linked\u{2014}remove it first"),
            "already linked. Remove it first"
        );
        // Trailing and leading long dashes end or open a sentence.
        assert_eq!(plain_punctuation("ends \u{2014}"), "ends.");
        assert_eq!(plain_punctuation("ends\u{2013}"), "ends.");
        assert_eq!(plain_punctuation("\u{2013} starts"), "Starts");
        // A run of spaced dashes is one break.
        assert_eq!(plain_punctuation("a \u{2014} \u{2013} b"), "a. B");
    }

    /// Vision nit 6 on 748: no join ever reads "..", ". ." or ": .".
    #[test]
    fn joins_never_double_their_punctuation() {
        let net = ClientError::Network("offline".into());
        let reasons = [
            NETWORK_ERROR_MESSAGE.to_string(),
            UNREADABLE_ANSWER_MESSAGE.to_string(),
            SERVER_TROUBLE_MESSAGE.to_string(),
            "HTTP error 409: last admin".to_string(),
            "HTTP error 409: last admin.".to_string(),
            "HTTP error: 404".to_string(),
            ".".to_string(),
            " ".to_string(),
            String::new(),
        ];
        let leads = [
            "Could not save",
            "Could not save.",
            "Could not save:",
            "Player03",
            "Could not save. ",
            "",
        ];
        for lead in leads {
            for reason in &reasons {
                for joined in [
                    join_lead(lead, reason),
                    two_sentences(lead, reason),
                    as_sentence(&join_lead(lead, reason)),
                    crate::components::SaveState::failed_text(lead, reason).text(),
                ] {
                    for bad in ["..", ". .", ": .", ":.", " :"] {
                        assert!(!joined.contains(bad), "{lead:?} + {reason:?} = {joined:?}");
                    }
                    assert_eq!(joined, joined.trim(), "{joined:?}");
                }
            }
            let shown = with_lead(lead, &net);
            assert!(!shown.contains(".."), "{shown}");
        }
        assert_eq!(join_lead("Could not save", "."), "Could not save");
        assert_eq!(two_sentences("Could not save", ""), "Could not save.");
        assert_eq!(
            two_sentences("Player03: HTTP error 409: last admin", "Use Delete all."),
            "Player03: HTTP error 409: last admin. Use Delete all."
        );
        assert_eq!(as_sentence("Failed:"), "Failed.");
    }

    /// QA round 7 on 748: a lead that ends in a full stop (or a colon) logs
    /// without it, through every logging helper, so the console never reads
    /// "failed.: ...". A lead with no trailing stop logs as it is. The page
    /// text keeps the lead's own sentence break.
    #[test]
    fn a_lead_ending_in_a_stop_logs_without_it() {
        use crate::components::{SaveState, Toast};
        let take = crate::util::take_logged;
        let _ = take();
        let err = ClientError::http(409, r#"{"error":"busy"}"#, None);
        let raw = err.to_string();

        let shown = logged_message("Loading your choices failed.", &err);
        assert_eq!(shown, user_message(&err));
        assert_eq!(take(), [format!("Loading your choices failed: {raw}")]);

        let shown = with_lead("Loading your choices failed.", &err);
        assert_eq!(
            shown,
            format!("Loading your choices failed. {}", user_message(&err))
        );
        assert_eq!(take(), [format!("Loading your choices failed: {raw}")]);

        let _ = SaveState::failed("Could not save.", &err);
        assert_eq!(take(), [format!("Could not save: {raw}")]);

        let _ = Toast::error_from("Could not delete the article.", &err);
        assert_eq!(take(), [format!("Could not delete the article: {raw}")]);

        for lead in [
            "Save failed:",
            " Save failed. ",
            "Save failed.:",
            "Save failed...",
        ] {
            let _ = logged_message(lead, &err);
            assert_eq!(take(), [format!("Save failed: {raw}")], "{lead:?}");
        }

        // No trailing stop: unchanged.
        let _ = logged_message("forum thread", &err);
        assert_eq!(take(), [format!("forum thread: {raw}")]);
        let _ = logged_message("/api/stats/member/m1/heroes", &err);
        assert_eq!(take(), [format!("/api/stats/member/m1/heroes: {raw}")]);
        crate::util::log_client_error("DM load", &err);
        assert_eq!(take(), [format!("DM load: {raw}")]);

        // A direct log with a page-text lead is labelled the same way.
        crate::util::log_client_error("Deleting your icons failed.", &err);
        let logged = take();
        assert_eq!(logged, [format!("Deleting your icons failed: {raw}")]);
        assert!(!logged[0].contains(".:"));
    }

    /// QA round 7 on 748: a save line and a toast for the same failure log
    /// it once. `SaveState::failed` logs, so a toast beside it in the same
    /// arm is `Toast::error_logged_elsewhere` (or `Toast::error` of mapped
    /// text), never a second logging helper. Any two logging calls within
    /// three lines of each other in production code are refused unless
    /// listed in [`TWO_LOGS_ALLOWED`] with a reason.
    #[test]
    fn a_failure_is_not_logged_by_two_helpers() {
        let src = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        // Test-only files, as found by the page text scan.
        let mut skip = Vec::new();
        {
            let mut scratch_hits = Vec::new();
            let mut scratch_used = vec![false; NOT_PAGE_TEXT.len()];
            collect(src, src, &mut scratch_hits, &mut scratch_used, &mut skip);
        }
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .expect("read dir")
                .map(|e| e.expect("entry").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(src, &mut files);
        let mut hits = Vec::new();
        let mut used = vec![false; TWO_LOGS_ALLOWED.len()];
        // Logging calls the walk reads below a file's first `#[cfg(test)]`,
        // as `file: enclosing fn: helper`.
        let mut after_seam = Vec::new();
        for path in files {
            if skip.contains(&path) {
                continue;
            }
            let rel = path
                .strip_prefix(src)
                .expect("relative")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).expect("read rs");
            // Out-of-line test module files are already in `skip`. The pin
            // and the pair search share one read (API nit 1 on round 9).
            let (seam_sites, pairs) = double_log_scan(&text);
            after_seam.extend(seam_sites.into_iter().map(|site| format!("{rel}: {site}")));
            for ((a, first), (b, second)) in pairs {
                match TWO_LOGS_ALLOWED
                    .iter()
                    .position(|(f, l1, l2, _)| *f == rel && *l1 == first && *l2 == second)
                {
                    Some(at) => used[at] = true,
                    None => hits.push(format!("{rel}:{a}: {first}\n{rel}:{b}: {second}")),
                }
            }
        }
        assert!(
            hits.is_empty(),
            "two logging helpers handle one error; make the second one non-logging \
             (Toast::error_logged_elsewhere, with_lead_unlogged, user_message):\n{}",
            hits.join("\n")
        );
        let stale: Vec<_> = TWO_LOGS_ALLOWED
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|((file, first, _, _), _)| format!("{file}: {first}"))
            .collect();
        assert!(stale.is_empty(), "stale entries:\n{}", stale.join("\n"));
        // Vision nit 1 on round 8: the walk reads production code past a
        // file's first `#[cfg(test)]` seam (it reads `prod_text`, not the
        // text before the first seam). These are the logging calls there
        // today, keyed by file, enclosing fn and helper so an unrelated
        // line shift does not break the pin (API nit 2 on round 9). Update
        // the list when a call is added, dropped or moved to another fn.
        assert_eq!(
            after_seam,
            [
                "pages/admin/brand_settings.rs: load_brand_settings: logged_message",
                "pages/admin/brand_settings.rs: BrandSettingsForm: Toast::error_from",
                "pages/login.rs: nostr_login_flow: with_lead",
                "state/relay.rs: load_relay_health: logged_message",
                "state/site_settings.rs: load_public_settings: logged_message",
            ],
            "logging calls read after a file's first #[cfg(test)]"
        );

        // The guard itself flags the QA round 7 shape and lets the fixed
        // one, a lone helper, and two far-apart logs through.
        let shape = "save_state.set(SaveState::failed(\"Could not save your profile\", &e));\n\
                     if keep {\n    toast.show(Toast::error_from(\"Save failed\", &e));\n}\n";
        assert_eq!(close_log_pairs(shape).len(), 1);
        let shape = "account_state.set(Some((id, SaveState::failed(\"x\", &e))));\n\
                     toast.show(Toast::error_from(\"Save failed\", &e));\n";
        assert_eq!(close_log_pairs(shape).len(), 1);
        let shape = "let m = crate::client_error::with_lead(\"x\", &e);\n\
                     crate::util::log_client_error(\"x\", &e);\n";
        assert_eq!(close_log_pairs(shape).len(), 1);
        let fixed = "save_state.set(SaveState::failed(\"Could not save your profile\", &e));\n\
                     if keep {\n    toast.show(Toast::error_logged_elsewhere(\"Save failed\", &e));\n}\n";
        assert!(close_log_pairs(fixed).is_empty());
        let far = "a(logged_message(\"x\", &e));\n\n\n\nb(logged_message(\"y\", &f));\n";
        assert!(close_log_pairs(far).is_empty());
        let def = "pub fn logged_message(what: &str, err: &ClientError) -> String {\n\
                   crate::util::log_client_error(what, err);\n";
        assert!(close_log_pairs(def).is_empty());

        // Vision nit 1 on round 7: a test seam inside production code does
        // not end the scan. The pair after the seam is found on its true
        // lines, and the pair inside the test module is not.
        let seamed = "#[cfg(test)]\nfn seam() {}\n\
                      fn save() {\n    s.set(SaveState::failed(\"x\", &e));\n    \
                      toast.show(Toast::error_from(\"y\", &e));\n}\n\
                      #[cfg(test)]\nmod tests {\n    fn t() {\n        \
                      a(logged_message(\"x\", &e));\n        \
                      b(logged_message(\"y\", &e));\n    }\n}\n";
        // API nit 1 on round 9: the walk's own read (`double_log_scan`)
        // finds that pair, so the pair search cannot fall back to the text
        // before the first seam while the pin still reads `prod_text`.
        let (seam_sites, pairs) = double_log_scan(seamed);
        assert_eq!(pairs.len(), 1, "{pairs:?}");
        assert_eq!((pairs[0].0.0, pairs[0].1.0), (4, 5));
        assert!(pairs[0].0.1.starts_with("s.set(SaveState::failed("));
        assert_eq!(
            seam_sites,
            ["save: SaveState::failed", "save: Toast::error_from"]
        );
        // API nit 2 on round 9: a line shift above the calls keeps the keys.
        let shifted = seamed.replacen("fn save()", "// moved down\n\nfn save()", 1);
        let (shifted_sites, shifted_pairs) = double_log_scan(&shifted);
        assert_eq!(shifted_sites, seam_sites);
        assert_eq!((shifted_pairs[0].0.0, shifted_pairs[0].1.0), (6, 7));
        // A call in another fn, or a nested fn, is keyed by its own fn.
        let nested = "#[cfg(test)]\nfn seam() {}\nfn outer() {\n    fn inner() {\n        \
                      a(logged_message(\"x\", &e));\n    }\n}\n\
                      async fn other() {\n    crate::util::log_client_error(\"y\", &e);\n}\n";
        assert_eq!(
            double_log_scan(nested).0,
            ["inner: logged_message", "other: log_client_error"]
        );
    }

    /// The double-log walk's read of one source file: `prod_text` is
    /// computed once and both results come from it. First, the logging
    /// calls below the file's first `#[cfg(test)]`, each as
    /// `enclosing fn: helper` (no line numbers, so an unrelated line shift
    /// keeps the key). Second, the close pairs of [`close_log_pairs`].
    #[allow(clippy::type_complexity)]
    fn double_log_scan(text: &str) -> (Vec<String>, Vec<((usize, String), (usize, String))>) {
        let prod = prod_text(text);
        let mut seam_sites = Vec::new();
        if let Some(seam) = text.find("#[cfg(test)]") {
            let seam_line = text[..seam].matches('\n').count() + 1;
            for (n, line) in log_calls(&prod) {
                if n > seam_line {
                    let helper = LOGGERS
                        .iter()
                        .find(|l| line.contains(*l))
                        .map_or("?", |l| l.trim_end_matches('('));
                    seam_sites.push(format!("{}: {helper}", enclosing_fn(&prod, n)));
                }
            }
        }
        (seam_sites, close_log_pairs(&prod))
    }

    /// Name of the nearest `fn` declared at or above line `n` (1-based) of
    /// `prod`, or `?` when there is none.
    fn enclosing_fn(prod: &str, n: usize) -> String {
        prod.lines()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .find_map(|line| {
                let at = line.find("fn ")?;
                let name: String = line[at + 3..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                (!name.is_empty()).then_some(name)
            })
            .unwrap_or_else(|| "?".to_string())
    }

    /// Production text of one source, read the way the page text scan reads
    /// it: comments and inline `#[cfg(test)] mod x { .. }` blocks are
    /// blanked in place (newlines kept, so line numbers stay true), and code
    /// after a `#[cfg(test)]` seam on a non-module item is still read.
    fn prod_text(text: &str) -> String {
        let mut lexed = lex(text);
        let _ = drop_test_modules(&mut lexed);
        lexed.code.iter().collect()
    }

    /// Logging calls (see [`a_failure_is_not_logged_by_two_helpers`]) that
    /// follow each other within three lines in `prod`, as line number and
    /// trimmed line. Definitions (`fn ` on the line) and comments are not
    /// calls.
    #[allow(clippy::type_complexity)]
    fn close_log_pairs(prod: &str) -> Vec<((usize, String), (usize, String))> {
        log_calls(prod)
            .windows(2)
            .filter(|pair| pair[1].0 - pair[0].0 <= 3)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect()
    }

    /// The logging helpers [`log_calls`] looks for.
    const LOGGERS: [&str; 5] = [
        "SaveState::failed(",
        "Toast::error_from(",
        "with_lead(",
        "logged_message(",
        "log_client_error(",
    ];

    /// Logging calls in `prod`, as line number and trimmed line.
    /// Definitions (`fn ` on the line) and comments are not calls.
    fn log_calls(prod: &str) -> Vec<(usize, String)> {
        prod.lines()
            .enumerate()
            .filter(|(_, line)| {
                let code = line.split("//").next().unwrap_or("");
                !code.contains("fn ") && LOGGERS.iter().any(|l| code.contains(l))
            })
            .map(|(n, line)| (n + 1, line.trim().to_string()))
            .collect()
    }

    /// Two logging calls close together that log two different failures
    /// (separate match arms), each with its reason.
    /// (file, first line, second line, reason). Empty: no production code
    /// needs one today.
    const TWO_LOGS_ALLOWED: &[(&str, &str, &str, &str)] = &[];

    /// Vision nit 8 and QA on 748: the helpers that put a request error on
    /// the page log the raw error once; `user_message` itself stays pure.
    #[test]
    fn the_page_text_helpers_log_the_raw_error_once() {
        use crate::components::{SaveState, Toast};
        let take = crate::util::take_logged;
        let _ = take();
        let parse =
            ClientError::Deserialize("missing field `display_name` at line 1 column 212".into());

        let _ = user_message(&parse);
        assert!(take().is_empty(), "user_message is pure");

        let _ = with_lead("Could not save the game.", &parse);
        assert_eq!(
            take(),
            [
                "Could not save the game: Deserialization error: missing field `display_name` at line 1 column 212"
            ]
        );

        let _ = Toast::error_from("Failed to save settings", &parse);
        assert_eq!(take().len(), 1);

        // QA on 748: the profile save with an unreadable 200 logs it.
        let line = SaveState::failed("Could not save your profile", &parse);
        assert_eq!(
            line.text(),
            format!("Could not save your profile. {UNREADABLE_ANSWER_MESSAGE}")
        );
        let logged = take();
        assert_eq!(logged.len(), 1, "{logged:?}");
        assert!(
            logged[0].contains("missing field `display_name`"),
            "{logged:?}"
        );
        let profile = include_str!("pages/profile_settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("prod part");
        // QA round 7 on 748: the profile save maps the error in one helper
        // (`profile_save_failure`), whose toast does not log again.
        assert!(profile.contains("SaveState::failed(\"Could not save your profile\", err),"));
        assert!(profile.contains("save_state.set(line);"));

        // The toast beside a save line maps the same text and logs nothing.
        let toast = Toast::error_logged_elsewhere("Save failed", &parse);
        assert_eq!(
            toast.message,
            format!("Save failed: {UNREADABLE_ANSWER_MESSAGE}")
        );
        assert!(take().is_empty(), "error_logged_elsewhere logs nothing");
        assert_eq!(
            with_lead_unlogged("Could not delete the article.", &parse),
            format!("Could not delete the article. {UNREADABLE_ANSWER_MESSAGE}")
        );
        assert!(take().is_empty(), "with_lead_unlogged logs nothing");

        // The DM load classifier logs every kind.
        let _ = crate::components::dm::classify_dm_client_error(&ClientError::http(403, "", None));
        assert_eq!(take().len(), 1);

        // An aborted fetch (the page is being left) is not logged.
        let _ = with_lead(
            "x",
            &ClientError::Network("JsValue(AbortError: The operation was aborted. )".into()),
        );
        assert!(take().is_empty());
    }

    /// The names the crate gives a bound error. A binding with one of these
    /// names that is not a [`ClientError`] goes in [`NOT_PAGE_TEXT`].
    const NAMES: [&str; 4] = ["e", "err", "error", "other"];

    /// Copy that blames the connection, or a fixed catch-all, written out by
    /// hand. It comes from [`user_message`] (or [`message_without_reason`])
    /// only, so a 403 or an unreadable answer is never called a connection
    /// problem (QA on 748). Lines that use one on purpose are in
    /// [`NOT_PAGE_TEXT`].
    const CATCH_ALL: [&str; 4] = [
        "Check your connection",
        "Something went wrong. Try again",
        "Invalid username or password",
        "Couldn't reach the server",
    ];

    /// Lines the scan flags that are not a [`ClientError`] shown on the
    /// page, or copy used on purpose. `(file, the exact trimmed line, why)`:
    /// an exact line, so a new line next to an old one is never absorbed.
    /// Every entry must still match, so the list cannot go stale.
    const NOT_PAGE_TEXT: &[(&str, &str, &str)] = &[
        (
            "components/chat/chat_widget.rs",
            "event_json: serde_json::to_string(event).map_err(|e| e.to_string())?,",
            "serde error",
        ),
        (
            "components/chat/chat_widget.rs",
            "\"Can't reach the chat relay. Check your connection and retry.\"",
            "relay WebSocket state, not a ClientError",
        ),
        (
            "components/chat/chat_widget.rs",
            "Err(e) => tracing::info!(\"Gift wrap decrypt skipped: {e}\"),",
            "log",
        ),
        (
            "components/dm/types.rs",
            "CheckedHrpstring::new::<Bech32>(trimmed).map_err(|e| format!(\"Invalid npub: {e}\"))?;",
            "bech32 error",
        ),
        (
            "pages/admin/articles.rs",
            "other => format!(\"Upload failed: HTTP {other}\"),",
            "status code (deferred, Vision nit 9)",
        ),
        (
            "pages/admin/brand_settings.rs",
            "return serde_json::from_str(&json).map_err(|err| err.to_string());",
            "test fixture serde error",
        ),
        (
            "pages/admin/members.rs",
            "if err.http_status() == Some(403) && err.to_string().contains(OFFICER_RANK_ERROR) {",
            "match, not shown",
        ),
        (
            "pages/admin/members.rs",
            "other => format!(\"Upload failed: HTTP {other}\"),",
            "status code (deferred, Vision nit 9)",
        ),
        (
            "pages/admin/moderation.rs",
            ".unwrap_or_else(|| other.to_string().trim_matches('\"').to_string()),",
            "JSON value",
        ),
        (
            "pages/admin/patch_notes.rs",
            "serde_json::from_str(trimmed).map_err(|e| format!(\"Hero changes JSON is invalid: {e}\"))",
            "serde error on typed JSON",
        ),
        (
            "pages/apply.rs",
            "other => (PillTone::Neutral, other.to_string()),",
            "status name",
        ),
        (
            "pages/changelog/data.rs",
            ".unwrap_or_else(|e| panic!(\"test source: {e}\"))",
            "panic",
        ),
        (
            "pages/identity.rs",
            ".map_err(|e| format!(\"getPublicKey call failed: {e:?}\"))?;",
            "NIP-07 extension error (deferred, Vision nit 9)",
        ),
        (
            "pages/identity.rs",
            ".map_err(|e| format!(\"getPublicKey rejected: {e:?}\"))?;",
            "NIP-07 extension error (deferred, Vision nit 9)",
        ),
        (
            "pages/identity.rs",
            ".map_err(|e| format!(\"signEvent call failed: {e:?}\"))?;",
            "NIP-07 extension error (deferred, Vision nit 9)",
        ),
        (
            "pages/identity.rs",
            ".map_err(|e| format!(\"signEvent rejected: {e:?}\"))?;",
            "NIP-07 extension error (deferred, Vision nit 9)",
        ),
        (
            "pages/identity.rs",
            "serde_wasm_bindgen::from_value(signed_event).map_err(|e| format!(\"Parse error: {e}\"))?;",
            "serde_wasm_bindgen error (deferred, Vision nit 9)",
        ),
        (
            "pages/link/mod.rs",
            "pub const SOMETHING_WENT_WRONG: &str = \"Something went wrong. Try again.\";",
            "link page copy for an unnamed status of an ObservedError, not a ClientError; no answer and an unreadable answer have their own lines",
        ),
        (
            "pages/login.rs",
            "const INVALID_CREDENTIALS: &str = \"Invalid username or password\";",
            "the 401 credential rejection only",
        ),
        (
            "pages/login.rs",
            ".map_err(|e| format!(\"signEvent call failed: {e:?}\"))?;",
            "NIP-07 extension error (deferred, Vision nit 9)",
        ),
        (
            "pages/login.rs",
            ".map_err(|e| format!(\"signEvent rejected: {e:?}\"))?;",
            "NIP-07 extension error (deferred, Vision nit 9)",
        ),
        (
            "pages/login.rs",
            "serde_wasm_bindgen::from_value(signed_event).map_err(|e| format!(\"Parse error: {e}\"))?;",
            "serde_wasm_bindgen error (deferred, Vision nit 9)",
        ),
        (
            "pages/scrims.rs",
            "other => Some((other.to_string(), PillTone::Neutral)),",
            "status name",
        ),
        (
            "pages/stat_reports.rs",
            "serde_json::from_value(value.clone()).map_err(|err| err.to_string())?;",
            "serde error",
        ),
        (
            "pages/stat_reports.rs",
            "serde_json::from_value(report.clone()).map_err(|err| err.to_string())?;",
            "serde error",
        ),
        (
            "pages/stat_reports.rs",
            "serde_json::from_value(report).map_err(|err| err.to_string())?;",
            "serde error",
        ),
        (
            "pages/stats_tokens.rs",
            "form_error.set(error.to_string());",
            "label check copy",
        ),
        (
            "pages/strategy/editor.rs",
            "tracing::error!(\"Failed to save strategy: {e}\");",
            "log",
        ),
        (
            "state/nostr.rs",
            "warn_console(&format!(\"Failed to parse relay message: {err}\"));",
            "console",
        ),
        (
            "state/nostr.rs",
            "tracing::error!(\"NIP-42 auth failed: {e}\");",
            "log",
        ),
        (
            "state/nostr.rs",
            "let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;",
            "serde error",
        ),
        (
            "state/nostr.rs",
            "let request = Request::new_with_str_and_init(endpoint, &opts).map_err(|e| format!(\"{e:?}\"))?;",
            "web_sys fetch, logged",
        ),
        (
            "state/nostr.rs",
            ".map_err(|e| format!(\"{e:?}\"))?;",
            "web_sys fetch, logged (1 of 3)",
        ),
        (
            "state/nostr.rs",
            ".map_err(|e| format!(\"{e:?}\"))?;",
            "web_sys fetch, logged (2 of 3)",
        ),
        (
            "state/nostr.rs",
            "let json = JsFuture::from(resp.json().map_err(|e| format!(\"{e:?}\"))?)",
            "web_sys fetch, logged",
        ),
        (
            "state/nostr.rs",
            ".map_err(|e| format!(\"{e:?}\"))?;",
            "web_sys fetch, logged (3 of 3)",
        ),
        (
            "state/nostr.rs",
            "serde_wasm_bindgen::from_value(json).map_err(|e| e.to_string())?;",
            "serde error",
        ),
        (
            "state/nostr.rs",
            ".map_err(|e| e.to_string())?;",
            "serde error",
        ),
        (
            "state/site_settings.rs",
            "tracing::warn!(\"ignored invalid #sc-settings JSON: {err}\");",
            "log",
        ),
        (
            "state/site_settings.rs",
            "Some(json) => serde_json::from_str(&json).map_err(|err| err.to_string()),",
            "test fixture serde error",
        ),
        (
            "training_crops_ui.rs",
            "format!(\"{LEAD} {UNREACHABLE_SENTENCE} Check your connection, then {BUTTON}\")",
            "no answer: the mapped first sentence plus this line's own next step (Vision nit 3)",
        ),
        (
            "util.rs",
            "is_cancelled_fetch_text(&err.to_string(), page_is_leaving())",
            "abort check, not shown",
        ),
        (
            "util.rs",
            "web_sys::console::warn_1(&format!(\"{what}: {err}\").into());",
            "the console log",
        ),
        (
            "util.rs",
            "LOGGED.with(|logged| logged.borrow_mut().push(format!(\"{what}: {err}\")));",
            "the console log's test seam",
        ),
    ];

    /// A Rust source as two same-length char vectors: `code` with comments
    /// blanked (strings kept), and `shape` with comments and the insides of
    /// string and char literals blanked (quotes kept), for brace and comma
    /// matching. Newlines are kept in both, so line numbers stay true.
    struct Lexed {
        code: Vec<char>,
        shape: Vec<char>,
    }

    fn lex(src: &str) -> Lexed {
        let chars: Vec<char> = src.chars().collect();
        let mut code = chars.clone();
        let mut shape = chars.clone();
        let blank = |v: &mut Vec<char>, at: usize| {
            if v[at] != '\n' {
                v[at] = ' ';
            }
        };
        let ident = |c: char| c.is_alphanumeric() || c == '_';
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if c == '/' && next == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    blank(&mut code, i);
                    blank(&mut shape, i);
                    i += 1;
                }
                continue;
            }
            if c == '/' && next == Some('*') {
                let mut depth = 0usize;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        for at in [i, i + 1] {
                            blank(&mut code, at);
                            blank(&mut shape, at);
                        }
                        i += 2;
                        continue;
                    }
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        for at in [i, i + 1] {
                            blank(&mut code, at);
                            blank(&mut shape, at);
                        }
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                        continue;
                    }
                    blank(&mut code, i);
                    blank(&mut shape, i);
                    i += 1;
                }
                continue;
            }
            // Raw strings: r"..", r#".."#, br#".."#.
            let prev_ident = i > 0 && ident(chars[i - 1]);
            let raw_at = match c {
                'r' if !prev_ident => Some(i + 1),
                'b' if !prev_ident && next == Some('r') => Some(i + 2),
                _ => None,
            };
            if let Some(mut at) = raw_at {
                let mut hashes = 0;
                while chars.get(at) == Some(&'#') {
                    hashes += 1;
                    at += 1;
                }
                if chars.get(at) == Some(&'"') {
                    let mut j = at + 1;
                    loop {
                        if j >= chars.len() {
                            break;
                        }
                        if chars[j] == '"'
                            && (0..hashes).all(|h| chars.get(j + 1 + h) == Some(&'#'))
                        {
                            break;
                        }
                        blank(&mut shape, j);
                        j += 1;
                    }
                    i = j + 1 + hashes;
                    continue;
                }
            }
            if c == '"' {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != '"' {
                    if chars[j] == '\\' {
                        blank(&mut shape, j);
                        j += 1;
                    }
                    if j < chars.len() {
                        blank(&mut shape, j);
                    }
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            if c == '\'' {
                // A char literal ('x', '\n', '\u{..}'), not a lifetime.
                let end = if next == Some('\\') {
                    (i + 2..chars.len()).find(|&j| chars[j] == '\'')
                } else if chars.get(i + 2) == Some(&'\'') {
                    Some(i + 2)
                } else {
                    None
                };
                if let Some(end) = end {
                    for at in i + 1..end {
                        blank(&mut shape, at);
                    }
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
        Lexed { code, shape }
    }

    /// The index after the bracket that closes the one at `open` (in `shape`).
    fn close_of(shape: &[char], open: usize) -> usize {
        let mut depth = 0i32;
        for (at, c) in shape.iter().enumerate().skip(open) {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return at + 1;
                    }
                }
                _ => {}
            }
        }
        shape.len()
    }

    fn starts_at(v: &[char], at: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(k, c)| v.get(at + k) == Some(&c))
    }

    /// Inline `#[cfg(test)] mod x { .. }` blocks are blanked (only the block:
    /// code after it is still scanned). Out-of-line `#[cfg(test)] mod x;`
    /// names are returned, so their files are skipped. Further attributes
    /// between `#[cfg(test)]` and the module (`#[allow(..)]`) and any
    /// visibility (`pub`, `pub(crate)`, `pub(super)`, `pub(in ..)`) are
    /// allowed. Test-only items that are not modules (a `#[cfg(test)]` fn,
    /// impl, use or const) are not dropped: they are read like production
    /// code.
    fn drop_test_modules(lexed: &mut Lexed) -> Vec<String> {
        let mut out_of_line = Vec::new();
        let marker = "#[cfg(test)]";
        let mut at = 0;
        while at < lexed.shape.len() {
            if !starts_at(&lexed.shape, at, marker) {
                at += 1;
                continue;
            }
            let mut j = at + marker.chars().count();
            let skip_ws = |j: &mut usize, v: &[char]| {
                while *j < v.len() && v[*j].is_whitespace() {
                    *j += 1;
                }
            };
            skip_ws(&mut j, &lexed.shape);
            // Further attributes: `#[allow(..)]`, `#[path = ".."]`.
            while starts_at(&lexed.shape, j, "#[") {
                j = close_of(&lexed.shape, j + 1);
                skip_ws(&mut j, &lexed.shape);
            }
            // Any visibility: `pub`, `pub(crate)`, `pub(super)`, `pub(in ..)`.
            if starts_at(&lexed.shape, j, "pub(") {
                j = close_of(&lexed.shape, j + 3);
                skip_ws(&mut j, &lexed.shape);
            } else if starts_at(&lexed.shape, j, "pub ") {
                j += 4;
                skip_ws(&mut j, &lexed.shape);
            }
            if !starts_at(&lexed.shape, j, "mod ") {
                at += 1;
                continue;
            }
            j += 4;
            let name_start = j;
            while j < lexed.shape.len()
                && (lexed.shape[j].is_alphanumeric() || lexed.shape[j] == '_')
            {
                j += 1;
            }
            let name: String = lexed.shape[name_start..j].iter().collect();
            skip_ws(&mut j, &lexed.shape);
            match lexed.shape.get(j) {
                Some('{') => {
                    let end = close_of(&lexed.shape, j);
                    for k in at..end {
                        if lexed.code[k] != '\n' {
                            lexed.code[k] = ' ';
                            lexed.shape[k] = ' ';
                        }
                    }
                    at = end;
                }
                Some(';') => {
                    out_of_line.push(name);
                    at = j + 1;
                }
                _ => at += 1,
            }
        }
        out_of_line
    }

    fn line_of(v: &[char], at: usize) -> usize {
        v[..at].iter().filter(|c| **c == '\n').count()
    }

    /// `name` is an error binding: `e`, `&e`, `*err`, `other.to_string()`.
    fn is_error_arg(arg: &str) -> bool {
        let arg = arg.trim().trim_start_matches(['&', '*']).trim();
        let arg = arg.strip_suffix(".to_string()").unwrap_or(arg);
        NAMES.contains(&arg)
    }

    /// `.to_string()` called on an error binding (not on `self.e`, `value`).
    fn to_string_on_name(line: &str) -> bool {
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

    /// `{e}`, `{err:?}`, `{error:#}`: an error binding inline in a format
    /// string, with any format spec.
    fn inline_name(line: &str) -> bool {
        NAMES.iter().any(|name| {
            line.contains(&format!("{{{name}}}")) || line.contains(&format!("{{{name}:"))
        })
    }

    /// Every flagged line of one source: `(line number, trimmed code line)`.
    /// Comments are not read. Test modules are skipped. A format macro whose
    /// arguments (on any line of the call) include an error binding is
    /// flagged on the argument's line.
    fn scan_source(text: &str) -> (Vec<(usize, String)>, Vec<String>) {
        let mut lexed = lex(text);
        let out_of_line = drop_test_modules(&mut lexed);
        let code: String = lexed.code.iter().collect();
        let lines: Vec<&str> = code.lines().collect();
        let mut hits = std::collections::BTreeSet::new();
        for (idx, line) in lines.iter().enumerate() {
            if inline_name(line)
                || to_string_on_name(line)
                || CATCH_ALL.iter().any(|copy| line.contains(copy))
            {
                hits.insert(idx);
            }
        }
        // Format-style macros: any argument after the template that is an
        // error binding (positional `{}` / `{0}` or named `x = e`).
        let shape = &lexed.shape;
        for at in 0..shape.len() {
            if shape[at] != '!' || shape.get(at + 1) != Some(&'(') {
                continue;
            }
            let name_start = (0..at)
                .rev()
                .take_while(|&p| shape[p].is_alphanumeric() || shape[p] == '_')
                .last()
                .unwrap_or(at);
            let name: String = shape[name_start..at].iter().collect();
            // Arguments before the template: none, or the writer.
            let leading = match name.as_str() {
                "format" | "format_args" | "panic" | "print" | "println" | "eprint"
                | "eprintln" => 1,
                "write" | "writeln" => 2,
                _ => continue,
            };
            let open = at + 1;
            let end = close_of(shape, open);
            let mut depth = 0i32;
            let mut from = open + 1;
            let mut index = 0;
            for (k, &c) in shape.iter().enumerate().take(end).skip(open + 1) {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' if depth > 0 => depth -= 1,
                    ',' | ')' if depth == 0 => {
                        let arg: String = lexed.code[from..k].iter().collect();
                        let value = arg.split_once('=').map_or(arg.as_str(), |(_, v)| v);
                        index += 1;
                        if index > leading && is_error_arg(value) {
                            let first = (from..k)
                                .find(|&p| !lexed.code[p].is_whitespace())
                                .unwrap_or(from);
                            hits.insert(line_of(&lexed.code, first));
                        }
                        from = k + 1;
                    }
                    _ => {}
                }
            }
        }
        let hits = hits
            .into_iter()
            .map(|idx| (idx + 1, lines.get(idx).map_or("", |l| l.trim()).to_string()))
            .collect();
        (hits, out_of_line)
    }

    fn collect(
        src: &std::path::Path,
        dir: &std::path::Path,
        hits: &mut Vec<String>,
        used: &mut [bool],
        skip: &mut Vec<std::path::PathBuf>,
    ) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|why| panic!("read {dir:?}: {why}"))
            .map(|entry| entry.expect("dir entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect(src, &path, hits, used, skip);
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
            if rel == "client_error.rs" || skip.contains(&path) {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read rs");
            let (found, test_mods) = scan_source(&text);
            // `mod x;` behind `#[cfg(test)]`: x.rs or x/mod.rs beside this
            // file (or in its own folder for a non-mod file).
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let base = if matches!(stem, "mod" | "main" | "lib") {
                path.parent().expect("parent").to_path_buf()
            } else {
                path.with_extension("")
            };
            for name in test_mods {
                skip.push(base.join(format!("{name}.rs")));
                skip.push(base.join(&name).join("mod.rs"));
            }
            // Each entry covers one occurrence: a second identical line in
            // the same file is a new hit until it gets its own entry.
            for (line_no, line) in found {
                match NOT_PAGE_TEXT
                    .iter()
                    .enumerate()
                    .position(|(at, (file, exact, _))| !used[at] && *file == rel && *exact == line)
                {
                    Some(at) => used[at] = true,
                    None => hits.push(format!(
                        "        (\"{rel}\", {line:?}, \"\"), // :{line_no}"
                    )),
                }
            }
        }
    }

    /// Issues 726 and 728, QA and Vision on 748: every toast and inline line
    /// that shows a [`ClientError`] goes through [`user_message`] (directly,
    /// or via [`with_lead`], `Toast::error_from`, `SaveState::failed`,
    /// [`super::logged_message`] or the `ApiResource` error). An error
    /// binding formatted any other way (inline `{e}` with any spec, a
    /// positional `{}` argument on any line of the call, `.to_string()`), or
    /// hand-written catch-all connection copy, anywhere else in the app is
    /// refused. A line that is not a ClientError goes in [`NOT_PAGE_TEXT`]
    /// with its reason.
    #[test]
    fn no_client_error_is_formatted_into_page_text() {
        let src = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut hits = Vec::new();
        let mut used = vec![false; NOT_PAGE_TEXT.len()];
        // Test-only files named from `main.rs` / `lib.rs` are found as the
        // scan goes; the folder walk is sorted, so collect twice to be sure
        // a test file listed before its parent is skipped too.
        let mut skip = Vec::new();
        {
            let mut scratch_hits = Vec::new();
            let mut scratch_used = vec![false; NOT_PAGE_TEXT.len()];
            collect(src, src, &mut scratch_hits, &mut scratch_used, &mut skip);
        }
        collect(src, src, &mut hits, &mut used, &mut skip);
        assert!(
            hits.is_empty(),
            "format the error with crate::client_error::user_message (or with_lead, \
             Toast::error_from, SaveState::failed), not its Display, and take \
             connection copy from the mapping; or add an exact line with its reason:\n{}",
            hits.join("\n")
        );
        let stale: Vec<_> = NOT_PAGE_TEXT
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|((file, line, _), _)| format!("{file}: {line}"))
            .collect();
        assert!(stale.is_empty(), "stale entries:\n{}", stale.join("\n"));
    }

    /// The scan itself: what it flags, what it lets through, and that it
    /// reads past comments, test modules and string contents correctly.
    #[test]
    fn the_scan_flags_these_shapes_and_skips_comments() {
        let flagged = |text: &str| -> Vec<usize> {
            scan_source(text).0.into_iter().map(|(n, _)| n).collect()
        };
        for bad in [
            "toast.show(Toast::error(format!(\"Failed to save game: {e}\")));",
            "toast.show(Toast::error(e.to_string()));",
            "p { class: \"fetch-error\", \"Couldn't load totals. {err}\" }",
            ".map_err(|e| e.to_string())",
            "other => DmLoadFailure::Other(other.to_string()),",
            "let s = format!(\"Failed: {}\", e);",
            "let s = format!(\"Failed: {0}\", &err);",
            "let s = format!(\"Failed: {why}\", why = error);",
            "let s = format!(\"Failed: {e:#}\");",
            "let s = format!(\"Failed: {err:?}\");",
            "message: \"Couldn't load. Check your connection and try again.\",",
            "let c = '\"'; let s = format!(\"{}\", e);",
        ] {
            assert_eq!(flagged(bad), vec![1], "{bad}");
        }
        // A call over several lines is flagged on the argument's line.
        assert_eq!(
            flagged("let s = format!(\n    \"Failed: {}\",\n    e,\n);"),
            vec![3]
        );
        for fine in [
            "toast.show(Toast::error_from(\"Failed to save game\", &e));",
            "p { class: \"fetch-error\", \"Couldn't load totals. {reason}\" }",
            "value.to_string()",
            "self.e.to_string()",
            "let n = format!(\"{} games\", count);",
            "let s = format!(\"{}\", e.http_status().unwrap_or(0));",
            "// toast.show(Toast::error(format!(\"{e}\")));",
            "/* Check your connection and try again. {err} */",
            "let s = \"a // not a comment {reason}\";",
            "let s = r#\"raw \"{why}\" string\"#;",
        ] {
            assert!(flagged(fine).is_empty(), "{fine}");
        }
        // Code after an out-of-line test module is still scanned, and the
        // module's name is reported so its file is skipped.
        let (hits, mods) =
            scan_source("#[cfg(test)]\nmod helpers;\nfn x() -> String { format!(\"{e}\") }\n");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].0, 3);
        assert_eq!(mods, ["helpers"]);
        // An inline test module is skipped, code after it is not.
        let (hits, _) = scan_source(
            "#[cfg(test)]\nmod tests {\n    fn t() -> String { format!(\"{e}\") }\n}\nfn after() -> String { format!(\"{err}\") }\n",
        );
        assert_eq!(
            hits,
            [(5, "fn after() -> String { format!(\"{err}\") }".to_string())]
        );
        // A brace inside a string does not end the test module early.
        let (hits, _) = scan_source(
            "#[cfg(test)]\nmod tests {\n    const S: &str = \"}\";\n    fn t() -> String { format!(\"{e}\") }\n}\n",
        );
        assert!(hits.is_empty(), "{hits:?}");
        // Vision nit 2 on round 8: an extra attribute between `#[cfg(test)]`
        // and the module (the components/ui/mod.rs shape) and a
        // `pub(super)` module (the bracket/single_elim.rs shape) are still
        // test modules; code after them is still scanned.
        let (hits, _) = scan_source(
            "#[cfg(test)]\n#[allow(unreachable_code)]\npub(crate) mod test_support {\n    fn t() -> String { format!(\"{e}\") }\n}\nfn after() -> String { format!(\"{err}\") }\n",
        );
        assert_eq!(
            hits,
            [(6, "fn after() -> String { format!(\"{err}\") }".to_string())]
        );
        let (hits, _) = scan_source(
            "#[cfg(test)]\npub(super) mod test_data {\n    fn t() -> String { format!(\"{e}\") }\n}\nfn after() -> String { format!(\"{err}\") }\n",
        );
        assert_eq!(
            hits,
            [(5, "fn after() -> String { format!(\"{err}\") }".to_string())]
        );
        let (hits, mods) = scan_source(
            "#[cfg(test)]\n#[path = \"x_tests.rs\"]\npub(in crate::pages) mod x_tests;\nfn x() -> String { format!(\"{e}\") }\n",
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].0, 4);
        assert_eq!(mods, ["x_tests"]);
        // A test-only item that is not a module is still read.
        let (hits, _) = scan_source(
            "#[cfg(test)]\n#[allow(dead_code)]\npub(crate) fn seam() -> String { format!(\"{e}\") }\n",
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].0, 3);
    }
}
