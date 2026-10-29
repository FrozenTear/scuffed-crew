//! Wire format and answers, without a daemon.

use super::proto::*;
use crate::games::aggregate::{GameFilter, aggregate_filtered};
use crate::games::model::{Game, GameOcr, Outcome, Role};

fn game(sid: &str, hero: &str, role: Role, outcome: Outcome, mins: i64) -> Game {
    Game {
        session_id: sid.into(),
        hero: hero.into(),
        map_name: "Busan".into(),
        role,
        outcome,
        elims: 1,
        deaths: 1,
        assists: 1,
        damage: 1,
        healing: 1,
        mitigation: 1,
        played_at: chrono::DateTime::from_timestamp(1_800_000_000 + mins * 60, 0).unwrap(),
        edited: false,
        edited_fields: Vec::new(),
        ocr: GameOcr::default(),
        segments: Vec::new(),
        upload_reject: None,
        suspect_fields: Vec::new(),
        synced: true,
        recognizer: "cv-v8".into(),
    }
}

#[test]
fn requests_round_trip_and_bad_lines_are_refused() {
    let req = Request::Totals {
        filter: GameFilter {
            roles: vec![Role::Support],
            hero: Some("Ana".into()),
            ..GameFilter::default()
        },
    };
    let line = serde_json::to_vec(&RequestLine {
        v: PROTO,
        request: req.clone(),
    })
    .unwrap();
    assert_eq!(parse_request(&line).unwrap(), req);
    assert!(matches!(
        parse_request(b"{\"v\":1,\"q\":\"rev\"}"),
        Ok(Request::Rev)
    ));
    let bad = |b: &[u8]| match parse_request(b) {
        Err(Response::Err { code, .. }) => code,
        other => panic!("{other:?}"),
    };
    assert_eq!(bad(b"not json"), "bad_request");
    assert_eq!(bad(b"{\"v\":1,\"q\":\"drop_tables\"}"), "bad_request");
    assert_eq!(bad(b"{\"v\":2,\"q\":\"rev\"}"), "unsupported_proto");
}

#[test]
fn totals_are_the_pages_own_totals() {
    let games = vec![
        game("a", "Ana", Role::Support, Outcome::Win, 0),
        game("b", "Ana", Role::Support, Outcome::Loss, 10),
        game("c", "Reinhardt", Role::Tank, Outcome::Win, 20),
    ];
    for filter in [
        GameFilter::default(),
        GameFilter {
            roles: vec![Role::Tank],
            ..GameFilter::default()
        },
        GameFilter {
            hero: Some("Ana".into()),
            outcome: Some(Outcome::Win),
            ..GameFilter::default()
        },
    ] {
        let Response::Totals { rev, totals } = answer(
            &Request::Totals {
                filter: filter.clone(),
            },
            7,
            &games,
        ) else {
            panic!("totals");
        };
        assert_eq!(rev, 7);
        assert_eq!(totals, aggregate_filtered(&games, &filter));
        // And they survive the wire.
        let back: Response = serde_json::from_slice(&encode(&Response::Totals {
            rev,
            totals: totals.clone(),
        }))
        .unwrap();
        assert_eq!(back, Response::Totals { rev, totals });
    }
    let Response::Games { games: kept, .. } = answer(
        &Request::Games {
            filter: GameFilter {
                roles: vec![Role::Support],
                ..GameFilter::default()
            },
        },
        7,
        &games,
    ) else {
        panic!("games");
    };
    assert_eq!(kept, games[..2].to_vec());
}

#[test]
fn a_long_data_dir_gets_a_short_socket_path_and_never_a_shared_dir() {
    let short = std::path::Path::new("/home/u/.local/share/scuffed-stat-tracker");
    assert_eq!(
        super::socket_path(short),
        Some(short.join(super::SOCKET_FILE))
    );
    let long = std::path::PathBuf::from(format!("/home/u/{}", "x".repeat(120)));
    if std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| !d.is_empty()) {
        let p = super::socket_path(&long).expect("runtime dir");
        assert!(p.as_os_str().len() <= 107, "{}", p.display());
        assert!(p.starts_with(std::env::var_os("XDG_RUNTIME_DIR").unwrap()));
        assert_ne!(
            super::socket_path(&long),
            super::socket_path(&long.join("y"))
        );
    } else {
        assert_eq!(super::socket_path(&long), None, "no /tmp fallback");
    }
}

#[test]
fn a_connect_to_a_daemon_that_never_accepts_gives_up_in_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = super::socket_path(dir.path()).unwrap();
    // Bound with a backlog but never accepted: the connect queues, then the
    // read must time out.
    let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let started = std::time::Instant::now();
    let res = super::client::ask(
        dir.path(),
        &Request::Rev,
        std::time::Duration::from_millis(200),
    );
    assert!(res.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
}

/// A listener with a backlog of 0 that never accepts: the first client
/// sits in the queue, the next connect cannot queue.
fn full_listener(path: &std::path::Path) -> std::os::fd::OwnedFd {
    use std::os::fd::FromRawFd as _;
    use std::os::unix::ffi::OsStrExt as _;
    // SAFETY: plain socket(2), bind(2), listen(2) calls on a fd we own.
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        assert!(fd >= 0);
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, b) in addr.sun_path.iter_mut().zip(path.as_os_str().as_bytes()) {
            *d = *b as libc::c_char;
        }
        let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        assert_eq!(
            libc::bind(fd, (&addr as *const libc::sockaddr_un).cast(), len),
            0
        );
        assert_eq!(libc::listen(fd, 0), 0);
        std::os::fd::OwnedFd::from_raw_fd(fd)
    }
}

#[test]
fn a_daemon_with_a_full_backlog_fails_fast_without_a_waiting_thread() {
    let dir = tempfile::tempdir().unwrap();
    let path = super::socket_path(dir.path()).unwrap();
    let _listener = full_listener(&path);
    let _queued = std::os::unix::net::UnixStream::connect(&path).unwrap();
    let started = std::time::Instant::now();
    let res = super::client::ask(dir.path(), &Request::Rev, std::time::Duration::from_secs(5));
    assert!(
        matches!(&res, Err(super::client::QueryError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut),
        "{res:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "a full backlog must not wait out the timeout"
    );
}

#[test]
fn a_server_that_trickles_bytes_is_cut_off_at_the_deadline() {
    use std::io::{BufRead as _, Write as _};
    let dir = tempfile::tempdir().unwrap();
    let path = super::socket_path(dir.path()).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(s.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        for _ in 0..100 {
            if s.write_all(b" ").is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
    let started = std::time::Instant::now();
    let res = super::client::ask(
        dir.path(),
        &Request::Rev,
        std::time::Duration::from_millis(300),
    );
    assert!(res.is_err(), "{res:?}");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1500),
        "the answer has one deadline, not one per read ({:?})",
        started.elapsed()
    );
}

#[test]
fn an_answer_over_the_cap_is_refused() {
    use std::io::{BufRead as _, Write as _};
    let dir = tempfile::tempdir().unwrap();
    let path = super::socket_path(dir.path()).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufReader::new(s.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let chunk = vec![b' '; 1 << 20];
        for _ in 0..20 {
            if s.write_all(&chunk).is_err() {
                return;
            }
        }
    });
    let res = super::client::ask(
        dir.path(),
        &Request::Rev,
        std::time::Duration::from_secs(10),
    );
    assert!(
        matches!(&res, Err(super::client::QueryError::Bad(m)) if m.contains("too large")),
        "{res:?}"
    );
}
