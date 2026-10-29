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
//! rows through `games_from_snapshot`, exactly as the GUI builds it from the
//! file, keyed by the store's revision (`LocalStore::rev`, moved by every
//! write). The deletes still queued in `commands/` are taken off on every
//! request (the GUI queues them without a stored change). The server holds
//! a store handle, so shutdown stops it ([`stop`]) before the store closes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::*;

use stat_tracker::games::model::Game;
use stat_tracker::query::proto::{self, Request, Response};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};

pub(crate) const MAX_CLIENTS: usize = 4;
pub(crate) const READ_WAIT: Duration = Duration::from_secs(2);
pub(crate) const WRITE_WAIT: Duration = Duration::from_secs(5);

/// Bind the socket for `data_dir`, replacing a stale one.
pub(crate) fn bind(data_dir: &Path) -> std::io::Result<(tokio::net::UnixListener, PathBuf)> {
    let Some(path) = stat_tracker::query::socket_path(data_dir) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the data dir path is too long for a socket and XDG_RUNTIME_DIR is unset",
        ));
    };
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

/// Serve only this user: a peer we cannot identify is refused too.
pub(crate) fn admit(peer_uid: std::io::Result<u32>, me: u32) -> bool {
    peer_uid.is_ok_and(|uid| uid == me)
}

static RUNNING: std::sync::Mutex<Option<tokio::task::AbortHandle>> = std::sync::Mutex::new(None);

/// Start [`serve`] as a task that [`stop`] can end.
pub(crate) fn start(
    listener: tokio::net::UnixListener,
    store: storage::LocalStore,
    data_dir: PathBuf,
) {
    let task = tokio::spawn(serve(listener, store, data_dir));
    *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some(task.abort_handle());
}

/// End the server and every open connection (each holds a store handle),
/// and remove the socket. Shutdown calls this before the store closes.
pub(crate) fn stop(data_dir: &Path) {
    if let Some(task) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        task.abort();
    }
    if let Some(path) = stat_tracker::query::socket_path(data_dir) {
        let _ = std::fs::remove_file(path);
    }
}

/// The game list at one revision.
#[derive(Default)]
pub(crate) struct GamesCache {
    at: Option<(u64, Arc<Vec<Game>>)>,
}

impl GamesCache {
    /// The list at `rev`, built if missing. The lock is held only to look
    /// and to store, not across the store read (two clients that miss at
    /// once both build; the second result wins). The rows are read from the
    /// store on the runtime (an async read, one page at a time), the list
    /// is built on the blocking pool.
    pub(crate) async fn games(
        cache: &tokio::sync::Mutex<GamesCache>,
        rev: u64,
        store: &storage::LocalStore,
    ) -> Result<Arc<Vec<Game>>, String> {
        if let Some((at, games)) = &cache.lock().await.at
            && *at == rev
        {
            return Ok(games.clone());
        }
        let snap = store.compact_snapshot().await.map_err(|e| e.to_string())?;
        let games = tokio::task::spawn_blocking(move || {
            Arc::new(stat_tracker::games::rows::games_from_snapshot(&snap))
        })
        .await
        .map_err(|e| e.to_string())?;
        cache.lock().await.at = Some((rev, games.clone()));
        Ok(games)
    }
}

/// Serve until the task is aborted ([`stop`]). Connections are tasks of a
/// `JoinSet` owned here, so aborting this ends them too.
pub(crate) async fn serve(
    listener: tokio::net::UnixListener,
    store: storage::LocalStore,
    data_dir: PathBuf,
) {
    let cache = Arc::new(tokio::sync::Mutex::new(GamesCache::default()));
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CLIENTS));
    let me = stat_tracker::query::my_uid();
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        let mut stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                // Out of descriptors, an aborted connect: not fatal.
                tracing::warn!(target: "scuffed_stat_tracker", error = %e, "query socket accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let peer = {
            use std::os::fd::AsFd as _;
            stat_tracker::query::peer_uid(stream.as_fd())
        };
        if !admit(peer, me) {
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
        connections.spawn(async move {
            let _slot = slot;
            let answer = match read_request(&mut stream).await {
                Err(e) => e,
                Ok(request) => respond(&request, store.rev(), &cache, &store, &data_dir).await,
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
            let games = match GamesCache::games(cache, rev, store).await {
                Ok(games) => games,
                Err(e) => return Response::err("internal", e),
            };
            // A delete the GUI queued is hidden at once, as on the file path.
            let queued = stat_tracker::games::rows::queued_deletes(data_dir);
            if queued.is_empty() {
                proto::answer(request, rev, &games)
            } else {
                let kept: Vec<Game> = games
                    .iter()
                    .filter(|g| g.session_id.is_empty() || !queued.contains(&g.session_id))
                    .cloned()
                    .collect();
                proto::answer(request, rev, &kept)
            }
        }
    }
}
