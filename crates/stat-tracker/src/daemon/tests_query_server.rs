//! Tests for `query_server.rs`: a real socket, the lib client, and the
//! answers the GUI would compute itself from `live_snapshot.json`.

use stat_tracker::games::aggregate::{GameFilter, aggregate_filtered};
use stat_tracker::games::model::Role;
use stat_tracker::query::client::{self, QueryError};
use stat_tracker::query::proto::{Request, Response};

use super::query_server::{admit, bind, serve};
use crate::storage::{LocalStore, PersonalMatch};

fn row(sid: &str, hero: &str, role: &str, outcome: &str, secs: i64) -> PersonalMatch {
    let at = chrono::DateTime::from_timestamp(1_800_000_000 + secs, 0).unwrap();
    serde_json::from_value(serde_json::json!({
        "hero": hero, "map_name": "Busan", "game_mode": "Control", "role": role,
        "outcome": outcome, "elims": 3, "deaths": 2, "assists": 1, "damage": 100,
        "healing": 50, "mitigation": 0, "played_at": at.to_rfc3339(),
        "session_id": sid, "synced": true, "name_recognizer": "names-v1",
    }))
    .unwrap()
}

async fn ask(dir: &std::path::Path, request: Request) -> Result<Response, QueryError> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || client::ask(&dir, &request, client::SLOW))
        .await
        .unwrap()
}

#[tokio::test]
async fn the_daemon_answers_with_the_totals_the_gui_computes() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    for (i, (sid, hero, role, outcome)) in [
        ("g1", "Ana", "Support", "victory"),
        ("g1", "Ana", "Support", "victory"),
        ("g2", "Ana", "Support", "defeat"),
        ("g3", "Reinhardt", "Tank", "victory"),
    ]
    .into_iter()
    .enumerate()
    {
        store
            .insert_match(row(sid, hero, role, outcome, i as i64 * 60))
            .await
            .unwrap();
    }
    let (listener, path) = bind(dir.path()).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    tokio::spawn(serve(listener, store.clone(), dir.path().to_path_buf()));

    let Response::Hello { proto, rev: r, .. } = ask(dir.path(), Request::Hello).await.unwrap()
    else {
        panic!("hello");
    };
    assert_eq!(proto, stat_tracker::query::proto::PROTO);
    assert_eq!(r, store.rev(), "the store's own revision");

    // What the GUI does with the file.
    store.export_snapshot(dir.path()).await.unwrap();
    let file = crate::storage::read_snapshot(dir.path()).unwrap();
    let gui_games = stat_tracker::games::rows::games_from_snapshot(&file);
    assert_eq!(gui_games.len(), 3);
    for filter in [
        GameFilter::default(),
        GameFilter {
            roles: vec![Role::Support],
            ..GameFilter::default()
        },
        GameFilter {
            hero: Some("Reinhardt".into()),
            ..GameFilter::default()
        },
    ] {
        let Response::Totals { totals, .. } = ask(
            dir.path(),
            Request::Totals {
                filter: filter.clone(),
            },
        )
        .await
        .unwrap() else {
            panic!("totals");
        };
        assert_eq!(
            totals,
            aggregate_filtered(&gui_games, &filter),
            "{filter:?}"
        );
    }
    let Response::Games { games, .. } = ask(
        dir.path(),
        Request::Games {
            filter: GameFilter::default(),
        },
    )
    .await
    .unwrap() else {
        panic!("games");
    };
    assert_eq!(games, gui_games);

    // A new stored change moves the revision; the next answer sees it.
    let before = store.rev();
    store
        .insert_match(row("g4", "Ana", "Support", "victory", 600))
        .await
        .unwrap();
    assert!(store.rev() > before, "a write moves the revision");
    let Response::Totals { rev: r, totals } = ask(
        dir.path(),
        Request::Totals {
            filter: GameFilter::default(),
        },
    )
    .await
    .unwrap() else {
        panic!("totals");
    };
    assert_eq!((r, totals.record.games), (store.rev(), 4));

    // A delete the GUI queued (no stored change, no new revision) is
    // hidden at once, as on the file path.
    crate::storage::queue_command(
        dir.path(),
        &crate::storage::StoreCommand::DeleteSession {
            session_id: "g3".into(),
        },
    )
    .unwrap();
    let Response::Totals { totals, .. } = ask(
        dir.path(),
        Request::Totals {
            filter: GameFilter::default(),
        },
    )
    .await
    .unwrap() else {
        panic!("totals");
    };
    assert_eq!(totals.record.games, 3, "g3 queued for delete");

    // A second daemon cannot take a live socket.
    assert!(bind(dir.path()).is_err());
}

