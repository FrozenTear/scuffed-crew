//! The `live_upload` key ("Upload games after each match", 0.6).
//!
//! Off keeps every game on this computer, queued as not yet synced. The
//! daemon's upload paths (after a game, the periodic and idle syncs, the
//! Tab request, and the drain at shutdown) all take the one client in
//! `DaemonCtx::sync_client`, so leaving that `None` turns every one of them
//! off: they behave as for a member with no `[sync]` at all. The reader
//! packs and the first-run player name fetch use the client made in `run`
//! before this runs, so they keep working with uploads off.
//!
//! The daemon reads the key once at start, like the other keys it holds in
//! `DaemonCtx`. Turning it back on and restarting the tracker sends the
//! queued games on the startup sync.

use crate::*;

/// The client the daemon uploads with: `client` while `live_upload` is on,
/// `None` while it is off. Logs once when it drops a working client, so a
/// log shows why nothing was sent.
pub(crate) fn upload_client(
    client: Option<sync::SyncClient>,
    live_upload: bool,
) -> Option<sync::SyncClient> {
    if live_upload {
        return client;
    }
    if client.is_some() {
        tracing::info!(
            target: "scuffed_stat_tracker",
            "uploads off (live_upload = false): games stay on this computer, queued until uploads are back on"
        );
    }
    None
}
