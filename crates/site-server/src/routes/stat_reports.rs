//! Tracker bug-report HTTP routes.
//!
//! `POST /api/stat-reports` stores a zip for a signed-in member.
//! `GET /api/stat-reports` lists the caller's rows, or every row for an officer.
//! `GET /api/stat-reports/{id}` and `GET /api/stat-reports/{id}/manifest` are
//! officer-only. `DELETE` is the owner or an officer. `POST .../withdraw` and
//! `PATCH /api/stat-reports/{id}` clear training consent for the owner only.
//! Withdraw sets expiry to the original received time plus 30 days, and deletes
//! the report now if that instant has already passed.

use std::path::Path;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AuditAction, AuditTargetType, NewStatReport, OrgRole, StatReport, retention_deadline,
    utc_day_start,
};
use scuffed_types::{
    DAILY_REPORT_CAP, MAX_BUNDLE_BYTES, StatReportCreated, StatReportDeleted, StatReportList,
    StatReportListItem, StatReportWithdrawn,
};
use uuid::Uuid;

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::stat_reports::{
    BundleReject, accept_bundle, ensure_reports_dir, is_report_id, remove_report_files,
    report_partial_path, report_zip_path, reports_dir_conflicts,
};
use crate::state::AppState;

fn err(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

fn map_bundle(reject: BundleReject) -> (StatusCode, Json<ErrorResponse>) {
    match reject {
        BundleReject::TooLarge => err(StatusCode::PAYLOAD_TOO_LARGE, "Bundle is too large"),
        BundleReject::Invalid(reason) => {
            let message = if reason.len() > 180 {
                "Invalid report bundle".to_string()
            } else {
                format!("Invalid report bundle: {reason}")
            };
            err(StatusCode::BAD_REQUEST, &message)
        }
    }
}

fn member_item(report: &StatReport) -> StatReportListItem {
    StatReportListItem {
        id: report.id.clone(),
        created_at: report.created_at,
        expires_at: report.expires_at,
        training_consent: report.training_consent,
        own_name_included: report.own_name_included,
        glyphs_included: report.glyphs_included,
        size_bytes: report.size_bytes,
        member_id: None,
        reason_category: None,
        reason_text: None,
        app_version: None,
        recognizer_matcher: None,
        recognizer_ocr: None,
        zip_sha256: None,
    }
}

fn officer_item(report: &StatReport) -> StatReportListItem {
    StatReportListItem {
        member_id: Some(report.member_id.clone()),
        reason_category: Some(report.reason_category.clone()),
        reason_text: Some(report.reason_text.clone()),
        app_version: Some(report.app_version.clone()),
        recognizer_matcher: Some(report.recognizer_matcher.clone()),
        recognizer_ocr: Some(report.recognizer_ocr.clone()),
        zip_sha256: Some(report.zip_sha256.clone()),
        ..member_item(report)
    }
}

fn configured(state: &AppState) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if !state.reports_enabled {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "reports_disabled"));
    }
    if reports_dir_conflicts(&state.reports_dir, &state.upload_dir) {
        tracing::error!("REPORTS_DIR overlaps the upload directory or the web root");
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"));
    }
    Ok(())
}

fn report_is_current(report: &StatReport) -> bool {
    report.expires_at.is_none_or(|exp| exp > Utc::now())
}

