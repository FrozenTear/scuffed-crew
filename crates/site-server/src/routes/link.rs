//! Device-code sign-in for the stat tracker.
//!
//! `POST /api/link/start` and `POST /api/link/poll` are unauthenticated and do
//! not check `Origin` or `Sec-Fetch-Site`. The desktop app calls them with no
//! browser headers. The device shows `user_code` (shaped `XXXX-XXXX`) and
//! polls with `device_code`.
//! `POST /api/link/lookup`, `/approve`, and `/deny` require a signed-in session
//! (`OrgMember`), the same cookie or bearer session check as other mutations.
//! Lookup, approve, and deny reject a request with 403 `{"error":"bad_origin"}`
//! unless `Origin` matches a configured site origin (`ALLOWED_ORIGINS`, or
//! `REDIRECT_BASE_URL` when that list is unset). When `Origin` is absent,
//! `Sec-Fetch-Site: same-origin` is accepted instead. Missing both is rejected.
//! A present `Origin` that does not match is rejected even if `Sec-Fetch-Site`
//! says same-origin. `Origin: null` and a lookalike host or an `http` scheme
//! do not match an `https` site origin. These three routes are POST only, which
//! keeps the "no state-changing GET" rule intact. Session cookies stay
//! `SameSite=Lax`.
//!
//! Every `/api/link/*` response, success or error, sends `Cache-Control: no-store`.
//! Per-IP 429s on these routes, including poll, use [`crate::rate_limit::rate_limited_response`]:
//! `{"error":"rate_limited","retry_after":N}` with a matching `Retry-After` header.
//!
//! A pending code can be denied by any signed-in member. After approve, and
//! before the device collects the token, only that member can deny the code.
//! That deny revokes the daemon token. Another member's approve or deny is the
//! same `invalid code` error, and the token stays active. If the code expires
//! first, cleanup revokes the uncollected token and clears its handover secret.
//! A deny revokes the daemon token before it writes `denied`. If that revoke
//! fails, the code stays approved. Cleanup also revokes a denied code whose
//! token is still active. The sweep starts with the server and runs about
//! every 60 seconds.
//!
//! Codes are read from the JSON body only. A query string that carries one is
//! stripped before the trace layer logs the URI, and the request is rejected.

use std::net::SocketAddr;
use std::sync::OnceLock;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::uri::PathAndQuery;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rand::RngCore;
use zeroize::Zeroize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::queries::device_link::{
    DEVICE_LINK_INTERVAL_SECS, DEVICE_LINK_TTL_SECS, DeviceLinkPoll,
};
use scuffed_db::{AuditAction, AuditTargetType};
use scuffed_types::{
    DeviceLinkLookupResponse, DeviceLinkOkResponse, DeviceLinkPollRequest, DeviceLinkPollResponse,
    DeviceLinkStartRequest, DeviceLinkStartResponse, DeviceLinkUserCodeRequest,
};

use crate::extractors::OrgMember;
use crate::rate_limit::TrustedProxyIpKeyExtractor;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// Burst for `POST /api/link/start`, then one request per 30s.
pub const LINK_START_BURST: u32 = 4;
pub use crate::link_poll::{LINK_POLL_BURST, LINK_POLL_REFILL_SECS};
/// Burst shared by lookup, approve, and deny, then one request per 30s.
pub const LINK_USER_BURST: u32 = 12;

const USER_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const USER_CODE_LEN: usize = 8;
const DEVICE_CODE_HEX_LEN: usize = 64;
const LABEL_ERROR: &str = "device_label must be 1-64 characters without control characters";
const VERSION_ERROR: &str =
    "app_version must be 1-32 characters from [A-Za-z0-9._+-] and include a letter or digit";
const INVALID_CODE: &str = "invalid code";
const QUERY_CODE_ERROR: &str = "codes must be sent in the request body";
const BAD_ORIGIN: &str = "bad_origin";
const SEC_FETCH_SITE: header::HeaderName = header::HeaderName::from_static("sec-fetch-site");

/// Set by [`strip_link_query_secrets`] when the URI carried a code or token.
#[derive(Clone, Copy)]
pub(crate) struct LinkSecretInQuery;

