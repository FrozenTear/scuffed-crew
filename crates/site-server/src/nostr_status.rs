//! Caches behind `GET /api/nostr/health`.
//!
//! The relay probe (settings read plus a short HTTP check of the relay) is
//! reused for 30 seconds. `tracker_latest` is a separate one-hour cache of the
//! highest `stat-tracker-v*` GitHub release. A health request never waits on
//! that lookup: it serves the cached version, or omits the field when none is
//! known yet, and refreshes in the background.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// How long a relay-health snapshot may be served.
pub const RELAY_HEALTH_TTL: Duration = Duration::from_secs(30);

/// How long a successful stat-tracker release lookup may be served.
pub const TRACKER_RELEASE_TTL: Duration = Duration::from_secs(60 * 60);

/// Bound on one GitHub releases lookup, including every page.
pub const GITHUB_TIMEOUT: Duration = Duration::from_secs(5);

/// After a failed lookup, wait this long before starting another.
///
/// A cold failure must not retry on every health hit (the unauthenticated
/// GitHub budget is small), and it must not sit for the full success TTL.
const TRACKER_FAILURE_BACKOFF: Duration = Duration::from_secs(60);

const TAG_PREFIX: &str = "stat-tracker-v";
const RELEASES_URL: &str = "https://api.github.com/repos/FrozenTear/scuffed-crew/releases";
const USER_AGENT: &str = "scuffed-site-server (+https://github.com/FrozenTear/scuffed-crew)";
const PER_PAGE: usize = 100;
const MAX_PAGES: u32 = 10;

/// Relay probe result, without `tracker_latest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayHealthSnapshot {
    pub configured: bool,
    pub reachable: bool,
    pub relay_url: Option<String>,
    pub extra_relay_urls: Vec<String>,
    pub relay_info: Option<RelayInfoSnapshot>,
    pub forum_backend: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayInfoSnapshot {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// One GitHub release, as far as version selection cares.
///
/// `published_at` is not a field: the winner is the highest semver, not the
/// newest publish time.
#[derive(Clone, Debug)]
pub struct ReleaseView<'a> {
    pub tag_name: &'a str,
    pub draft: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
}

struct RelayCacheInner {
    snap: Option<(RelayHealthSnapshot, Instant)>,
}

struct TrackerCacheInner {
    value: Option<String>,
    stored_at: Option<Instant>,
    refreshing: bool,
    retry_not_before: Option<Instant>,
}

/// Process-local relay and tracker caches. Shared across `AppState` clones.
#[derive(Clone)]
pub struct NostrStatus {
    relay: Arc<Mutex<RelayCacheInner>>,
    tracker: Arc<Mutex<TrackerCacheInner>>,
}

impl NostrStatus {
    pub fn new() -> Self {
        Self {
            relay: Arc::new(Mutex::new(RelayCacheInner { snap: None })),
            tracker: Arc::new(Mutex::new(TrackerCacheInner {
                value: None,
                stored_at: None,
                refreshing: false,
                retry_not_before: None,
            })),
        }
    }

    /// Snapshot stored within [`RELAY_HEALTH_TTL`], if any.
    pub fn cached_relay(&self, now: Instant) -> Option<RelayHealthSnapshot> {
        let guard = self.relay.lock().unwrap_or_else(|err| err.into_inner());
        guard.snap.as_ref().and_then(|(snap, stored)| {
            (now.saturating_duration_since(*stored) < RELAY_HEALTH_TTL).then(|| snap.clone())
        })
    }

    pub fn store_relay(&self, now: Instant, snap: RelayHealthSnapshot) {
        let mut guard = self.relay.lock().unwrap_or_else(|err| err.into_inner());
        guard.snap = Some((snap, now));
    }

    /// Cached tracker version for this response.
    ///
    /// Does not wait on GitHub. A miss or an expired entry schedules one
    /// background refresh and returns the previous value (`None` until the
    /// first success).
    pub fn tracker_latest(&self) -> Option<String> {
        let (value, refresh) = self.serve_tracker(Instant::now());
        if refresh {
            let status = self.clone();
            tokio::spawn(async move {
                status.refresh_tracker().await;
            });
        }
        value
    }

    fn serve_tracker(&self, now: Instant) -> (Option<String>, bool) {
        let mut guard = self.tracker.lock().unwrap_or_else(|err| err.into_inner());
        let fresh = guard
            .stored_at
            .is_some_and(|stored| now.saturating_duration_since(stored) < TRACKER_RELEASE_TTL);
        if fresh || guard.refreshing {
            return (guard.value.clone(), false);
        }
        if guard.retry_not_before.is_some_and(|at| now < at) {
            return (guard.value.clone(), false);
        }
        guard.refreshing = true;
        (guard.value.clone(), true)
    }

    fn store_tracker_success(&self, now: Instant, value: Option<String>) {
        let mut guard = self.tracker.lock().unwrap_or_else(|err| err.into_inner());
        guard.value = value;
        guard.stored_at = Some(now);
        guard.refreshing = false;
        guard.retry_not_before = None;
    }

