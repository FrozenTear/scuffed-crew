//! Reader packs on the real daemon binary (alpha.7 P1c): the pid guard is
//! taken before the daemon talks to the site, a second start on the same
//! data dir exits there without a request, and the daemon that stays runs
//! the reader pack job, says once that the site has packs switched off,
//! and still stops at once on SIGTERM while that job waits for its next
//! try.
//!
//! The site is a stub on 127.0.0.1 that answers with JSON only: no pack
//! is served and no game art is involved. The daemon runs with an empty
//! HOME and XDG dirs in a temp directory and no display, so it reads no
//! real config and captures nothing. Like `run_marker_sigterm.rs` it does
//! reach the keyboard open.

use std::io::{Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const TOKEN: &str = "p1c-daemon-test-token";
const LIST_PATH: &str = "/api/tracker/packs";
const PACKS_OFF: &str = "the site has reader pack downloads turned off; the tracker will try again";

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// A daemon that cannot outlive its test: killed when this is dropped (a
/// failed assert included), and by the kernel if the test dies first.
struct Daemon {
    child: Child,
    log: PathBuf,
}

impl Daemon {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn output(&self) -> String {
        read(&self.log)
    }

    /// Wait until `ok` holds. Panics, with the daemon's output, when the
    /// daemon ends first or `timeout` passes.
    fn wait_for(&mut self, what: &str, timeout: Duration, mut ok: impl FnMut(&str) -> bool) {
        let start = Instant::now();
        while !ok(&self.output()) {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                panic!(
                    "daemon ended ({status}) before {what}; output:\n{}",
                    self.output()
                );
            }
            assert!(
                start.elapsed() < timeout,
                "timed out waiting for {what}; output:\n{}",
                self.output()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Wait for the process to end by itself.
    fn exit_within(&mut self, timeout: Duration) -> std::process::ExitStatus {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            assert!(
                start.elapsed() < timeout,
                "daemon {} still runs after {timeout:?}; output:\n{}",
                self.pid(),
                self.output()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What the stub answers the first pack list request with, once the test
/// says so.
type Held = Arc<Mutex<mpsc::Receiver<(u16, &'static str)>>>;

/// The stub site. Every request is recorded as it arrives, each on its own
/// thread, so one held answer never hides another request. The first pack
/// list request is held until `release` gets the answer to give it;
/// anything else is a 404.
struct Site {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    release: mpsc::Sender<(u16, &'static str)>,
}

fn answer(mut sock: std::net::TcpStream, seen: &Mutex<Vec<String>>, held: &Held) {
    let _ = sock.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    while !buf.windows(4).any(|window| window == b"\r\n\r\n") {
        match sock.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
    let lists_before = {
        let mut seen = seen.lock().expect("requests");
        let before = seen.iter().filter(|p| p.as_str() == LIST_PATH).count();
        seen.push(path.clone());
        before
    };
    let (status, body) = if path == LIST_PATH && lists_before == 0 {
        // Held until the test says what to answer. If the test is gone,
        // the daemon is too.
        let held = held.lock().expect("held answer");
        held.recv().unwrap_or((404, "{}"))
    } else {
        (404, r#"{"error":"not_found"}"#)
    };
    let reason = if status == 503 {
        "Service Unavailable"
    } else {
        "Not Found"
    };
    let _ = write!(
        sock,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

impl Site {
    fn start() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (release, held) = mpsc::channel();
        let held: Held = Arc::new(Mutex::new(held));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(sock) = stream else { break };
                let (seen, held) = (seen.clone(), held.clone());
                std::thread::spawn(move || answer(sock, &seen, &held));
            }
        });
        Self {
            base,
            requests,
            release,
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }

    fn list_requests(&self) -> usize {
        self.requests()
            .iter()
            .filter(|path| path.as_str() == LIST_PATH)
            .count()
    }
}

/// Scratch HOME and XDG dirs with a config that points sync at `site`.
struct Scratch {
    root: tempfile::TempDir,
    data_dir: PathBuf,
}

impl Scratch {
    fn new(site: &Site) -> Self {
        use std::os::unix::fs::OpenOptionsExt;
        let root = tempfile::tempdir().expect("tempdir");
        let config_dir = root.path().join("config").join("scuffed-stat-tracker");
        let data_dir = root.path().join("data").join("scuffed-stat-tracker");
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        // player_name is set, so the daemon's first request is the pack list.
        let config = format!(
            "data_dir = \"{}\"\nplayer_name = \"p1c\"\n\n[sync]\nserver_url = \"{}\"\ntoken = \"{TOKEN}\"\n",
            data_dir.display(),
            site.base
        );
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(config_dir.join("config.toml"))
            .expect("config.toml");
        file.write_all(config.as_bytes()).expect("write config");
        Self { root, data_dir }
    }

    /// Start the daemon binary on these dirs, logging to `<name>.out`.
    fn start_daemon(&self, name: &str) -> Daemon {
        let root = self.root.path();
        let log_path = root.join(format!("{name}.out"));
        let log = std::fs::File::create(&log_path).expect("log file");
        let mut command = Command::new(env!("CARGO_BIN_EXE_scuffed-stat-tracker"));
        command
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", root.join("home"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CACHE_HOME", root.join("cache"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().expect("log clone")))
            .stderr(Stdio::from(log));
        // SAFETY: runs between fork and exec and only makes async-signal-safe
        // calls (prctl, close_range).
        unsafe {
            command.pre_exec(|| {
                // SIGKILL when the test thread that started it ends, also
                // when the test is killed and Drop never runs.
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // Nothing the test harness holds open (other tests' sockets
                // and temp files) is inherited: every fd from 3 up is
                // closed at exec. CLOSE_RANGE_CLOEXEC, Linux 5.11.
                const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;
                libc::syscall(
                    libc::SYS_close_range,
                    3 as libc::c_uint,
                    libc::c_uint::MAX,
                    CLOSE_RANGE_CLOEXEC,
                );
                Ok(())
            });
        }
        let child = command.spawn().expect("spawn daemon");
        Daemon {
            child,
            log: log_path,
        }
    }

    fn pid_file(&self) -> PathBuf {
        self.data_dir.join("daemon.pid")
    }

    /// Staging folders, backups and tmp files under `templates/`: whatever
    /// is not the lock file, the versions file or a pack folder.
    fn pack_leftovers(&self) -> Vec<String> {
        let mut left: Vec<String> = std::fs::read_dir(self.data_dir.join("templates"))
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        left.retain(|name| {
            (name.starts_with('.') && name != ".pack-sync.lock") || name.contains(".tmp")
        });
        left
    }
}

/// The pid on the first line of `daemon.pid`.
fn pid_in(path: &Path) -> Option<u32> {
    read(path).lines().next()?.trim().parse().ok()
}

#[test]
fn a_second_start_sends_nothing_and_the_daemon_that_stays_keeps_trying_for_packs() {
    let site = Site::start();
    let scratch = Scratch::new(&site);

    // Daemon A starts and is held inside its startup pack sync.
    let mut a = scratch.start_daemon("a");
    a.wait_for("its pack list request", Duration::from_secs(60), |_| {
        site.list_requests() == 1
    });
    assert_eq!(
        pid_in(&scratch.pid_file()),
        Some(a.pid()),
        "the pid guard is taken before the startup pack sync; output:\n{}",
        a.output()
    );
    assert!(
        !scratch.data_dir.join("run.marker").exists(),
        "the crash marker is still written after the startup sync"
    );

    // Daemon B, same data dir, while A is still in that sync: it exits at
    // the pid guard and never reaches the site.
    let mut b = scratch.start_daemon("b");
    let status = b.exit_within(Duration::from_secs(60));
    let out_b = b.output();
    assert!(
        !status.success(),
        "B must refuse to start; output:\n{out_b}"
    );
    assert_eq!(status.signal(), None, "B exits by itself; output:\n{out_b}");
    assert!(
        out_b.contains("another daemon is already running"),
        "output:\n{out_b}"
    );
    assert!(
        !out_b.contains("reader pack"),
        "B never syncs; output:\n{out_b}"
    );
    assert_eq!(site.requests(), [LIST_PATH], "B sent no request");
    assert_eq!(
        pid_in(&scratch.pid_file()),
        Some(a.pid()),
        "B leaves A's pid file alone"
    );
    assert!(!out_b.contains(TOKEN));

    // The site answers A: reader packs are switched off.
    site.release
        .send((503, r#"{"error":"packs_disabled"}"#))
        .expect("answer A");
    // A is the daemon that stays. It reaches the run loop and its pack job
    // says, once, that it will try again. SIGTERM is a clean stop only
    // after the handlers are in place, so wait for both lines.
    a.wait_for(
        "its run loop and pack job",
        Duration::from_secs(60),
        |out| out.contains("signal handlers ready") && out.contains(PACKS_OFF),
    );

    // The job now sleeps for a minute. SIGTERM must not wait for it.
    // SAFETY: plain kill(2) on the child this test spawned.
    assert_eq!(unsafe { libc::kill(a.pid() as i32, libc::SIGTERM) }, 0);
    let stopping = Instant::now();
    let status = a.exit_within(Duration::from_secs(30));
    let took = stopping.elapsed();
    let out_a = a.output();
    assert!(status.success(), "exit {status:?}; output:\n{out_a}");
    assert!(
        took < Duration::from_secs(20),
        "SIGTERM took {took:?} with the pack job asleep; output:\n{out_a}"
    );
    assert_eq!(
        out_a.matches(PACKS_OFF).count(),
        1,
        "said once; output:\n{out_a}"
    );
    assert_eq!(
        site.list_requests(),
        1,
        "no second try before the wait is over"
    );
    assert_eq!(scratch.pack_leftovers(), Vec::<String>::new());
    assert!(
        !scratch.pid_file().exists(),
        "clean shutdown removes daemon.pid; output:\n{out_a}"
    );
    assert!(
        !scratch.data_dir.join("run.marker").exists(),
        "SIGTERM removes run.marker; output:\n{out_a}"
    );
    assert!(!out_a.contains(TOKEN), "output:\n{out_a}");
    for line in out_a.lines().filter(|line| line.contains("reader pack")) {
        assert!(
            !line.contains("127.0.0.1") && !line.contains("http"),
            "{line}"
        );
    }
}
