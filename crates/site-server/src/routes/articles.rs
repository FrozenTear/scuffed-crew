use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{Article, AuditAction, AuditTargetType};

use crate::extractors::{AdminUser, OfficerUser, OptionalOrgMember};
use crate::routes::audit_log::audit;
use crate::routes::uploads::upload_rel_is_safe;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListArticlesQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    20
}

/// GET /api/articles — list published articles (public)
pub async fn list_articles(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListArticlesQuery>,
) -> Result<Json<Vec<Article>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_published_articles(query.limit.min(100), query.offset)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/articles/admin/all — list all articles, including drafts (officer+)
pub async fn list_all_articles(
    State(state): State<AppState>,
    _officer: OfficerUser,
    axum::extract::Query(query): axum::extract::Query<ListArticlesQuery>,
) -> Result<Json<Vec<Article>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_all_articles(query.limit.min(100), query.offset)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/articles/:slug — published articles are public.
///
/// Drafts are returned only to officer+ (the same role as the admin article
/// routes) so the editor and an authenticated preview can still load them.
/// Everyone else gets the same 404 as a missing slug.
pub async fn get_article(
    State(state): State<AppState>,
    viewer: OptionalOrgMember,
    Path(slug): Path<String>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    let article = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => article_not_found(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ),
        })?;

    if article.published || viewer_can_read_drafts(&viewer) {
        Ok(Json(article))
    } else {
        Err(article_not_found())
    }
}

/// Officer and admin, matching `list_all_articles` / create / update / publish.
fn viewer_can_read_drafts(viewer: &OptionalOrgMember) -> bool {
    viewer
        .0
        .as_ref()
        .is_some_and(|member| member.org_role.is_at_least(scuffed_db::OrgRole::Officer))
}

fn article_not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Article not found".into(),
        }),
    )
}

pub(crate) const COVER_IMAGE_URL_ERROR: &str =
    "cover_image_url must be empty, an https URL, or an /uploads/images/ path";

/// Accept an article cover image URL on create / update.
///
/// `Ok(None)` clears the field (empty / whitespace). `Ok(Some)` is the trimmed
/// value: either an absolute `https://` URL with a host, or a file under
/// `/uploads/images/<folder>/` (what `POST /api/upload/image` returns; any
/// member's folder, since officers co-edit articles). Mirrors
/// `normalize_avatar_url`.
pub(crate) fn normalize_cover_image_url(raw: &str) -> Result<Option<String>, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err(COVER_IMAGE_URL_ERROR);
    }
    if let Some(rest) = trimmed
        .get(..8)
        .filter(|prefix| prefix.eq_ignore_ascii_case("https://"))
        .map(|_| &trimmed[8..])
    {
        // `url` folds `https:///x` into host `x`; require the authority here.
        let host_ok = !rest.starts_with('/')
            && url::Url::parse(trimmed).is_ok_and(|u| {
                u.scheme() == "https" && u.host_str().is_some_and(|h| !h.is_empty())
            });
        return if host_ok {
            Ok(Some(trimmed.to_string()))
        } else {
            Err(COVER_IMAGE_URL_ERROR)
        };
    }
    if let Some(rel) = trimmed.strip_prefix("/uploads/images/")
        && rel.contains('/')
        && upload_rel_is_safe(rel)
    {
        return Ok(Some(trimmed.to_string()));
    }
    Err(COVER_IMAGE_URL_ERROR)
}

fn bad_cover_image_url(msg: &'static str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

#[derive(Deserialize)]
pub struct CreateArticleRequest {
    pub slug: String,
    pub title: String,
    pub content_markdown: String,
    pub summary: Option<String>,
    pub cover_image_url: Option<String>,
}

/// POST /api/articles — create a draft article (officer+)
pub async fn create_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateArticleRequest>,
) -> Result<(StatusCode, Json<Article>), (StatusCode, Json<ErrorResponse>)> {
    let cover_image_url = match body.cover_image_url.as_deref() {
        Some(raw) => normalize_cover_image_url(raw).map_err(bad_cover_image_url)?,
        None => None,
    };
    let article = state
        .db
        .create_article(
            &body.slug,
            &body.title,
            &body.content_markdown,
            body.summary.as_deref(),
            cover_image_url.as_deref(),
            &officer.member.id,
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedArticle,
        AuditTargetType::Article,
        &article.id,
        Some(&format!("Created: {}", article.title)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(article)))
}

#[derive(Deserialize)]
pub struct UpdateArticleRequest {
    pub title: Option<String>,
    pub content_markdown: Option<String>,
    pub summary: Option<Option<String>>,
    pub cover_image_url: Option<Option<String>>,
}