fn key_extractor() -> &'static TrustedProxyIpKeyExtractor {
    static EXTRACTOR: OnceLock<TrustedProxyIpKeyExtractor> = OnceLock::new();
    EXTRACTOR.get_or_init(TrustedProxyIpKeyExtractor::from_env)
}

fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> std::net::IpAddr {
    key_extractor().client_ip(peer.ip(), headers)
}

fn invalid_code() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: INVALID_CODE.into(),
        }),
    )
}

fn bad_request(message: &'static str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn internal() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal error".into(),
        }),
    )
}

fn too_many_codes(secs: u64) -> Response {
    crate::rate_limit::rate_limited_response(secs)
}

fn note_invalid(state: &AppState, ip: std::net::IpAddr) -> Response {
    state.link_code_attempts.record_failure(ip);
    invalid_code().into_response()
}

fn blocked_response(state: &AppState, ip: std::net::IpAddr) -> Option<Response> {
    state.link_code_attempts.retry_after(ip).map(too_many_codes)
}

fn header_text<'a>(headers: &'a HeaderMap, name: &header::HeaderName) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn normalized_origin(value: &str) -> &str {
    value.trim().trim_end_matches('/')
}

/// `Origin` must match a configured site origin. With no `Origin`, only
/// `Sec-Fetch-Site: same-origin` is enough. A mismatched `Origin` fails on its own.
fn origin_is_allowed(state: &AppState, headers: &HeaderMap) -> bool {
    if let Some(origin) = header_text(headers, &header::ORIGIN) {
        let origin = normalized_origin(origin);
        if origin.is_empty() {
            return false;
        }
        let redirect = normalized_origin(&state.oauth_config.redirect_base_url);
        return origin == redirect
            || state
                .oauth_config
                .allowed_origins
                .iter()
                .any(|allowed| normalized_origin(allowed) == origin);
    }
    header_text(headers, &SEC_FETCH_SITE) == Some("same-origin")
}

fn too_many_polls(secs: u64) -> Response {
    crate::rate_limit::rate_limited_response(secs)
}

/// `Cache-Control: no-store` on every link response, including errors.
pub async fn no_store_link(req: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// Per-IP poll bucket. Runs outside the query-secret reject so a rejected
/// request still spends a cell, and inside [`no_store_link`] so the 429 is
/// not cached.
pub async fn limit_link_poll(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let Some(ConnectInfo(peer)) = req.extensions().get::<ConnectInfo<SocketAddr>>().copied() else {
        return internal().into_response();
    };
    let ip = client_ip(peer, req.headers());
    if let Some(secs) = state.link_poll.take(ip) {
        return too_many_polls(secs);
    }
    next.run(req).await
}

fn origin_rejected() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: BAD_ORIGIN.into(),
        }),
    )
        .into_response()
}

/// Drop query secrets before access logs see the URI, and flag the request.
pub async fn strip_link_query_secrets(mut req: axum::extract::Request, next: Next) -> Response {
    let carries_secret = req.uri().path().starts_with("/api/link")
        && req.uri().query().is_some_and(query_has_link_secret);
    if carries_secret {
        let cleaned = uri_without_query(req.uri());
        *req.uri_mut() = cleaned;
        req.extensions_mut().insert(LinkSecretInQuery);
    }
    next.run(req).await
}

/// Reject a flagged link request. Runs inside the per-route governor.
pub async fn reject_link_query_secrets(req: axum::extract::Request, next: Next) -> Response {
    if req.extensions().get::<LinkSecretInQuery>().is_some() {
        return bad_request(QUERY_CODE_ERROR).into_response();
    }
    next.run(req).await
}

fn query_has_link_secret(query: &str) -> bool {
    query.split('&').any(|pair| {
        let raw_key = pair.split_once('=').map(|(key, _)| key).unwrap_or(pair);
        let key = decode_query_key(raw_key);
        is_secret_query_key(key.trim())
    })
}

fn decode_query_key(raw: &str) -> String {
    let plus = raw.replace('+', " ");
    urlencoding::decode(&plus)
        .map(|decoded| decoded.into_owned())
        .unwrap_or(plus)
}

fn is_secret_query_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "user_code"
            | "device_code"
            | "usercode"
            | "devicecode"
            | "user-code"
            | "device-code"
            | "token"
            | "code"
    )
}

