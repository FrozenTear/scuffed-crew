//! Opening the xdg-desktop-portal ScreenCast session: cursor modes,
//! select and start, the restore token round trip, which stream is used,
//! and the portal-unavailable warning.

use std::path::Path;
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream,
};
use ashpd::desktop::{PersistMode, Session};
use futures_util::StreamExt;

use super::policy::{is_share_declined, must_close_session};
use super::token::{read_token, token_path, write_token};
use super::{Live, arm_portal_watch, mark_portal_closed, portal_backend, pw};
use crate::capture::geom::OutputRect;
use crate::capture::outputs;

pub(super) enum OpenFail {
    Select(String),
    Other(String),
}

impl std::fmt::Display for OpenFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenFail::Select(msg) | OpenFail::Other(msg) => f.write_str(msg),
        }
    }
}

pub(super) const PORTAL_UNAVAILABLE: &str = "screencast portal is unavailable";

/// How an `AvailableCursorModes` read decides the open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CursorModesBlock {
    /// Hidden is offered. Continue into select and start.
    Ok,
    /// The portal is down, or the property could not be read, or the bit
    /// set was empty. Retry later. Do not drop the restore token.
    PortalUnavailable,
    /// A compositor answered with modes and Hidden was not one of them.
    NoHiddenCursor,
}

/// Empty flags are a dead backend, not a compositor that lacks Hidden.
/// A failed read is the same. Only a non-empty set without Hidden is
/// [`CursorModesBlock::NoHiddenCursor`].
pub(super) fn cursor_modes_block(
    read_failed: bool,
    modes_empty: bool,
    has_hidden: bool,
) -> CursorModesBlock {
    if read_failed || modes_empty {
        CursorModesBlock::PortalUnavailable
    } else if has_hidden {
        CursorModesBlock::Ok
    } else {
        CursorModesBlock::NoHiddenCursor
    }
}

/// How long a portal-unavailable warning stays quiet. The open itself
/// retries every [`START_BACKOFF`], so the warning window has to be longer
/// than that gap or every retry warns again.
pub(super) const PORTAL_WARN_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Which portal check failed. The warning episode is this kind, not the
/// error text, so a proxy that alternates messages does not warn on every
/// retry. A different kind does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PortalFail {
    Proxy,
    CursorRead,
    CursorEmpty,
}

/// Warn on the first failure, again when the failure kind changes, and
/// again after [`PORTAL_WARN_WINDOW`]. A retry of the same kind inside
/// the window does not warn.
pub(super) fn portal_should_warn(
    last: Option<(Instant, PortalFail)>,
    now: Instant,
    kind: PortalFail,
) -> bool {
    match last {
        None => true,
        Some((at, prev)) => prev != kind || now.saturating_duration_since(at) >= PORTAL_WARN_WINDOW,
    }
}

pub(super) struct PortalWarn {
    at: Instant,
    kind: PortalFail,
}

static PORTAL_WARN_AT: std::sync::Mutex<Option<PortalWarn>> = std::sync::Mutex::new(None);

pub(super) fn portal_unavailable_in(
    slot: &std::sync::Mutex<Option<PortalWarn>>,
    kind: PortalFail,
    err: &impl std::fmt::Display,
) -> OpenFail {
    let mut last = slot.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let warn = {
        let prev = last.as_ref().map(|warn| (warn.at, warn.kind));
        portal_should_warn(prev, now, kind)
    };
    if warn {
        *last = Some(PortalWarn { at: now, kind });
        tracing::warn!(error = %err, "{PORTAL_UNAVAILABLE}");
    } else {
        tracing::debug!(error = %err, "{PORTAL_UNAVAILABLE}");
    }
    OpenFail::Other(PORTAL_UNAVAILABLE.into())
}

fn portal_unavailable(kind: PortalFail, err: &impl std::fmt::Display) -> OpenFail {
    portal_unavailable_in(&PORTAL_WARN_AT, kind, err)
}

