//! The GitHub Releases query behind [`crate::update::check_for_update`],
//! with a check that could not run told apart from "no newer version", so
//! Check now never reports "No newer version found" when it was offline or
//! refused.

use std::time::Duration;

use crate::update::{
    REPO, UpdateInfo, current_version, installed_update_channel, select_newer_release_on,
};

/// The check did not get a release list: offline, a timeout, or an answer
/// that is not one (a GitHub rate limit or error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckFailed;

/// `Ok(Some)` when a newer release is out, `Ok(None)` when this install is
/// current or its version cannot be read (nothing to compare with).
///
/// `per_page=100` so a player who skipped several releases still gets those
/// bodies for the in-app notes. The bundled changelog covers anything the
/// response leaves out.
pub async fn check() -> Result<Option<UpdateInfo>, CheckFailed> {
    let Some(current) = current_version() else {
        return Ok(None);
    };
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    let client = reqwest::Client::builder()
        .user_agent("scuffed-stat-tracker-gui")
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| CheckFailed)?;
    let releases: Vec<serde_json::Value> = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| CheckFailed)?
        .json()
        .await
        .map_err(|_| CheckFailed)?;
    Ok(select_newer_release_on(
        installed_update_channel(&current),
        &current,
        &releases,
    ))
}