fn uri_without_query(uri: &Uri) -> Uri {
    let path = uri.path();
    let path = if path.is_empty() { "/" } else { path };
    let Ok(path_and_query) = path.parse::<PathAndQuery>() else {
        return Uri::from_static("/");
    };
    let mut parts = uri.clone().into_parts();
    parts.path_and_query = Some(path_and_query);
    Uri::from_parts(parts).unwrap_or_else(|_| Uri::from_static("/"))
}

fn validate_device_label(raw: &str) -> Result<String, &'static str> {
    let label = raw.trim();
    let len = label.chars().count();
    if !(1..=64).contains(&len) || label.chars().any(|c| c.is_control()) {
        return Err(LABEL_ERROR);
    }
    Ok(label.to_string())
}

fn validate_app_version(raw: &str) -> Result<String, &'static str> {
    let version = raw.trim();
    let bytes = version.as_bytes();
    if !(1..=32).contains(&bytes.len())
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
        || !bytes.iter().any(|b| b.is_ascii_alphanumeric())
    {
        return Err(VERSION_ERROR);
    }
    Ok(version.to_string())
}

fn canonical_user_code(raw: &str) -> Option<String> {
    if raw.len() > 32 {
        return None;
    }
    let mut out = String::with_capacity(USER_CODE_LEN);
    for ch in raw.chars() {
        if ch == '-' || ch == ' ' {
            continue;
        }
        let upper = ch.to_ascii_uppercase();
        if !upper.is_ascii() || !USER_CODE_ALPHABET.contains(&(upper as u8)) {
            return None;
        }
        out.push(upper);
    }
    if out.len() == USER_CODE_LEN {
        Some(out)
    } else {
        None
    }
}

fn format_user_code(canonical: &str) -> String {
    debug_assert_eq!(canonical.len(), USER_CODE_LEN);
    format!("{}-{}", &canonical[..4], &canonical[4..])
}

fn canonical_device_code(raw: &str) -> Option<String> {
    if raw.len() > 80 {
        return None;
    }
    let mut out = String::with_capacity(DEVICE_CODE_HEX_LEN);
    for ch in raw.chars() {
        if ch == ' ' || ch == '-' {
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            return None;
        }
        out.push(ch.to_ascii_lowercase());
    }
    if out.len() == DEVICE_CODE_HEX_LEN {
        Some(out)
    } else {
        None
    }
}

fn generate_user_code() -> (String, String) {
    let mut bytes = [0u8; USER_CODE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let canonical: String = bytes
        .iter()
        .map(|byte| USER_CODE_ALPHABET[(*byte as usize) % USER_CODE_ALPHABET.len()] as char)
        .collect();
    let display = format_user_code(&canonical);
    (display, canonical)
}

/// 32 random bytes, lowercase hex. Same shape as a pasted daemon token.
fn generate_secret_hex() -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn poll_body(status: &'static str, token: Option<String>) -> Json<DeviceLinkPollResponse> {
    Json(DeviceLinkPollResponse {
        status: status.to_string(),
        token,
    })
}

/// POST /api/link/start
pub async fn start(
    State(state): State<AppState>,
    Json(body): Json<DeviceLinkStartRequest>,
) -> Result<Json<DeviceLinkStartResponse>, (StatusCode, Json<ErrorResponse>)> {
    let device_label = validate_device_label(&body.device_label).map_err(bad_request)?;
    let app_version = validate_app_version(&body.app_version).map_err(bad_request)?;
    let (user_code, canonical_user) = generate_user_code();
    let device_code = generate_secret_hex();
    state
        .db
        .insert_device_link(&canonical_user, &device_code, &device_label, &app_version)
        .await
        .map_err(|_error| {
            tracing::error!("device link start failed");
            internal()
        })?;
    tracing::info!("device link started");
    Ok(Json(DeviceLinkStartResponse {
        user_code,
        device_code,
        interval: DEVICE_LINK_INTERVAL_SECS,
        expires_in: DEVICE_LINK_TTL_SECS,
    }))
}

/// POST /api/link/poll
pub async fn poll(
    State(state): State<AppState>,
    Json(body): Json<DeviceLinkPollRequest>,
) -> Result<Json<DeviceLinkPollResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some(device_code) = canonical_device_code(&body.device_code) else {
        return Ok(poll_body("expired", None));
    };
    let outcome = state
        .db
        .poll_device_link(&device_code, state.link_poll.now())
        .await
        .map_err(|_error| {
            tracing::error!("device link poll failed");
            internal()
        })?;
    let body = match outcome {
        DeviceLinkPoll::Pending => poll_body("pending", None),
        DeviceLinkPoll::SlowDown => poll_body("slow_down", None),
        DeviceLinkPoll::Denied => poll_body("denied", None),
        DeviceLinkPoll::Expired => poll_body("expired", None),
        DeviceLinkPoll::Approved(token) => {
            tracing::info!("device link token handed over");
            poll_body("approved", Some(token))
        }
    };
    Ok(body)
}

