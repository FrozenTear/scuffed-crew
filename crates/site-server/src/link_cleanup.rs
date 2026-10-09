//! Background cleanup for device-link codes.
//!
//! The server starts this loop on boot. An approved code that expires before
//! the device collects the token is revoked, and so is a denied code whose
//! token is still active. The handover secret is cleared even when nobody
//! starts another link.

use std::sync::Arc;
use std::time::Duration;

use scuffed_db::Database;

/// How often the server revokes and deletes expired device-link codes.
pub const DEVICE_LINK_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

pub fn spawn_device_link_cleanup(db: Arc<Database>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(DEVICE_LINK_CLEANUP_INTERVAL);
        // A slow pass must not queue a burst of catch-up ticks.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(error) = db.cleanup_expired_device_links().await {
                // Log the variant only. `Display` on a driver error can echo a bound value.
                tracing::error!(
                    error_kind = cleanup_error_kind(&error),
                    "device link cleanup failed"
                );
            }
        }
    });
}

/// Variant name only. Never the driver string.
fn cleanup_error_kind(error: &scuffed_db::DbError) -> &'static str {
    match error {
        scuffed_db::DbError::Surreal(_) => "surreal",
        scuffed_db::DbError::NotFound(_) => "not_found",
        scuffed_db::DbError::Conflict(_) => "conflict",
        scuffed_db::DbError::Config(_) => "config",
        scuffed_db::DbError::Timeout => "timeout",
        scuffed_db::DbError::Crypto(_) => "crypto",
    }
}

#[cfg(test)]
mod tests {
    use super::cleanup_error_kind;

    #[test]
    fn cleanup_log_kind_is_the_variant_not_the_message() {
        let error = scuffed_db::DbError::NotFound("daemon-secret".into());
        let kind = cleanup_error_kind(&error);
        assert_eq!(kind, "not_found");
        assert!(!kind.contains("daemon-secret"));
        assert!(!format!("{error}").is_empty());
        assert_eq!(cleanup_error_kind(&scuffed_db::DbError::Timeout), "timeout");
    }
}