pub(super) fn clear_portal_warn_in(slot: &std::sync::Mutex<Option<PortalWarn>>) {
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub(super) fn clear_portal_warn() {
    clear_portal_warn_in(&PORTAL_WARN_AT);
}

pub(super) async fn open_session(
    data_dir: &Path,
    output_name: Option<&str>,
    max_fps: u32,
) -> Result<Live, String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    portal_backend::log_answered(&desktop);
    let token = read_token(data_dir);
    match open_once(
        data_dir,
        output_name,
        token.as_deref(),
        PersistMode::ExplicitlyRevoked,
        max_fps,
    )
    .await
    {
        Ok(live) => Ok(live),
        Err(OpenFail::Select(_)) if token.is_some() => {
            tracing::info!(
                "portal backend did not accept a persistent restore token; asking again without it"
            );
            let _ = std::fs::remove_file(token_path(data_dir));
            match open_once(
                data_dir,
                output_name,
                None,
                PersistMode::ExplicitlyRevoked,
                max_fps,
            )
            .await
            {
                Ok(live) => Ok(live),
                Err(err) => Err(err.to_string()),
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

struct SessionClose {
    session: Option<Session<Screencast>>,
}

impl SessionClose {
    fn arm(session: Session<Screencast>) -> Self {
        Self {
            session: Some(session),
        }
    }

    fn session(&self) -> &Session<Screencast> {
        self.session.as_ref().expect("portal session")
    }

    async fn fail(mut self, err: OpenFail) -> OpenFail {
        if must_close_session(self.session.is_some(), false)
            && let Some(session) = self.session.take()
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
        }
        err
    }

    fn disarm(mut self) -> Session<Screencast> {
        self.session.take().expect("portal session")
    }
}

impl Drop for SessionClose {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            });
        }
    }
}