    fn store_tracker_failure(&self, now: Instant) {
        let mut guard = self.tracker.lock().unwrap_or_else(|err| err.into_inner());
        guard.refreshing = false;
        guard.retry_not_before = Some(now + TRACKER_FAILURE_BACKOFF);
    }

    async fn refresh_tracker(&self) {
        // If the task is cancelled mid-lookup, drop clears `refreshing` so
        // the next health request can try again. A finished store has already
        // cleared that flag, so drop then does nothing.
        let guard = RefreshGuard {
            status: self.clone(),
        };
        match tokio::time::timeout(GITHUB_TIMEOUT, fetch_latest_tracker_version()).await {
            Ok(Ok(value)) => guard.status.store_tracker_success(Instant::now(), value),
            Ok(Err(err)) => {
                tracing::warn!(error = %err, "stat tracker release lookup failed");
                guard.status.store_tracker_failure(Instant::now());
            }
            Err(_) => {
                tracing::warn!("stat tracker release lookup timed out");
                guard.status.store_tracker_failure(Instant::now());
            }
        }
    }
}

/// Clears an in-flight refresh when the lookup task ends without storing.
struct RefreshGuard {
    status: NostrStatus,
}

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        let mut guard = self
            .status
            .tracker
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        if guard.refreshing {
            guard.refreshing = false;
            guard.retry_not_before = Some(Instant::now() + TRACKER_FAILURE_BACKOFF);
        }
    }
}

impl Default for NostrStatus {
    fn default() -> Self {
        Self::new()
    }
}

/// Highest `stat-tracker-v*` version, prefix removed.
///
/// Pre-releases stay in the running. Drafts are skipped. Tags that do not
/// start with [`TAG_PREFIX`], and suffixes that are not semver, are skipped.
/// Publish time is not an input.
pub fn select_latest_tracker_version<'a>(
    releases: impl IntoIterator<Item = ReleaseView<'a>>,
) -> Option<String> {
    let mut best: Option<(semver::Version, &'a str)> = None;
    for release in releases {
        let Some(version) = tracker_version(&release) else {
            continue;
        };
        let Ok(parsed) = semver::Version::parse(version) else {
            continue;
        };
        let replace = match &best {
            Some((current, _)) => parsed > *current,
            None => true,
        };
        if replace {
            best = Some((parsed, version));
        }
    }
    best.map(|(_, version)| version.to_string())
}

fn tracker_version<'a>(release: &ReleaseView<'a>) -> Option<&'a str> {
    if release.draft {
        return None;
    }
    let version = release.tag_name.strip_prefix(TAG_PREFIX)?;
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// Keep reading pages until a short page or `max_pages`.
///
/// A page shorter than `per_page` is the last page, including an empty one.
pub fn concat_release_pages<T: Clone>(
    pages: &[Vec<T>],
    per_page: usize,
    max_pages: usize,
) -> Vec<T> {
    let mut all = Vec::new();
    for page in pages.iter().take(max_pages) {
        let last = page.len() < per_page;
        all.extend(page.iter().cloned());
        if last {
            break;
        }
    }
    all
}

async fn fetch_latest_tracker_version() -> Result<Option<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(GITHUB_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let github = url.scheme() == "https" && url.host_str() == Some("api.github.com");
            if !github {
                return attempt.error("redirect left api.github.com");
            }
            if attempt.previous().len() >= 3 {
                attempt.error("too many redirects")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|err| format!("http client: {err}"))?;

    let mut pages = Vec::new();
    for page_no in 1..=MAX_PAGES {
        let batch = fetch_release_page(&client, page_no).await?;
        let last = batch.len() < PER_PAGE;
        pages.push(batch);
        if last {
            break;
        }
    }
    let releases = concat_release_pages(&pages, PER_PAGE, MAX_PAGES as usize);
    let views = releases
        .iter()
        .map(|release| ReleaseView {
            tag_name: release.tag_name.as_str(),
            draft: release.draft,
        })
        .collect::<Vec<_>>();
    Ok(select_latest_tracker_version(views))
}