async fn delete_files_or_keep_row(
    state: &AppState,
    id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    remove_report_files(&state.reports_dir, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, report_id = %id, "stat report file delete failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })
}

async fn load_owned(
    state: &AppState,
    id: &str,
) -> Result<StatReport, (StatusCode, Json<ErrorResponse>)> {
    if !is_report_id(id) {
        return Err(err(StatusCode::NOT_FOUND, "Report not found"));
    }
    state
        .db
        .get_stat_report(id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "stat report lookup failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "Report not found"))
}

/// POST /api/stat-reports
pub async fn create_stat_report(
    State(state): State<AppState>,
    member: OrgMember,
    body: Bytes,
) -> Result<(StatusCode, Json<StatReportCreated>), (StatusCode, Json<ErrorResponse>)> {
    configured(&state)?;
    if body.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(err(StatusCode::PAYLOAD_TOO_LARGE, "Bundle is too large"));
    }

    let start = utc_day_start(Utc::now());
    let stored = state
        .db
        .count_stat_reports_since(&member.member.id, start)
        .await
        .map_err(|error| {
            tracing::error!(%error, "stat report count failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;
    if stored >= DAILY_REPORT_CAP {
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "Daily report limit reached",
        ));
    }
    let accepted = accept_bundle(&body).map_err(map_bundle)?;

    ensure_reports_dir(&state.reports_dir)
        .await
        .map_err(|error| {
            tracing::error!(%error, "could not create reports directory");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;

    let id = Uuid::new_v4().simple().to_string();
    let created_at = Utc::now();
    let training = accepted.manifest.training;
    let expires_at = if training {
        None
    } else {
        Some(retention_deadline(created_at))
    };
    let partial = report_partial_path(&state.reports_dir, &id);
    write_private(&partial, &accepted.bytes).await?;

    let row = NewStatReport {
        id: id.clone(),
        member_id: member.member.id.clone(),
        created_at,
        reason_category: accepted.manifest.reason_category.clone(),
        reason_text: accepted.manifest.reason_text.clone(),
        app_version: accepted.manifest.app_version.clone(),
        recognizer_matcher: accepted.manifest.matcher.clone(),
        recognizer_ocr: accepted.manifest.ocr.clone(),
        training_consent: training,
        own_name_included: accepted.manifest.own_name_included,
        glyphs_included: accepted.manifest.glyphs_included,
        size_bytes: accepted.bytes.len() as u64,
        zip_sha256: accepted.sha256.clone(),
        expires_at,
    };
    if let Err(error) = state.db.insert_stat_report(&row).await {
        if let Err(file_error) = remove_report_files(&state.reports_dir, &id).await {
            tracing::error!(%file_error, report_id = %id, "stat report file delete failed");
        }
        tracing::error!(%error, report_id = %id, "stat report insert failed");
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"));
    }

    let stored = state
        .db
        .count_stat_reports_since(&member.member.id, start)
        .await
        .map_err(|error| {
            tracing::error!(%error, "stat report recount failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;
    if stored > DAILY_REPORT_CAP {
        delete_files_or_keep_row(&state, &id).await?;
        let _ = state.db.delete_stat_report(&id).await;
        return Err(err(
            StatusCode::TOO_MANY_REQUESTS,
            "Daily report limit reached",
        ));
    }

    let final_path = report_zip_path(&state.reports_dir, &id);
    match tokio::fs::rename(&partial, &final_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && final_path.exists() => {}
        Err(error) => {
            tracing::error!(%error, report_id = %id, "stat report rename failed");
            delete_files_or_keep_row(&state, &id).await?;
            let _ = state.db.delete_stat_report(&id).await;
            return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"));
        }
    }

    audit(
        &state.db,
        &member.member.id,
        AuditAction::CreatedStatReport,
        AuditTargetType::StatReport,
        &id,
        Some(&format!(
            "training={training} size={}",
            accepted.bytes.len()
        )),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(StatReportCreated {
            id,
            training,
            expires_at,
        }),
    ))
}

async fn write_private(path: &Path, bytes: &[u8]) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        // tokio::fs::OpenOptions::mode is inherent on unix. 0600 keeps the zip
        // unreadable by other users on the host.
        options.mode(0o600);
    }
    let mut file = options.open(path).await.map_err(|error| {
        tracing::error!(%error, "stat report file create failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    use tokio::io::AsyncWriteExt;
    file.write_all(bytes).await.map_err(|error| {
        tracing::error!(%error, "stat report file write failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    file.sync_all().await.map_err(|error| {
        tracing::error!(%error, "stat report file sync failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    Ok(())
}

/// GET /api/stat-reports
pub async fn list_stat_reports(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<StatReportList>, (StatusCode, Json<ErrorResponse>)> {
    configured(&state)?;
    let officer = member.member.org_role.is_at_least(OrgRole::Officer);
    let rows = if officer {
        state.db.list_stat_reports().await
    } else {
        state
            .db
            .list_stat_reports_for_member(&member.member.id)
            .await
    }
    .map_err(|error| {
        tracing::error!(%error, "stat report list failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
    })?;
    let rows: Vec<_> = rows.into_iter().filter(report_is_current).collect();
    let reports = if officer {
        rows.iter().map(officer_item).collect()
    } else {
        rows.iter().map(member_item).collect()
    };
    Ok(Json(StatReportList { reports }))
}

/// GET /api/stat-reports/{id}
pub async fn download_stat_report(
    State(state): State<AppState>,
    officer: OfficerUser,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    configured(&state)?;
    let report = load_owned(&state, &id).await?;
    if !report_is_current(&report) {
        return Err(err(StatusCode::NOT_FOUND, "Report not found"));
    }
    let bytes = tokio::fs::read(report_zip_path(&state.reports_dir, &report.id))
        .await
        .map_err(|error| {
            tracing::error!(%error, report_id = %report.id, "stat report read failed");
            err(StatusCode::NOT_FOUND, "Report not found")
        })?;
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::DownloadedStatReport,
        AuditTargetType::StatReport,
        &report.id,
        None,
    )
    .await;
    let id = &report.id;
    let filename = format!("attachment; filename=\"stat-report-{id}.zip\"");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, filename)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))
}

/// GET /api/stat-reports/{id}/manifest
pub async fn read_stat_report_manifest(
    State(state): State<AppState>,
    officer: OfficerUser,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    configured(&state)?;
    let report = load_owned(&state, &id).await?;
    if !report_is_current(&report) {
        return Err(err(StatusCode::NOT_FOUND, "Report not found"));
    }
    let bytes = tokio::fs::read(report_zip_path(&state.reports_dir, &report.id))
        .await
        .map_err(|_| err(StatusCode::NOT_FOUND, "Report not found"))?;
    let manifest = manifest_from_zip(&bytes)?;
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::ReadStatReportManifest,
        AuditTargetType::StatReport,
        &report.id,
        None,
    )
    .await;
    Ok(Json(manifest))
}

fn manifest_from_zip(bytes: &[u8]) -> Result<serde_json::Value, (StatusCode, Json<ErrorResponse>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))?;
    let mut file = archive
        .by_name("manifest.json")
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))?;
    let mut raw = Vec::new();
    use std::io::Read;
    file.read_to_end(&mut raw)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))?;
    if raw.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"));
    }
    serde_json::from_slice(&raw)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error"))
}

/// DELETE /api/stat-reports/{id}
pub async fn delete_stat_report(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<StatReportDeleted>, (StatusCode, Json<ErrorResponse>)> {
    configured(&state)?;
    let report = load_owned(&state, &id).await?;
    let officer = member.member.org_role.is_at_least(OrgRole::Officer);
    if report.member_id != member.member.id && !officer {
        return Err(err(StatusCode::FORBIDDEN, "Forbidden"));
    }
    delete_files_or_keep_row(&state, &report.id).await?;
    state
        .db
        .delete_stat_report(&report.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "stat report delete failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;
    audit(
        &state.db,
        &member.member.id,
        AuditAction::DeletedStatReport,
        AuditTargetType::StatReport,
        &report.id,
        None,
    )
    .await;
    Ok(Json(StatReportDeleted { deleted: true }))
}

/// POST /api/stat-reports/{id}/withdraw
/// PATCH /api/stat-reports/{id}
pub async fn withdraw_stat_report(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
    body: Bytes,
) -> Result<Json<StatReportWithdrawn>, (StatusCode, Json<ErrorResponse>)> {
    configured(&state)?;
    if !withdraw_body_ok(&body) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "Withdraw only clears training consent",
        ));
    }
    let report = load_owned(&state, &id).await?;
    if report.member_id != member.member.id {
        return Err(err(StatusCode::FORBIDDEN, "Forbidden"));
    }
    let deadline = retention_deadline(report.created_at);
    let due = deadline <= Utc::now();
    if !report.training_consent {
        if due || report.expires_at.is_some_and(|exp| exp <= Utc::now()) {
            return Ok(Json(delete_for_withdraw(&state, &member, &report).await?));
        }
        return Ok(Json(StatReportWithdrawn {
            id: report.id,
            deleted: false,
            training: false,
            expires_at: report.expires_at,
        }));
    }
    if due {
        return Ok(Json(delete_for_withdraw(&state, &member, &report).await?));
    }
    let updated = state
        .db
        .withdraw_stat_report_training(&report.id, deadline)
        .await
        .map_err(|error| {
            tracing::error!(%error, "stat report withdraw failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;
    let Some(updated) = updated else {
        let current = load_owned(&state, &report.id).await?;
        if !current.training_consent {
            return Ok(Json(StatReportWithdrawn {
                id: current.id,
                deleted: false,
                training: false,
                expires_at: current.expires_at,
            }));
        }
        return Err(err(StatusCode::CONFLICT, "Report changed"));
    };
    audit(
        &state.db,
        &member.member.id,
        AuditAction::WithdrawnStatReportTraining,
        AuditTargetType::StatReport,
        &updated.id,
        None,
    )
    .await;
    Ok(Json(StatReportWithdrawn {
        id: updated.id,
        deleted: false,
        training: false,
        expires_at: updated.expires_at,
    }))
}

async fn delete_for_withdraw(
    state: &AppState,
    member: &OrgMember,
    report: &StatReport,
) -> Result<StatReportWithdrawn, (StatusCode, Json<ErrorResponse>)> {
    delete_files_or_keep_row(state, &report.id).await?;
    state
        .db
        .delete_stat_report(&report.id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "stat report withdraw delete failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        })?;
    audit(
        state.db.as_ref(),
        &member.member.id,
        AuditAction::WithdrawnStatReportTraining,
        AuditTargetType::StatReport,
        &report.id,
        Some("deleted=true"),
    )
    .await;
    Ok(StatReportWithdrawn {
        id: report.id.clone(),
        deleted: true,
        training: false,
        expires_at: None,
    })
}

fn withdraw_body_ok(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.len() != 1 {
        return false;
    }
    matches!(
        (
            obj.get("training").and_then(|v| v.as_bool()),
            obj.get("training_consent").and_then(|v| v.as_bool()),
        ),
        (Some(false), None) | (None, Some(false))
    )
}