async fn open_once(
    data_dir: &Path,
    output_name: Option<&str>,
    token: Option<&str>,
    persist: PersistMode,
    max_fps: u32,
) -> Result<Live, OpenFail> {
    let proxy = match Screencast::new().await {
        Ok(proxy) => proxy,
        Err(err) => return Err(portal_unavailable(PortalFail::Proxy, &err)),
    };
    let modes = match proxy.available_cursor_modes().await {
        Ok(modes) => {
            let block =
                cursor_modes_block(false, modes.is_empty(), modes.contains(CursorMode::Hidden));
            if block == CursorModesBlock::PortalUnavailable {
                let _ = portal_unavailable(PortalFail::CursorEmpty, &PORTAL_UNAVAILABLE);
            }
            block
        }
        Err(err) => {
            let _ = portal_unavailable(PortalFail::CursorRead, &err);
            cursor_modes_block(true, true, false)
        }
    };
    match modes {
        CursorModesBlock::Ok => {}
        CursorModesBlock::PortalUnavailable => {
            return Err(OpenFail::Other(PORTAL_UNAVAILABLE.into()));
        }
        CursorModesBlock::NoHiddenCursor => {
            return Err(OpenFail::Other(
                "portal does not offer a hidden cursor, so cursor metadata was not requested"
                    .into(),
            ));
        }
    }
    let session = match proxy.create_session(Default::default()).await {
        Ok(session) => session,
        Err(err) => return Err(OpenFail::Other(err.to_string())),
    };
    let guard = SessionClose::arm(session);
    let mut options = SelectSourcesOptions::default()
        .set_sources(enumflags2::BitFlags::from(SourceType::Monitor))
        .set_multiple(false)
        .set_cursor_mode(CursorMode::Hidden)
        .set_persist_mode(persist);
    if let Some(token) = token {
        options = options.set_restore_token(Some(token));
    }
    let selected = match proxy.select_sources(guard.session(), options).await {
        Ok(selected) => selected,
        Err(err) => {
            return Err(guard
                .fail(select_error(err.to_string(), token.is_some()))
                .await);
        }
    };
    if let Err(err) = selected.response() {
        return Err(guard
            .fail(select_error(err.to_string(), token.is_some()))
            .await);
    }
    let started = match proxy.start(guard.session(), None, Default::default()).await {
        Ok(started) => started,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    let streams = match started.response() {
        Ok(streams) => streams,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    if let Some(restore) = streams.restore_token() {
        write_token(data_dir, restore);
    }
    let outputs = outputs::output_geometry().await;
    let Some(chosen) = pick_stream(streams.streams(), output_name, &outputs) else {
        return Err(guard
            .fail(OpenFail::Other(
                "screencast start returned no stream".into(),
            ))
            .await);
    };
    let node = chosen.pipe_wire_node_id();
    tracing::info!(node, "screencast stream ready");
    let fd = match proxy
        .open_pipe_wire_remote(guard.session(), Default::default())
        .await
    {
        Ok(fd) => fd,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    let handle = match pw::open_stream(std::os::fd::AsRawFd::as_raw_fd(&fd), node, max_fps) {
        Ok(handle) => handle,
        Err(err) => return Err(guard.fail(OpenFail::Other(err)).await),
    };
    let session = guard.disarm();
    let (hold, rx) = tokio::sync::oneshot::channel();
    let portal_gen = arm_portal_watch();
    tokio::spawn(async move {
        let _proxy = proxy;
        let closed = session.receive_closed().await;
        let Ok(closed) = closed else {
            // No closed signal. Our own close is the only way this session ends.
            let _ = rx.await;
            let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            return;
        };
        let mut closed = std::pin::pin!(closed);
        tokio::select! {
            biased;
            _ = rx => {
                let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            }
            _ = closed.next() => {
                mark_portal_closed(portal_gen);
            }
        }
    });
    Ok(Live {
        _fd: fd,
        stream: Some(std::sync::Arc::new(handle)),
        close_session: Some(hold),
    })
}

pub(super) fn select_error(err: String, had_token: bool) -> OpenFail {
    // A cancel or a closed dialog is a decline. The token stays, so a later
    // launch can restore it. KDE invalidates a token by rejecting the select,
    // which is the branch below. The one warn is logged where the decline is
    // recorded, not here.
    if is_share_declined(&err) {
        return OpenFail::Other("select_sources cancelled".into());
    }
    if had_token {
        OpenFail::Select("select_sources failed".into())
    } else {
        OpenFail::Other(format!("select_sources failed: {err}"))
    }
}

fn pick_stream<'a>(
    streams: &'a [Stream],
    output_name: Option<&str>,
    outputs: &[OutputRect],
) -> Option<&'a Stream> {
    let monitors: Vec<&Stream> = streams
        .iter()
        .filter(|stream| stream.source_type() == Some(SourceType::Monitor))
        .collect();
    let pool: Vec<&Stream> = if monitors.is_empty() {
        streams.iter().collect()
    } else {
        monitors
    };
    if let Some(name) = output_name
        && let Some(output) = outputs.iter().find(|output| output.name == name)
    {
        if let Some(found) = pool
            .iter()
            .copied()
            .find(|stream| stream_covers_output(stream, output))
        {
            return Some(found);
        }
        tracing::warn!(
            output = name,
            "configured monitor was not the shared stream; using the stream the portal returned"
        );
    }
    if let Some(name) = output_name
        && let Some(found) = pool
            .iter()
            .copied()
            .find(|stream| stream.id() == Some(name) || stream.mapping_id() == Some(name))
    {
        return Some(found);
    }
    pool.first().copied().or_else(|| streams.first())
}

/// Position and size in compositor space, against the monitor the user set.
pub fn stream_covers_output(stream: &Stream, output: &OutputRect) -> bool {
    monitor_matches(stream.position(), stream.size(), output)
}

pub fn monitor_matches(
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    output: &OutputRect,
) -> bool {
    match (position, size) {
        (Some((x, y)), Some((w, h))) => {
            x == output.x
                && y == output.y
                && w == i32::try_from(output.width).unwrap_or(i32::MAX)
                && h == i32::try_from(output.height).unwrap_or(i32::MAX)
        }
        _ => false,
    }
}
