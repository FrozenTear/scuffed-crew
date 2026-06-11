//! Dump the local store: sessions and per-outcome match counts.
//! Usage: cargo run -p scuffed-stat-tracker --example dumpdb
//! (stop the daemon first — surrealkv is single-process)

use std::collections::BTreeMap;

use stat_tracker::{config::Config, storage::LocalStore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let store = LocalStore::open(&config.data_dir).await?;

    let matches = store.get_all_matches().await?;
    let mut by_outcome: BTreeMap<String, usize> = BTreeMap::new();
    for m in &matches {
        *by_outcome.entry(m.outcome.clone()).or_default() += 1;
    }
    println!(
        "personal_match rows: {} — by outcome: {by_outcome:?}",
        matches.len()
    );

    println!("\nsessions (newest first):");
    for s in store.get_all_sessions().await? {
        let started: chrono::DateTime<chrono::Utc> = s.started_at.into();
        let last: chrono::DateTime<chrono::Utc> = s.last_capture_at.into();
        println!(
            "  {}  {}–{}  hero={:<14} map={:<22} captures={:<3} outcome={}",
            s.session_id,
            started.with_timezone(&chrono::Local).format("%m-%d %H:%M"),
            last.with_timezone(&chrono::Local).format("%H:%M"),
            s.hero,
            s.map_name,
            s.capture_count,
            s.final_outcome
        );
    }
    Ok(())
}