/// POST /api/link/lookup
pub async fn lookup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    _member: OrgMember,
    Json(body): Json<DeviceLinkUserCodeRequest>,
) -> Response {
    if !origin_is_allowed(&state, &headers) {
        return origin_rejected();
    }
    let ip = client_ip(peer, &headers);
    if let Some(response) = blocked_response(&state, ip) {
        return response;
    }
    let Some(user_code) = canonical_user_code(&body.user_code) else {
        return note_invalid(&state, ip);
    };
    match state.db.lookup_device_link(&user_code).await {
        Ok(Some(info)) => (
            StatusCode::OK,
            Json(DeviceLinkLookupResponse {
                device_label: info.device_label,
                app_version: info.app_version,
                created_at: info.created_at,
            }),
        )
            .into_response(),
        Ok(None) => note_invalid(&state, ip),
        Err(_error) => {
            tracing::error!("device link lookup failed");
            internal().into_response()
        }
    }
}

/// POST /api/link/approve
pub async fn approve(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    member: OrgMember,
    Json(body): Json<DeviceLinkUserCodeRequest>,
) -> Response {
    if !origin_is_allowed(&state, &headers) {
        return origin_rejected();
    }
    let ip = client_ip(peer, &headers);
    if let Some(response) = blocked_response(&state, ip) {
        return response;
    }
    let Some(user_code) = canonical_user_code(&body.user_code) else {
        return note_invalid(&state, ip);
    };
    let info = match state.db.lookup_device_link(&user_code).await {
        Ok(Some(info)) => info,
        Ok(None) => return note_invalid(&state, ip),
        Err(_error) => {
            tracing::error!("device link lookup failed");
            return internal().into_response();
        }
    };
    let mut secret = generate_secret_hex();
    let minted = match state
        .db
        .create_daemon_token(&member.member.id, &secret, &info.device_label)
        .await
    {
        Ok(token) => token,
        Err(_error) => {
            secret.zeroize();
            tracing::error!("device link token mint failed");
            return internal().into_response();
        }
    };
    let approved = state
        .db
        .approve_device_link(&user_code, &member.member.id, &minted.id, &secret)
        .await;
    secret.zeroize();
    match approved {
        Ok(true) => {
            tracing::info!("device link approved");
            audit(
                &state.db,
                &member.member.id,
                AuditAction::CreatedDaemonToken,
                AuditTargetType::DaemonToken,
                &minted.id,
                Some(&format!("label: {}", info.device_label)),
            )
            .await;
            (StatusCode::OK, Json(DeviceLinkOkResponse { ok: true })).into_response()
        }
        Ok(false) => {
            if let Err(_error) = state
                .db
                .revoke_daemon_token(&minted.id, &member.member.id)
                .await
            {
                tracing::error!("device link approve lost the race and revoke failed");
            }
            note_invalid(&state, ip)
        }
        Err(_error) => {
            tracing::error!("device link approve failed");
            internal().into_response()
        }
    }
}