#[test]
fn only_this_user_is_served() {
    assert!(admit(Ok(1000), 1000));
    assert!(!admit(Ok(1001), 1000), "another uid");
    assert!(
        !admit(Err(std::io::Error::other("no cred")), 1000),
        "unknown peer"
    );
}

#[tokio::test]
async fn a_stale_socket_is_replaced_and_no_daemon_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        ask(dir.path(), Request::Rev).await,
        Err(QueryError::NotRunning)
    ));
    // A socket file left by a killed daemon: nobody listens.
    let path = stat_tracker::query::socket_path(dir.path()).unwrap();
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    assert!(matches!(
        ask(dir.path(), Request::Rev).await,
        Err(QueryError::NotRunning)
    ));
    let store = LocalStore::open(dir.path()).await.unwrap();
    let (listener, _) = bind(dir.path()).expect("stale socket replaced");
    tokio::spawn(serve(listener, store, dir.path().to_path_buf()));
    assert!(matches!(
        ask(dir.path(), Request::Rev).await,
        Ok(Response::Rev { .. })
    ));
}

#[tokio::test]
async fn a_bad_request_gets_an_error_line() {
    use std::io::{Read as _, Write as _};
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    let (listener, path) = bind(dir.path()).unwrap();
    tokio::spawn(serve(listener, store, dir.path().to_path_buf()));
    let answer = tokio::task::spawn_blocking(move || {
        let mut s = std::os::unix::net::UnixStream::connect(path).unwrap();
        s.write_all(b"{\"v\":9,\"q\":\"rev\"}\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    })
    .await
    .unwrap();
    let parsed: Response = serde_json::from_str(&answer).unwrap();
    assert!(matches!(parsed, Response::Err { code, .. } if code == "unsupported_proto"));
}

/// Request times on a copy of a real store (memfix PR 2 acceptance: games
/// under 1 s, rev under 50 ms). Ignored: needs `MEMFIX_PARITY_STORE` = a
/// data dir holding `stats.surrealkv` (copied to a temp dir first). Run in
/// a release-like profile.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn the_query_times_on_a_real_store_copy() {
    let Some(src) = std::env::var_os("MEMFIX_PARITY_STORE") else {
        panic!("set MEMFIX_PARITY_STORE");
    };
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg(std::path::Path::new(&src).join("stats.surrealkv"))
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    let store = LocalStore::open(dir.path()).await.unwrap();
    let (listener, _) = bind(dir.path()).unwrap();
    tokio::spawn(serve(listener, store.clone(), dir.path().to_path_buf()));
    let time = |req: Request| {
        let d = dir.path().to_path_buf();
        async move {
            let t = std::time::Instant::now();
            let r = ask(&d, req).await.unwrap();
            (t.elapsed().as_millis(), r)
        }
    };
    let (rev_ms, _) = time(Request::Rev).await;
    let (cold_ms, cold) = time(Request::Totals {
        filter: GameFilter::default(),
    })
    .await;
    let (warm_ms, _) = time(Request::Totals {
        filter: GameFilter::default(),
    })
    .await;
    let (games_ms, games) = time(Request::Games {
        filter: GameFilter::default(),
    })
    .await;
    let Response::Totals { totals, .. } = cold else {
        panic!("totals");
    };
    let Response::Games { games, .. } = games else {
        panic!("games");
    };
    println!(
        "{} games: rev {rev_ms} ms, totals cold {cold_ms} ms (builds the list), totals warm {warm_ms} ms, games list warm {games_ms} ms ({} games)",
        totals.record.games,
        games.len()
    );
}
