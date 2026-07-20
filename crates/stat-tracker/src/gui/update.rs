//! Lightweight "update available" check for the GUI.
//!
//! On dashboard load we query the GitHub Releases API once, compare the newest
//! `stat-tracker-v*` release to this build's version, and surface a banner if a
//! newer one exists. This is intentionally *detect + notify only* — it never
//! downloads or executes anything on its own (that would be a security surface
//! needing signed releases + restart coordination). The user re-runs the
//! installer one-liner, or clicks through to the release page.

use std::time::Duration;

use dioxus::prelude::*;

/// GitHub repo that publishes stat-tracker releases.
const REPO: &str = "FrozenTear/scuffed-crew";

/// The installer one-liner surfaced in the banner (matches the website).
const UPDATE_CMD: &str = "curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash";

/// Version of THIS build. Release CI sets `SST_RELEASE_VERSION` from the git
/// tag; otherwise fall back to the compiled `CARGO_PKG_VERSION`. Mirrors the
/// daemon's `version()` so the GUI and daemon report the same string.
pub fn current_version() -> &'static str {
    option_env!("SST_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Clone, PartialEq)]
struct UpdateInfo {
    latest: String,
    url: String,
}

/// Parse `MAJOR.MINOR.PATCH` into a comparable tuple; a leading `v` and any
/// `-pre`/`+build` suffix are stripped. Returns `None` on anything unparseable.
fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let core = s.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// Query GitHub Releases; return update info only if a newer release exists.
/// Any network/parse failure returns `None` so the banner simply stays hidden —
/// an update check must never break or block the dashboard.
async fn check_for_update() -> Option<UpdateInfo> {
    let cur = parse_semver(current_version())?;
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=20");
    let client = reqwest::Client::builder()
        .user_agent("scuffed-stat-tracker-gui")
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let releases: Vec<serde_json::Value> = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    // Newest non-draft, non-prerelease release tagged `stat-tracker-vX.Y.Z`.
    let mut best: Option<((u32, u32, u32), String, String)> = None;
    for r in releases {
        if r["draft"].as_bool().unwrap_or(false) || r["prerelease"].as_bool().unwrap_or(false) {
            continue;
        }
        let Some(ver_str) = r["tag_name"]
            .as_str()
            .and_then(|t| t.strip_prefix("stat-tracker-v"))
        else {
            continue;
        };
        let Some(ver) = parse_semver(ver_str) else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _, _)| ver > *b) {
            let html = r["html_url"].as_str().unwrap_or_default().to_string();
            best = Some((ver, ver_str.to_string(), html));
        }
    }

    let (latest_ver, latest_str, html_url) = best?;
    (latest_ver > cur).then_some(UpdateInfo {
        latest: latest_str,
        url: html_url,
    })
}

/// Dashboard banner shown only when a newer release is available. Renders
/// nothing while checking, on failure, or when up to date.
#[component]
pub fn UpdateBanner() -> Element {
    let info = use_resource(check_for_update);

    let Some(Some(update)) = info.read().clone() else {
        return rsx! {};
    };

    let open_url = update.url.clone();
    rsx! {
        div { class: "card card-update",
            h3 { "Update available — v{update.latest}" }
            p { class: "update-sub",
                "You're on v{current_version()}. Update by re-running the installer:"
            }
            pre { class: "update-cmd", "{UPDATE_CMD}" }
            button {
                class: "btn btn-primary",
                onclick: move |_| {
                    // Linux-only tool; open the release page in the default browser.
                    let _ = std::process::Command::new("xdg-open").arg(&open_url).spawn();
                },
                "View release notes"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parses_and_orders() {
        assert_eq!(parse_semver("0.2.1"), Some((0, 2, 1)));
        assert_eq!(parse_semver("v1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver("0.2"), Some((0, 2, 0)));
        assert_eq!(parse_semver("0.3.0-rc1"), Some((0, 3, 0)));
        assert!(parse_semver("garbage").is_none());
        // The whole point: a newer release must compare greater.
        assert!(parse_semver("0.2.1").unwrap() > parse_semver("0.1.0").unwrap());
        assert!(parse_semver("0.2.0").unwrap() > parse_semver("0.1.9").unwrap());
    }
}