/// PUT /api/articles/:slug — update article (officer+)
pub async fn update_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
    Json(body): Json<UpdateArticleRequest>,
) -> Result<Json<Article>, (StatusCode, Json<ErrorResponse>)> {
    // Omitted keeps the stored value; null or blank clears it.
    let cover_image_url = match body.cover_image_url {
        Some(Some(raw)) => Some(normalize_cover_image_url(&raw).map_err(bad_cover_image_url)?),
        Some(None) => Some(None),
        None => None,
    };

    // Look up the article by slug to get its ID
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => article_not_found(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ),
        })?;

    let article = state
        .db
        .update_article(
            &existing.id,
            body.title.as_deref(),
            body.content_markdown.as_deref(),
            body.summary.as_ref().map(|s| s.as_deref()),
            cover_image_url.as_ref().map(|s| s.as_deref()),
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedArticle,
        AuditTargetType::Article,
        &existing.id,
        None,
    )
    .await;

    Ok(Json(article))
}

/// POST /api/articles/:slug/publish — publish article (officer+)
pub async fn publish_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => article_not_found(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ),
        })?;

    state.db.publish_article(&existing.id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::PublishedArticle,
        AuditTargetType::Article,
        &existing.id,
        Some(&format!("Published: {}", existing.title)),
    )
    .await;

    Ok(StatusCode::OK)
}

/// POST /api/articles/:slug/unpublish — unpublish article (officer+)
pub async fn unpublish_article(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => article_not_found(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ),
        })?;

    state
        .db
        .unpublish_article(&existing.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UnpublishedArticle,
        AuditTargetType::Article,
        &existing.id,
        Some(&format!("Unpublished: {}", existing.title)),
    )
    .await;

    Ok(StatusCode::OK)
}

/// DELETE /api/articles/:slug — delete article (admin only)
pub async fn delete_article(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let existing = state
        .db
        .get_article_by_slug(&slug)
        .await
        .map_err(|e| match &e {
            scuffed_db::DbError::NotFound(_) => article_not_found(),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            ),
        })?;

    state.db.delete_article(&existing.id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::DeletedArticle,
        AuditTargetType::Article,
        &existing.id,
        Some(&format!("Deleted: {}", existing.title)),
    )
    .await;

    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_image_url_accepts_blank_https_and_image_uploads() {
        for blank in ["", "   ", "\t\n"] {
            assert_eq!(normalize_cover_image_url(blank), Ok(None), "{blank:?}");
        }
        for (raw, want) in [
            (
                "https://cdn.example.com/a.png",
                "https://cdn.example.com/a.png",
            ),
            (
                "HTTPS://cdn.example.com/a.png",
                "HTTPS://cdn.example.com/a.png",
            ),
            ("HtTpS://cdn.example.com", "HtTpS://cdn.example.com"),
            (
                "  https://cdn.example.com/a.png?w=1#x  ",
                "https://cdn.example.com/a.png?w=1#x",
            ),
            (
                "/uploads/images/member_abc/cover.png",
                "/uploads/images/member_abc/cover.png",
            ),
            (
                " /uploads/images/member_other/sub/c-1_x.webp ",
                "/uploads/images/member_other/sub/c-1_x.webp",
            ),
        ] {
            assert_eq!(
                normalize_cover_image_url(raw),
                Ok(Some(want.to_string())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn cover_image_url_rejects_everything_else() {
        for bad in [
            "http://cdn.example.com/a.png",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:image/png;base64,AAAA",
            "vbscript:msgbox(1)",
            "//cdn.example.com/a.png",
            "https:cdn.example.com/a.png",
            "https:/cdn.example.com/a.png",
            "https://",
            "https:///a.png",
            "https://?x",
            "https://#x",
            "https://cdn.example.com/a b.png",
            "https://cdn.example.com/a\tb.png",
            "https://cdn.example.com/a\u{0}.png",
            "https://cdn.example.com\\a.png",
            "/uploads/avatars/member_abc/a.png",
            "/uploads/other/member_abc/a.png",
            "/assets/a.png",
            "uploads/images/member_abc/a.png",
            "/uploads/images/",
            "/uploads/images/a.png",
            "/uploads/images/member_abc/",
            "/uploads/images/member_abc/../x.png",
            "/uploads/images/../avatars/m/x.png",
            "/uploads/images/member_abc/./x.png",
            "/uploads/images/member_abc//x.png",
            "/uploads/images/member_abc/x.png?v=1",
            "/uploads/images/member_abc/x.png#f",
            "/uploads/images/member_abc/%2e%2e/x.png",
            "/uploads/images/member_abc\\x.png",
            "/uploads/images/member abc/x.png",
        ] {
            assert_eq!(
                normalize_cover_image_url(bad),
                Err(COVER_IMAGE_URL_ERROR),
                "{bad:?}"
            );
        }
    }
}
