//! `live_upload` decides whether the daemon gets an upload client
//! ([`super::upload_client`]).

use crate::{main_tests::*, *};

fn client() -> Option<sync::SyncClient> {
    open_sync_client(Some(&config::SyncConfig {
        server_url: "https://crew.example".into(),
        token: "secret".into(),
    }))
}

#[test]
fn live_upload_on_keeps_the_client() {
    assert!(client().is_some(), "the test URL must make a client");
    assert!(upload_client(client(), true).is_some());
}

#[test]
fn live_upload_off_drops_the_client_so_nothing_uploads() {
    assert!(upload_client(client(), false).is_none());
}

#[test]
fn no_sync_stays_no_client_either_way() {
    assert!(upload_client(None, true).is_none());
    assert!(upload_client(None, false).is_none());
}

#[test]
fn default_config_keeps_uploads_on() {
    let cfg = config::Config::default();
    assert!(cfg.live_upload);
    assert!(upload_client(client(), cfg.live_upload).is_some());
}

/// The daemon's one upload client goes through the gate. Every upload path
/// (`try_sync` callers and the shutdown drain) reads `ctx.sync_client`.
#[test]
fn the_daemon_context_takes_the_gated_client() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let run = norm(fn_source("run"));
    assert!(
        run.contains("sync_client: upload_client(sync_client, config.live_upload),"),
        "DaemonCtx must hold the client gated on live_upload"
    );
    assert_eq!(run.matches("upload_client(").count(), 1);
}

/// Uploads off still leaves the install linked: the bans and picks reader
/// is told about the site client from before the gate, so its missing pack
/// notice says the pack is on its way, not that no account is linked.
#[test]
fn bans_and_picks_see_the_link_before_the_gate() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let run = norm(fn_source("run"));
    let linked = run
        .find("let site_linked = sync_client.is_some();")
        .expect("the link is read from the ungated client");
    let gated = run
        .find("sync_client: upload_client(sync_client, config.live_upload),")
        .expect("DaemonCtx takes the gated client");
    assert!(
        linked < gated,
        "the link must be read before the client moves into the gate"
    );
    assert!(run.contains("&ctx.data_dir, site_linked, );"));
    assert!(!run.contains("ctx.sync_client.is_some()"));
}
