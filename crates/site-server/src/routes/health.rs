//! `GET /api/health`: liveness probe plus the git SHA of the running build.
//!
//! The status is always 200. `scripts/update.sh` and the Containerfile
//! HEALTHCHECK only look at the status code, so the body and header are
//! additive. The SHA comes from `SCUFFED_GIT_SHA`, which the publish workflow
//! passes as a Containerfile build-arg. A local or hand-built server reports
//! `unknown`.

use std::sync::OnceLock;

use axum::Json;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Environment variable the image sets to the commit it was built from.
pub const GIT_SHA_ENV: &str = "SCUFFED_GIT_SHA";

/// Response header that carries the same SHA as the JSON body.
pub const GIT_SHA_HEADER: HeaderName = HeaderName::from_static("x-scuffed-git-sha");

/// Reported when the build has no SHA, or the value is not a git SHA.
pub const UNKNOWN_SHA: &str = "unknown";

#[derive(Serialize)]
struct HealthBody<'a> {
    status: &'static str,
    git_sha: &'a str,
}

/// Keep only a plain hex git SHA (7 to 40 characters), lowercased.
/// Anything else becomes `unknown`, so the header value is always valid.
fn normalize_sha(raw: Option<&str>) -> String {
    let Some(raw) = raw.map(str::trim) else {
        return UNKNOWN_SHA.to_string();
    };
    if (7..=40).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        raw.to_ascii_lowercase()
    } else {
        UNKNOWN_SHA.to_string()
    }
}

fn build_sha() -> &'static str {
    static SHA: OnceLock<String> = OnceLock::new();
    SHA.get_or_init(|| normalize_sha(std::env::var(GIT_SHA_ENV).ok().as_deref()))
}

fn health_response(sha: &str) -> Response {
    let mut resp = (
        StatusCode::OK,
        Json(HealthBody {
            status: "ok",
            git_sha: sha,
        }),
    )
        .into_response();
    let headers = resp.headers_mut();
    if let Ok(value) = HeaderValue::from_str(sha) {
        headers.insert(GIT_SHA_HEADER, value);
    }
    resp
}

pub async fn health() -> Response {
    health_response(build_sha())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, header};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    async fn json_of(resp: Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn normalize_sha_keeps_hex_and_rejects_the_rest() {
        let full = "2FA4A28D0123456789abcdef0123456789ABCDEF";
        assert_eq!(normalize_sha(Some(full)), full.to_ascii_lowercase());
        assert_eq!(normalize_sha(Some(" 2fa4a28\n")), "2fa4a28");
        assert_eq!(normalize_sha(None), UNKNOWN_SHA);
        assert_eq!(normalize_sha(Some("")), UNKNOWN_SHA);
        assert_eq!(normalize_sha(Some("unknown")), UNKNOWN_SHA);
        assert_eq!(normalize_sha(Some("2fa4a2")), UNKNOWN_SHA);
        assert_eq!(normalize_sha(Some(&"a".repeat(41))), UNKNOWN_SHA);
        assert_eq!(normalize_sha(Some("2fa4a28\r\nx-evil: 1")), UNKNOWN_SHA);
    }

    #[tokio::test]
    async fn response_carries_the_sha_in_body_and_header() {
        let sha = "2fa4a28d0123456789abcdef0123456789abcdef";
        let resp = health_response(sha);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[&GIT_SHA_HEADER], sha);
        // API responses carry no Cache-Control (tests/seo.rs checks this
        // on /api/health).
        assert!(resp.headers().get(header::CACHE_CONTROL).is_none());
        assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/json");
        let body = json_of(resp).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["git_sha"], sha);
    }

    #[tokio::test]
    async fn router_health_is_200_and_reports_a_sha() {
        let app = crate::create_router(crate::test_support::test_state().await);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header("x-forwarded-for", "127.0.0.1")
                    .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                        [127, 0, 0, 1],
                        9,
                    ))))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let header_sha = resp.headers()[&GIT_SHA_HEADER]
            .to_str()
            .unwrap()
            .to_string();
        let body = json_of(resp).await;
        assert_eq!(body["status"], "ok");
        assert_eq!(body["git_sha"], header_sha.as_str());
        assert_eq!(header_sha, build_sha());
    }
}
