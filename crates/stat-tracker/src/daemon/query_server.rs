//! The daemon end of `stat_tracker::query`: a Unix socket in the data dir,
//! one request per connection.
//!
//! Only this user can reach it: the socket is mode 0600 in a private data
//! dir, and a connection from another uid is closed unread. A leftover
//! socket from a killed daemon is replaced at bind (a live one is not: the
//! pid guard already keeps one daemon per data dir). At most
//! [`MAX_CLIENTS`] connections are served at once; a request line must
//! arrive within [`READ_WAIT`] and the answer must be taken within
//! [`WRITE_WAIT`], so a stuck client cannot hold the list.
//!
//! The game list is built once per stored revision: the compact snapshot
//! rows, the deletes still queued in `commands/`, then
//! `games_from_snapshot`, exactly as the GUI builds it from the file.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use stat_tracker::games::model::Game;
use stat_tracker::query::proto::{self, Request, Response};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

pub(crate) const MAX_CLIENTS: usize = 4;
pub(crate) const READ_WAIT: Duration = Duration::from_secs(2);
pub(crate) const WRITE_WAIT: Duration = Duration::from_secs(5);

/// Bind the socket for `data_dir`, replacing a stale one.
pub(crate) fn bind(data_dir: &Path) -> std::io::Result<(tokio::net::UnixListener, PathBuf)> {
    let path = stat_tracker::query::socket_path(data_dir);
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(&path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "a tracker already answers on this socket",
            ));
        }
        std::fs::remove_file(&path)?;
    }
    let listener = tokio::net::UnixListener::bind(&path)?;
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, path))
}

/// The game list at one revision.
#[derive(Default)]
pub(crate) struct GamesCache {
    at: Option<(u64, Arc<Vec<Game>>)>,
}

impl GamesCache {
    async fn games(
        &mut self,
        rev: u64,
        store: &storage::LocalStore,
        data_dir: &Path,
    ) -> Result<Arc<Vec<Game>>, String> {
        if let Some((at, games)) = &self.at
            && *at == rev
        {
            return Ok(games.clone());
        }
        let mut snap = store.compact_snapshot().await.map_err(|e| e.to_string())?;
        let queued = stat_tracker::games::rows::queued_deletes(data_dir);
        stat_tracker::games::rows::drop_sessions(&mut snap, &queued);
        let games = Arc::new(stat_tracker::games::rows::games_from_snapshot(&snap));
        self.at = Some((rev, games.clone()));
        Ok(games)
    }
}

/// Serve until the listener fails or the task is dropped.
pub(crate) async fn serve(
    listener: tokio::net::UnixListener,
    store: storage::LocalStore,
    data_dir: PathBuf,
    rev: fn() -> u64,
) {
    let cache = Arc::new(tokio::sync::Mutex::new(GamesCache::default()));
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CLIENTS));
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        if !stream.peer_cred().is_ok_and(|c| c.uid() == me) {
            continue; // another user: closed unread
        }
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            let _ = tokio::time::timeout(
                WRITE_WAIT,
                stream.write_all(&proto::encode(&Response::err("busy", "too many clients"))),
            )
            .await;
            continue;
        };
        let (store, data_dir, cache) = (store.clone(), data_dir.clone(), cache.clone());
        tokio::spawn(async move {
            let _slot = slot;
            let answer = match read_request(&mut stream).await {
                Err(e) => e,
                Ok(request) => respond(&request, rev(), &cache, &store, &data_dir).await,
            };
            let _ =
                tokio::time::timeout(WRITE_WAIT, stream.write_all(&proto::encode(&answer))).await;
            let _ = stream.shutdown().await;
        });
    }
}

async fn read_request(stream: &mut tokio::net::UnixStream) -> Result<Request, Response> {
    let mut line = Vec::new();
    let mut reader = tokio::io::BufReader::new(stream).take(proto::MAX_REQUEST as u64);
    match tokio::time::timeout(READ_WAIT, reader.read_until(b'\n', &mut line)).await {
        Ok(Ok(_)) => proto::parse_request(&line),
        Ok(Err(e)) => Err(Response::err("bad_request", e.to_string())),
        Err(_) => Err(Response::err("bad_request", "no request line in time")),
    }
}

pub(crate) async fn respond(
    request: &Request,
    rev: u64,
    cache: &tokio::sync::Mutex<GamesCache>,
    store: &storage::LocalStore,
    data_dir: &Path,
) -> Response {
    match request {
        Request::Hello | Request::Rev => proto::answer(request, rev, &[]),
        Request::Totals { .. } | Request::Games { .. } => {
            match cache.lock().await.games(rev, store, data_dir).await {
                Ok(games) => proto::answer(request, rev, &games),
                Err(e) => Response::err("internal", e),
            }
        }
    }
}
