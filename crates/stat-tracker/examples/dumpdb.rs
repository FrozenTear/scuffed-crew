//! Dump the local store: sessions and per-outcome match counts.
//! Usage: cargo run -p scuffed-stat-tracker --example dumpdb [repair]
//! (stop the daemon first — surrealkv is single-process)
//!
//! `repair` (alias: fix-heroes) relabels every session with the majority hero
//! across its snapshots, and settles each session's map (the session's own
//! map if known, else the snapshot majority) onto the session AND all its
//! snapshots — fixing labels frozen on a bad first capture and phantom maps
//! from the old "king" substring bug.

use std::collections::BTreeMap;

use stat_tracker::{config::Config, parse, storage, storage::LocalStore};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repair = std::env::args().any(|a| a == "repair" || a == "fix-heroes");
    let config = Config::load()?;
    let store = LocalStore::open(&config.data_dir).await?;

    if repair {
        for s in store.get_all_sessions().await? {
            let snaps = store.get_session_snapshots(&s.session_id).await?;
            if let Some(hero) = storage::majority_hero(&snaps)
                && hero != s.hero
            {
                let role = parse::guess_role_public(&hero);
                store.set_session_hero(&s.session_id, &hero, &role).await?;
                println!("hero relabeled {}: {} -> {hero}", s.session_id, s.hero);
            }
            // The session's map was set from a dedicated-region read at
            // creation — trust it over snapshot majority (which can be
            // polluted by old full-text misreads); fall back to majority.
            let map = if s.map_name.is_empty() {
                storage::majority_map(&snaps)
            } else {
                Some(s.map_name.clone())
            };
            if let Some(map) = map {
                let stray = snaps.iter().filter(|m| m.map_name != map).count();
                if stray > 0 || s.map_name.is_empty() {
                    store.set_session_map(&s.session_id, &map).await?;
                    println!(
                        "map settled {}: {} ({} snapshots corrected)",
                        s.session_id, map, stray
                    );
                }
            }
        }
        // Refresh the GUI snapshot so the repair is visible immediately.
        store.export_snapshot(&config.data_dir).await?;
    }

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