async fn fetch_release_page(
    client: &reqwest::Client,
    page: u32,
) -> Result<Vec<GithubRelease>, String> {
    // Unauthenticated on purpose: release tags are public, and this process
    // does not hold a GitHub token.
    let response = client
        .get(RELEASES_URL)
        .query(&[
            ("per_page", PER_PAGE.to_string()),
            ("page", page.to_string()),
        ])
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|err| format!("github releases request: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("github releases HTTP {status}"));
    }
    response
        .json::<Vec<GithubRelease>>()
        .await
        .map_err(|err| format!("github releases body: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(tag_name: &'a str, draft: bool) -> ReleaseView<'a> {
        ReleaseView { tag_name, draft }
    }

    #[test]
    fn selects_highest_tracker_semver_not_the_newest_publish_date() {
        // `published_at` is part of the fixture and is not passed to
        // selection. `0.4.2` is the newest non-draft. `alpha.2` sorts above
        // `alpha.11` as text. The draft would win on both semver and date.
        struct Fixture {
            tag: &'static str,
            draft: bool,
            published_at: &'static str,
        }
        let fixtures = [
            Fixture {
                tag: "stat-tracker-v0.4.2",
                draft: false,
                published_at: "2026-10-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-v0.5.0-alpha.2",
                draft: false,
                published_at: "2026-08-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-v0.5.0-alpha.10",
                draft: false,
                published_at: "2026-01-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-v0.5.0-alpha.11",
                draft: false,
                published_at: "2026-02-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-v9.9.9",
                draft: true,
                published_at: "2026-10-02T00:00:00Z",
            },
            Fixture {
                tag: "v0.9.0",
                draft: false,
                published_at: "2026-09-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-0.8.0",
                draft: false,
                published_at: "2026-07-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-vnot-semver",
                draft: false,
                published_at: "2026-06-01T00:00:00Z",
            },
            Fixture {
                tag: "stat-tracker-v",
                draft: false,
                published_at: "2026-05-01T00:00:00Z",
            },
        ];
        let newest = fixtures
            .iter()
            .max_by_key(|row| row.published_at)
            .expect("fixtures");
        assert_eq!(newest.tag, "stat-tracker-v9.9.9");
        assert!(newest.draft);
        let newest_published = fixtures
            .iter()
            .filter(|row| !row.draft)
            .max_by_key(|row| row.published_at)
            .expect("published fixtures");
        assert_eq!(newest_published.tag, "stat-tracker-v0.4.2");

        let selected =
            select_latest_tracker_version(fixtures.iter().map(|row| view(row.tag, row.draft)));
        assert_eq!(selected.as_deref(), Some("0.5.0-alpha.11"));
    }

    #[test]
    fn empty_tracker_release_list_is_none() {
        assert_eq!(select_latest_tracker_version([]), None);
    }

    #[test]
    fn a_full_first_page_does_not_hide_a_higher_version_on_the_next() {
        let page1 = vec![
            view("stat-tracker-v0.4.9", false),
            view("stat-tracker-v0.4.8", false),
        ];
        let page2 = vec![
            view("stat-tracker-v0.5.0-alpha.11", false),
            view("stat-tracker-v9.0.0", true),
            view("site-v1.0.0", false),
        ];
        let pages = [page1, page2];
        let all = concat_release_pages(&pages, 2, 10);
        assert_eq!(all.len(), 5);
        assert_eq!(
            select_latest_tracker_version(all).as_deref(),
            Some("0.5.0-alpha.11")
        );
        let only_first = concat_release_pages(&pages, 2, 1);
        assert_eq!(
            select_latest_tracker_version(only_first).as_deref(),
            Some("0.4.9")
        );
    }

    #[test]
    fn tracker_cache_hit_within_ttl_does_not_refresh() {
        let status = NostrStatus::new();
        let t0 = Instant::now();
        let (cold, refresh) = status.serve_tracker(t0);
        assert_eq!(cold, None);
        assert!(refresh, "a cold cache schedules one refresh");
        let (again, refresh_again) = status.serve_tracker(t0);
        assert_eq!(again, None);
        assert!(
            !refresh_again,
            "a refresh already in flight must not start another"
        );

        status.store_tracker_success(t0, Some("0.5.0-alpha.11".to_string()));
        let (hit, refresh_hit) = status.serve_tracker(t0 + Duration::from_secs(30));
        assert_eq!(hit.as_deref(), Some("0.5.0-alpha.11"));
        assert!(!refresh_hit);
        let (still, refresh_still) =
            status.serve_tracker(t0 + TRACKER_RELEASE_TTL - Duration::from_secs(1));
        assert_eq!(still.as_deref(), Some("0.5.0-alpha.11"));
        assert!(!refresh_still, "inside the TTL must be a cache hit");

        let (stale, refresh_stale) = status.serve_tracker(t0 + TRACKER_RELEASE_TTL);
        assert_eq!(
            stale.as_deref(),
            Some("0.5.0-alpha.11"),
            "an expired entry is still served while a refresh is scheduled"
        );
        assert!(refresh_stale);
    }

    #[test]
    fn relay_health_cache_hit_within_ttl() {
        let status = NostrStatus::new();
        let t0 = Instant::now();
        assert!(status.cached_relay(t0).is_none());
        let snap = RelayHealthSnapshot {
            configured: false,
            reachable: false,
            relay_url: None,
            extra_relay_urls: Vec::new(),
            relay_info: None,
            forum_backend: "local".to_string(),
        };
        status.store_relay(t0, snap.clone());
        assert_eq!(
            status.cached_relay(t0 + Duration::from_secs(29)).as_ref(),
            Some(&snap)
        );
        assert!(status.cached_relay(t0 + RELAY_HEALTH_TTL).is_none());
    }
}
