//! Training crop sending (opt-in): its own single-flight task and its own
//! clock, apart from stats sync.
//!
//! The command tick calls [`TrainingSched::tick`]. It never awaits HTTP: a
//! pass runs on a spawned task, so a slow or failing site cannot hold up
//! recording, the command queue or stats sync. A pass never changes the
//! stats sync clock or its pause file; a refused token here only leaves a
//! training crop warning. While stats sync is paused on a refused token or
//! a stats delete, no pass runs, so a dead token is not tried twice.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::*;

/// Least time between two passes while crops wait. The site lets one
/// upload per kind through per minute after a burst of ten.
pub(crate) const TRAINING_PASS_EVERY: Duration = Duration::from_secs(60);

/// How often the consent is read again while nothing waits, so the daemon
/// learns that a kind was turned on (or off) on the site.
pub(crate) const TRAINING_CONSENT_EVERY: Duration = Duration::from_secs(10 * 60);

/// First wait after a network or server error. Doubles per failure.
pub(crate) const TRAINING_BACKOFF_BASE: Duration = Duration::from_secs(2 * 60);

/// Longest wait: after a refused token, a store that is off, or many
/// failures in a row.
pub(crate) const TRAINING_BACKOFF_CAP: Duration = Duration::from_secs(60 * 60);

/// When the next pass may start.
#[derive(Debug, Default)]
pub(crate) struct TrainingClock {
    next_at: Option<Instant>,
    failures: u32,
    consent_read_at: Option<Instant>,
}

impl TrainingClock {
    /// Take a finished pass's report at `now`.
    pub(crate) fn observe(&mut self, report: &sync::TrainingPassReport, now: Instant) {
        if report.consent_read {
            self.consent_read_at = Some(now);
        }
        let wait = match report.retry {
            sync::TrainingRetry::Normal => {
                self.failures = 0;
                TRAINING_PASS_EVERY
            }
            sync::TrainingRetry::After(after) => {
                after.clamp(TRAINING_PASS_EVERY, TRAINING_BACKOFF_CAP)
            }
            sync::TrainingRetry::Backoff => {
                self.failures = self.failures.saturating_add(1);
                let shift = self.failures.saturating_sub(1).min(8);
                (TRAINING_BACKOFF_BASE * (1u32 << shift)).min(TRAINING_BACKOFF_CAP)
            }
            sync::TrainingRetry::Pause => TRAINING_BACKOFF_CAP,
        };
        self.next_at = Some(now + wait);
    }

    /// A pass is due at `now`: the wait is over, and crops wait or the
    /// consent is old.
    pub(crate) fn due(&self, now: Instant, has_queued: bool) -> bool {
        if self.next_at.is_some_and(|at| now < at) {
            return false;
        }
        has_queued
            || self
                .consent_read_at
                .is_none_or(|at| now.saturating_duration_since(at) >= TRAINING_CONSENT_EVERY)
    }

    /// Hold off the next look while a pass runs.
    fn started(&mut self, now: Instant) {
        self.next_at = Some(now + TRAINING_PASS_EVERY);
    }

    #[cfg(test)]
    pub(crate) fn wait_left(&self, now: Instant) -> Duration {
        self.next_at
            .map_or(Duration::ZERO, |at| at.saturating_duration_since(now))
    }
}

/// The daemon's training crop sender.
#[derive(Debug, Default)]
pub(crate) struct TrainingSched {
    task: Option<tokio::task::JoinHandle<()>>,
    pub(crate) clock: Arc<Mutex<TrainingClock>>,
    /// Waiting crops were dropped since sync went off.
    forgot_without_sync: bool,
}

impl TrainingSched {
    /// One look on the command tick. Returns at once and reads no file:
    /// all file work runs on the spawned task, on blocking threads.
    ///
    /// - No sync client (uploads off): drop every waiting crop and the
    ///   cached consent once, since nothing may be sent.
    /// - `sync_paused`: stats sync is paused on a refused token or a
    ///   stats delete. Wait.
    /// - Otherwise, when no task runs, start one. It drops crops and
    ///   consent kept for another website or token, then runs a pass when
    ///   one is due.
    pub(crate) fn tick(
        &mut self,
        client: Option<&sync::SyncClient>,
        sync_paused: bool,
        data_dir: &std::path::Path,
        now: Instant,
    ) {
        if self.task.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let Some(client) = client else {
            if !self.forgot_without_sync {
                let data_dir = data_dir.to_path_buf();
                self.task = Some(tokio::task::spawn_blocking(move || {
                    sync::forget_training_crops(&data_dir);
                }));
                self.forgot_without_sync = true;
            }
            return;
        };
        self.forgot_without_sync = false;
        if sync_paused {
            return;
        }
        let client = client.clone();
        let data_dir = data_dir.to_path_buf();
        let clock = Arc::clone(&self.clock);
        self.task = Some(tokio::spawn(async move {
            let creds = client.credentials();
            let dir = data_dir.clone();
            let looked = tokio::task::spawn_blocking(move || {
                // Crops and consent kept for another website or token are
                // dropped at once, and the new site's consent is read now.
                let changed = sync::adopt_training_owner(&dir, &creds.server_url, &creds.token);
                (changed, sync::has_queued_crops(&dir))
            })
            .await;
            let Ok((changed, has_queued)) = looked else {
                return;
            };
            {
                let mut clock = clock.lock().unwrap_or_else(|e| e.into_inner());
                if changed {
                    clock.consent_read_at = None;
                    clock.next_at = None;
                }
                if !clock.due(now, has_queued) {
                    return;
                }
                clock.started(now);
            }
            let report = sync::run_training_pass(&client, &data_dir).await;
            if report.sent > 0 {
                tracing::info!(target: "scuffed_stat_tracker", sent = report.sent, "training crops sent");
            }
            clock
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .observe(&report, Instant::now());
        }));
    }

    /// The running pass, for tests.
    #[cfg(test)]
    pub(crate) fn take_task(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        self.task.take()
    }
}

/// Stats sync is paused on a refused or expired token or a stats delete.
pub(crate) fn stats_sync_paused(backoff: &std::sync::Mutex<sync::SyncBackoff>) -> bool {
    let clock = backoff.lock().unwrap_or_else(|e| e.into_inner());
    clock.auth_rejected() || clock.delete_pending()
}
