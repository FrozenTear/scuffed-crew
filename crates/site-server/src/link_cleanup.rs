//! Background cleanup for device-link codes.
//!
//! The server starts this loop on boot. `POST /api/link/start` also runs one
//! pass, so a busy host does not wait for the next tick. An approved code that
//! expires before the device collects the token is revoked, and its handover
//! secret is cleared, even when nobody starts another link.

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
            if let Err(_error) = db.cleanup_expired_device_links().await {
                // The driver string can echo a bound value. Keep this static.
                tracing::error!("device link cleanup failed");
            }
        }
    });
}
