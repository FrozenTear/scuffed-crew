//! The periodic sync clock: [`SyncAttempt`], [`SyncBackoff`] and
//! `Retry-After` parsing.

/// First wait after a server/network upload failure. Doubles each consecutive
/// failure until [`SYNC_BACKOFF_CAP`].
pub const SYNC_BACKOFF_BASE: std::time::Duration = std::time::Duration::from_secs(15);
/// Ceiling so a down server is retried, not forgotten, and not hammered.
pub const SYNC_BACKOFF_CAP: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How long to wait before the next periodic sync after `consecutive_failures`
/// server/network errors in a row. Zero when there have been none.
pub fn sync_backoff_delay(consecutive_failures: u32) -> std::time::Duration {
    if consecutive_failures == 0 {
        return std::time::Duration::ZERO;
    }
    // 2^(n-1), shift capped so a long outage cannot overflow the factor.
    let exp = consecutive_failures.saturating_sub(1).min(16);
    let factor = 1u32 << exp;
    SYNC_BACKOFF_BASE
        .saturating_mul(factor)
        .min(SYNC_BACKOFF_CAP)
}

/// Result of one sync attempt, for the periodic scheduler's backoff clock.
/// Shutdown's final upload does not consult this clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAttempt {
    /// The server accepted the upload.
    Uploaded,
    /// Server or network error, including HTTP 503. Back off before the next
    /// periodic attempt. `retry_after` is a floor when the server sent one.
    ServerError {
        retry_after: Option<std::time::Duration>,
    },
    /// HTTP 429. Wait, then try again. This is not a hard failure: the
    /// exponential failure streak is left unchanged and the rows stay queued.
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    /// HTTP 401 or 403 from upload or daemon-config. Stop periodic retries
    /// until the saved server URL or token changes.
    AuthRejected,
    /// HTTP 401 with code `token_expired`. Same pause as [`Self::AuthRejected`].
    /// The GUI copy is [`SYNC_TOKEN_EXPIRED_STATUS`].
    ///
    /// [`SYNC_TOKEN_EXPIRED_STATUS`]: super::SYNC_TOKEN_EXPIRED_STATUS
    TokenExpired,
    /// HTTP 413 `batch_too_large` that the pass could not shrink below
    /// (the server repeated the same caps, or sent none). Nothing in that
    /// request was written. Counts as a failure so the next pass waits.
    BatchTooLarge,
    /// HTTP 409 `stats_delete_pending`. Stop this pass and keep every row
    /// queued. Automatic sync stays off until a new token is saved. Not a
    /// failure streak.
    StatsDeletePending,
    /// Nothing to upload, or the failure was local. Leave the clock alone.
    NoServerCall,
}

/// `Retry-After` as delay-seconds or an HTTP-date (IMF-fixdate). Values that
/// do not fit in `Duration` saturate instead of panicking. A date in the past
/// is a zero wait ("retry now"). Unrecognized text is `None`.
pub fn parse_retry_after(
    value: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<std::time::Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(saturating_duration_secs(secs));
    }
    let target = chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%a %b %e %H:%M:%S %Y")
                .ok()
                .map(|naive| naive.and_utc())
        })?;
    let delta = target.signed_duration_since(now);
    if delta <= chrono::TimeDelta::zero() {
        return Some(std::time::Duration::ZERO);
    }
    Some(delta.to_std().unwrap_or(SYNC_BACKOFF_CAP))
}

fn saturating_duration_secs(secs: u64) -> std::time::Duration {
    if secs >= std::time::Duration::MAX.as_secs() {
        std::time::Duration::MAX
    } else {
        std::time::Duration::from_secs(secs)
    }
}

/// Bounded exponential backoff for periodic sync. Reset on a successful upload.
/// An empty queue does not count as success or failure.
#[derive(Debug, Clone, Default)]
pub struct SyncBackoff {
    failures: u32,
    next_attempt_at: Option<std::time::Instant>,
    /// Set on HTTP 401/403. Periodic sync stays off until
    /// [`Self::clear_auth_rejected`] (saved URL or token changed).
    auth_rejected: bool,
    /// Set on HTTP 409 `stats_delete_pending`. Periodic sync stays off until
    /// [`Self::clear_auth_rejected`] (a new token was saved). There is no
    /// timed retry: undoing the delete does not restore the old token.
    delete_pending: bool,
}

