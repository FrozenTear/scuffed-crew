//! WebSocket read gate in front of strfry.
//!
//! Listens on `RELAY_GATE_LISTEN` (default `0.0.0.0:7779`) and dials strfry at
//! `RELAY_GATE_UPSTREAM` (default `ws://127.0.0.1:7777`). Compose publishes host
//! `127.0.0.1:7777` to this process, not to strfry. Same-origin `/relay` therefore
//! cannot read events the member is not allowed to see.

use std::sync::{Arc, RwLock};

use relay_policy::creds;
use relay_policy::directory::load_read_directory;
use relay_policy::limits::GateLimits;
use relay_policy::nip42::NostrAuthVerifier;
use relay_policy::proxy;
use relay_policy::read_policy::ReadDirectory;
use relay_policy::session::AuthVerifier;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "relay_policy=info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("relay-read-gate starting");
    let db_cfg = creds::enforce_or_exit();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async move {
        let connected = creds::connect_or_retry(&db_cfg).await;
        tracing::info!(url = %db_cfg.url, "read gate connected to SurrealDB");
        let db = Some(Arc::new(connected));
        if let Some(db) = db.clone() {
            creds::spawn_record_token_refresh(db, db_cfg.clone());
        }

        let directory = Arc::new(RwLock::new(ReadDirectory::default()));
        if let Some(db) = db.clone() {
            refresh(&db, &directory).await;
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let directory_reload = Arc::clone(&directory);
            let db_reload = Arc::clone(&db);
            tokio::spawn(async move {
                relay_policy::directory::coalesce_reloads(rx, || {
                    let db = Arc::clone(&db_reload);
                    let directory = Arc::clone(&directory_reload);
                    async move { refresh(&db, &directory).await }
                })
                .await;
            });
            let tx_live = tx.clone();
            let db_live = Arc::clone(&db);
            tokio::spawn(async move {
                relay_policy::directory::watch_writer_tables(db_live, tx_live).await;
            });
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(refresh_secs()));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    let _ = tx.send(());
                }
            });
        }

        let listen = std::env::var("RELAY_GATE_LISTEN").unwrap_or_else(|_| "0.0.0.0:7779".into());
        let listen: std::net::SocketAddr = match listen.parse() {
            Ok(addr) => addr,
            Err(err) => {
                tracing::error!(%listen, error = %err, "RELAY_GATE_LISTEN is not a socket address");
                eprintln!("error: RELAY_GATE_LISTEN={listen} is not a socket address");
                std::process::exit(1);
            }
        };
        let upstream =
            std::env::var("RELAY_GATE_UPSTREAM").unwrap_or_else(|_| "ws://127.0.0.1:7777".into());
        let verifier: Arc<dyn AuthVerifier> = Arc::new(NostrAuthVerifier);
        let limits = GateLimits::from_env();
        if let Err(err) = proxy::serve(listen, upstream, directory, verifier, limits).await {
            tracing::error!(error = %err, "read gate stopped");
            std::process::exit(1);
        }
    });
}

fn refresh_secs() -> u64 {
    std::env::var("RELAY_POLICY_REFRESH_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60)
}

async fn refresh(db: &scuffed_db::Database, directory: &RwLock<ReadDirectory>) {
    match load_read_directory(db).await {
        Ok(next) => {
            let readers = next.reader_count();
            let groups = next.group_count();
            *directory.write().unwrap_or_else(|err| err.into_inner()) = next;
            tracing::info!(readers, groups, "refreshed read directory");
        }
        Err(err) => {
            tracing::error!(error = %err, "read directory refresh failed; keeping the previous snapshot");
        }
    }
}