/// POST /api/link/deny
///
/// Pending codes are open to any signed-in member. An approved code that the
/// device has not collected yet can be denied only by the member who approved
/// it. The daemon token is revoked before the code is marked denied. If that
/// revoke fails, the response is an error, not `ok: true`, and the code stays
/// approved. Any other member gets `invalid code` and the token stays active.
pub async fn deny(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    member: OrgMember,
    Json(body): Json<DeviceLinkUserCodeRequest>,
) -> Response {
    if !origin_is_allowed(&state, &headers) {
        return origin_rejected();
    }
    let ip = client_ip(peer, &headers);
    if let Some(response) = blocked_response(&state, ip) {
        return response;
    }
    let Some(user_code) = canonical_user_code(&body.user_code) else {
        return note_invalid(&state, ip);
    };
    match state
        .db
        .deny_device_link(&user_code, &member.member.id)
        .await
    {
        Ok(Some(denied)) => {
            if let Some(token_id) = denied.daemon_token_id {
                audit(
                    &state.db,
                    &member.member.id,
                    AuditAction::DeniedDeviceLink,
                    AuditTargetType::DaemonToken,
                    &token_id,
                    None,
                )
                .await;
            } else {
                audit(
                    &state.db,
                    &member.member.id,
                    AuditAction::DeniedDeviceLink,
                    AuditTargetType::Member,
                    &member.member.id,
                    None,
                )
                .await;
            }
            (StatusCode::OK, Json(DeviceLinkOkResponse { ok: true })).into_response()
        }
        Ok(None) => note_invalid(&state, ip),
        Err(_error) => {
            tracing::error!("device link deny failed");
            internal().into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_code_alphabet_is_unambiguous_and_32() {
        assert_eq!(USER_CODE_ALPHABET.len(), 32);
        let alphabet = std::str::from_utf8(USER_CODE_ALPHABET).unwrap();
        for banned in ['0', 'O', '1', 'I', 'l'] {
            assert!(!alphabet.contains(banned), "{banned}");
        }
        assert_eq!(256 % 32, 0, "byte modulo alphabet must be unbiased");
    }

    #[test]
    fn user_code_accepts_hyphen_and_case() {
        assert_eq!(
            canonical_user_code("abcd-2345").as_deref(),
            Some("ABCD2345")
        );
        assert_eq!(canonical_user_code("ABCD2345").as_deref(), Some("ABCD2345"));
        assert_eq!(
            canonical_user_code("ABCD 2345").as_deref(),
            Some("ABCD2345")
        );
        assert!(canonical_user_code("ABCD-234").is_none());
        assert!(canonical_user_code("ABCD-234O").is_none());
        assert!(canonical_user_code(&"A".repeat(40)).is_none());
        assert_eq!(format_user_code("ABCD2345"), "ABCD-2345");
    }

    #[test]
    fn device_code_is_64_hex() {
        let code = generate_secret_hex();
        assert_eq!(code.len(), 64);
        assert_eq!(canonical_device_code(&code).as_deref(), Some(code.as_str()));
        assert!(canonical_device_code("zz").is_none());
        assert!(canonical_device_code(&"ab".repeat(40)).is_none());
    }

    #[test]
    fn query_secret_keys_are_detected_without_keeping_values() {
        assert!(query_has_link_secret("user_code=ABCD-2345"));
        assert!(query_has_link_secret("foo=1&device_code=abc"));
        assert!(query_has_link_secret("user%5Fcode=ABCD"));
        assert!(query_has_link_secret("TOKEN=secret"));
        assert!(!query_has_link_secret("device_label=pc&app_version=1.0.0"));
    }

    #[test]
    fn label_and_version_bounds() {
        assert!(validate_device_label("  Desk  ").is_ok());
        assert!(validate_device_label("").is_err());
        assert!(validate_device_label(&"a".repeat(65)).is_err());
        assert!(validate_device_label("bad\nname").is_err());
        assert_eq!(validate_app_version(" 0.4.2 ").unwrap(), "0.4.2");
        assert!(validate_app_version("").is_err());
        assert!(validate_app_version("1.0.0 beta").is_err());
        assert!(validate_app_version(&"a".repeat(33)).is_err());
        assert!(validate_app_version("...").is_err());
    }

    #[test]
    fn wrong_code_limit_constant_is_the_documented_budget() {
        assert_eq!(crate::link_attempts::LINK_WRONG_CODE_LIMIT, 5);
        assert!((LINK_USER_BURST as usize) > crate::link_attempts::LINK_WRONG_CODE_LIMIT);
    }
}