impl SyncBackoff {
    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn auth_rejected(&self) -> bool {
        self.auth_rejected
    }

    pub fn should_attempt(&self, now: std::time::Instant) -> bool {
        if self.auth_rejected || self.delete_pending {
            return false;
        }
        self.next_attempt_at.is_none_or(|at| now >= at)
    }

    /// True while a stats-delete hold is set. It does not expire on its own.
    pub fn delete_pending(&self) -> bool {
        self.delete_pending
    }

    pub fn record_failure(&mut self, now: std::time::Instant) {
        self.failures = self.failures.saturating_add(1);
        self.next_attempt_at = Some(now + sync_backoff_delay(self.failures));
    }

    pub fn record_success(&mut self) {
        self.failures = 0;
        self.next_attempt_at = None;
        self.auth_rejected = false;
        self.delete_pending = false;
    }

    /// Pause periodic sync. Does not grow the failure streak and does not
    /// schedule a retry, the next attempt waits for a credential change.
    pub fn record_auth_rejected(&mut self) {
        self.auth_rejected = true;
        self.next_attempt_at = None;
    }

    /// A saved credential change. Clears an auth pause and a stats-delete
    /// hold so the next attempt can run immediately. The caller resumes a
    /// stats-delete hold only when the saved token changed.
    pub fn clear_auth_rejected(&mut self) {
        self.auth_rejected = false;
        if self.delete_pending {
            self.delete_pending = false;
            self.next_attempt_at = None;
        }
    }

    /// HTTP 409 `stats_delete_pending`. Stop automatic retries. Does not
    /// grow the failure streak and does not schedule a later attempt.
    pub fn record_delete_pending(&mut self) {
        self.delete_pending = true;
        self.next_attempt_at = None;
    }

    /// HTTP 429. Schedule a wait of at least `retry_after` (the base delay
    /// when the server sent none), capped at [`SYNC_BACKOFF_CAP`]. Does not
    /// increment [`Self::failures`].
    pub fn record_rate_limit(
        &mut self,
        now: std::time::Instant,
        retry_after: Option<std::time::Duration>,
    ) {
        let hinted = retry_after.unwrap_or(SYNC_BACKOFF_BASE);
        self.schedule_at_least(now, hinted);
    }

    /// Raise the next-attempt time so it is at least `hint`, still capped.
    /// Used for `Retry-After` on 503 after the exponential delay is set.
    pub fn extend_for_retry_after(
        &mut self,
        now: std::time::Instant,
        retry_after: Option<std::time::Duration>,
    ) {
        let Some(hint) = retry_after else {
            return;
        };
        self.schedule_at_least(now, hint);
    }

    fn schedule_at_least(&mut self, now: std::time::Instant, hint: std::time::Duration) {
        let hinted = hint.min(SYNC_BACKOFF_CAP);
        let wait = hinted.max(self.retry_after(now));
        self.next_attempt_at = Some(now + wait);
    }

    /// Remaining delay. Zero when an attempt is allowed, and also while a
    /// stats-delete hold is set: that hold is not a timed retry.
    pub fn retry_after(&self, now: std::time::Instant) -> std::time::Duration {
        if self.auth_rejected || self.delete_pending {
            return std::time::Duration::ZERO;
        }
        match self.next_attempt_at {
            Some(at) if at > now => at.saturating_duration_since(now),
            _ => std::time::Duration::ZERO,
        }
    }

    pub fn observe(&mut self, attempt: SyncAttempt, now: std::time::Instant) {
        match attempt {
            SyncAttempt::Uploaded => self.record_success(),
            SyncAttempt::ServerError { retry_after } => {
                self.record_failure(now);
                self.extend_for_retry_after(now, retry_after);
            }
            SyncAttempt::RateLimited { retry_after } => self.record_rate_limit(now, retry_after),
            SyncAttempt::AuthRejected | SyncAttempt::TokenExpired => self.record_auth_rejected(),
            SyncAttempt::StatsDeletePending => self.record_delete_pending(),
            SyncAttempt::BatchTooLarge => self.record_failure(now),
            SyncAttempt::NoServerCall => {}
        }
    }
}
