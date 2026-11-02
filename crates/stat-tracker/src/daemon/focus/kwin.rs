//! KDE Plasma on Wayland: the active window from a small KWin script.
//!
//! A normal Wayland client cannot read the active window, so the daemon
//! loads a script into KWin (`org.kde.KWin /Scripting`, `loadScript` then
//! `start`). The script reports the active window's `resourceClass` and
//! `resourceName` once when it starts and on every activation, with
//! `callDBus` to a session bus name the daemon owns ([`BUS_NAME`]). The
//! daemon reads those calls from the raw message stream (zbus through
//! ashpd, no object server and no extra crate) and keeps the last one.
//!
//! The script is loaded again every [`KWIN_REFRESH`], which also sends a
//! fresh report and brings it back after KWin restarts; a report older
//! than [`KWIN_STALE`] is unreadable. No bus, a name conflict, a rejected
//! script or no report yet is unreadable too, so reads go on, and the next
//! round tries again. Only reports whose sender owns `org.kde.KWin` count
//! ([`report_from_kwin`]). The script is unloaded at shutdown
//! ([`shutdown`]).

use super::{FocusRead, FocusSource};
use ashpd::zbus;
use futures_util::StreamExt as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The name the daemon owns on the session bus for the script's reports.
pub(crate) const BUS_NAME: &str = "io.github.FrozenTear.ScuffedStatTracker.Focus";
pub(crate) const OBJECT_PATH: &str = "/Focus";
pub(crate) const INTERFACE: &str = "io.github.FrozenTear.ScuffedStatTracker.Focus";
pub(crate) const REPORT_MEMBER: &str = "Report";
/// The script's plugin name in KWin, for loading and unloading.
pub(crate) const PLUGIN: &str = "scuffed-stat-tracker-focus";
/// The script file, under the data dir.
pub(crate) const SCRIPT_FILE: &str = "kwin_focus.js";

/// How often the script is loaded again (a fresh report each time).
pub(crate) const KWIN_REFRESH: Duration = Duration::from_secs(30);
/// A report older than this is unreadable: the script or KWin stopped.
pub(crate) const KWIN_STALE: Duration = Duration::from_secs(75);
/// Longest wait for one bus connect or KWin call.
const DBUS_TIMEOUT: Duration = Duration::from_secs(2);

/// The script KWin runs. Plasma 6 names are tried first, then Plasma 5's.
pub(crate) fn kwin_script() -> String {
    format!(
        r#"// Scuffed Stat Tracker focus gate: reports the active window.
function report(w) {{
    callDBus("{BUS_NAME}", "{OBJECT_PATH}", "{INTERFACE}", "{REPORT_MEMBER}",
        w ? String(w.resourceClass || "") : "",
        w ? String(w.resourceName || "") : "");
}}
report(workspace.activeWindow !== undefined ? workspace.activeWindow : workspace.activeClient);
(workspace.windowActivated || workspace.clientActivated).connect(report);
"#
    )
}

/// One report from the script: the window's `resourceClass` and
/// `resourceName`, both empty when no window is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KwinReport {
    pub(crate) class: String,
    pub(crate) name: String,
}

/// A `Report` call's body, two strings.
pub(crate) fn parse_kwin_report(body: (String, String)) -> KwinReport {
    KwinReport {
        class: body.0.trim().to_string(),
        name: body.1.trim().to_string(),
    }
}

/// `loadScript`'s answer: the script's id, negative when KWin refused it.
pub(crate) fn load_result(id: i32) -> Result<i32, String> {
    if id < 0 {
        Err(format!(
            "KWin refused the focus script (loadScript gave {id})"
        ))
    } else {
        Ok(id)
    }
}

/// The last report and the last error, shared by the bus task and reads.
#[derive(Debug, Default)]
pub(crate) struct KwinState {
    report: Option<(KwinReport, Instant)>,
    error: Option<String>,
}

impl KwinState {
    pub(crate) fn note_report(&mut self, report: KwinReport, now: Instant) {
        self.report = Some((report, now));
        self.error = None;
    }

    /// A failure (no bus, a refused or failed reload): the script may not
    /// run, so the last report is dropped and reads are unreadable until
    /// the next report.
    pub(crate) fn note_error(&mut self, why: String) {
        self.report = None;
        self.error = Some(why);
    }

    /// The focus as of `now`: unreadable with no report yet or a stale one;
    /// no window when the report is empty; else the window's two ids.
    pub(crate) fn read(&self, now: Instant) -> FocusRead {
        let Some((report, at)) = &self.report else {
            let why = self
                .error
                .clone()
                .unwrap_or_else(|| "no report from the KWin focus script yet".into());
            return FocusRead::Unreadable(why);
        };
        if now.saturating_duration_since(*at) > KWIN_STALE {
            return FocusRead::Unreadable("the KWin focus script's last report is stale".into());
        }
        let ids: Vec<String> = [&report.class, &report.name]
            .into_iter()
            .filter(|id| !id.is_empty())
            .cloned()
            .collect();
        if ids.is_empty() {
            FocusRead::NoWindow
        } else {
            FocusRead::Window(ids)
        }
    }
}

pub(crate) struct KwinSource {
    state: Arc<Mutex<KwinState>>,
}

impl KwinSource {
    /// A source over `state`, fed by [`start`] or by a test.
    pub(crate) fn from_state(state: Arc<Mutex<KwinState>>) -> Self {
        Self { state }
    }
}

impl FocusSource for KwinSource {
    fn name(&self) -> &'static str {
        "kwin-script"
    }

    fn read(&self) -> FocusRead {
        self.state
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .read(Instant::now())
    }
}

/// The live bus connection, for the unload at shutdown. Replaced when a
/// failed round drops the connection and a later one makes a new one.
static CONNECTION: Mutex<Option<zbus::Connection>> = Mutex::new(None);
/// The reload task, stopped at shutdown before the last unload.
static RELOADER: std::sync::OnceLock<tokio::task::AbortHandle> = std::sync::OnceLock::new();

/// Start the bus task and return the source it feeds.
pub(crate) fn start(data_dir: &std::path::Path) -> KwinSource {
    let state = Arc::new(Mutex::new(KwinState::default()));
    let task = tokio::spawn(run_script(Arc::clone(&state), data_dir.join(SCRIPT_FILE)));
    let _ = RELOADER.set(task.abort_handle());
    KwinSource::from_state(state)
}

fn note_error(state: &Mutex<KwinState>, why: String) {
    state
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .note_error(why);
}

/// Every [`KWIN_REFRESH`]: connect and own [`BUS_NAME`] if not done yet,
/// write the script, learn KWin's bus owner and reload the script. Any
/// failure is noted (unreadable, reads go on) and tried again next time,
/// with one warn line until a round works.
async fn run_script(state: Arc<Mutex<KwinState>>, script: std::path::PathBuf) {
    let owner = Arc::new(Mutex::new(None::<String>));
    let mut conn = None;
    let mut warned = false;
    loop {
        match refresh(&mut conn, &state, &owner, &script).await {
            Ok(()) => warned = false,
            Err(why) => {
                let why = redact_path(&why, &script);
                if !warned {
                    tracing::warn!(target: "scuffed_stat_tracker", reason = %why, "focus gate: the KWin focus script could not be loaded, reads go on; trying again every 30 s");
                    warned = true;
                }
                note_error(&state, why);
            }
        }
        tokio::time::sleep(KWIN_REFRESH).await;
    }
}

/// One round of [`run_script`].
async fn refresh(
    conn: &mut Option<zbus::Connection>,
    state: &Arc<Mutex<KwinState>>,
    owner: &Arc<Mutex<Option<String>>>,
    script: &std::path::Path,
) -> Result<(), String> {
    let bus = match conn {
        Some(bus) => bus.clone(),
        None => {
            let bus = match tokio::time::timeout(DBUS_TIMEOUT, zbus::Connection::session()).await {
                Ok(Ok(bus)) => bus,
                Ok(Err(err)) => return Err(format!("no session bus: {err}")),
                Err(_) => return Err("the session bus did not answer".into()),
            };
            bus.request_name(BUS_NAME)
                .await
                .map_err(|err| format!("cannot own {BUS_NAME}: {err}"))?;
            *CONNECTION.lock().unwrap_or_else(|err| err.into_inner()) = Some(bus.clone());
            let listener = tokio::spawn(listen(bus.clone(), Arc::clone(state), Arc::clone(owner)));
            *LISTENER.lock().unwrap_or_else(|err| err.into_inner()) = Some(listener.abort_handle());
            *conn = Some(bus.clone());
            bus
        }
    };
    std::fs::write(script, kwin_script())
        .map_err(|err| format!("cannot write the KWin focus script: {err}"))?;
    // A failed bus call may mean the connection died (the bus restarted),
    // and a dead one fails every later round. Drop it on any failure, so the
    // next round connects again; a round that fails for another reason (no
    // KWin yet) only costs that reconnect.
    let round = async {
        let kwin = kwin_owner(&bus).await?;
        *owner.lock().unwrap_or_else(|err| err.into_inner()) = Some(kwin);
        reload(&bus, script).await
    }
    .await;
    drop_on_error(conn, round, forget_bus)
}

/// `result` as it is, but on an error the cached connection in `slot` is
/// taken out and handed to `forget`, so the next round makes a new one.
pub(crate) fn drop_on_error<T>(
    slot: &mut Option<T>,
    result: Result<(), String>,
    forget: impl FnOnce(T),
) -> Result<(), String> {
    if result.is_err()
        && let Some(dead) = slot.take()
    {
        forget(dead);
    }
    result
}

/// Stop the listener and clear the shutdown handle of a dropped connection,
/// then close it, so its bus name is free for the next one.
fn forget_bus(bus: zbus::Connection) {
    if let Some(task) = LISTENER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        task.abort();
    }
    *CONNECTION.lock().unwrap_or_else(|err| err.into_inner()) = None;
    tokio::spawn(async move {
        let _ = tokio::time::timeout(DBUS_TIMEOUT, bus.graceful_shutdown()).await;
    });
}

/// The unique bus name that owns `org.kde.KWin` now.
async fn kwin_owner(conn: &zbus::Connection) -> Result<String, String> {
    let call = conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetNameOwner",
        &("org.kde.KWin",),
    );
    let reply = match tokio::time::timeout(DBUS_TIMEOUT, call).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(err)) => return Err(format!("no KWin on the session bus: {err}")),
        Err(_) => return Err("the session bus did not answer GetNameOwner".into()),
    };
    reply
        .body()
        .deserialize::<String>()
        .map_err(|err| format!("GetNameOwner reply: {err}"))
}

/// A report counts only when it comes from KWin itself: its sender is the
/// current owner of `org.kde.KWin`. Any other session bus client is
/// dropped.
pub(crate) fn report_from_kwin(sender: Option<&str>, kwin: Option<&str>) -> bool {
    matches!((sender, kwin), (Some(sender), Some(kwin)) if sender == kwin)
}

/// `text` with the script's full path (it sits in the home folder) cut
/// down to its file name, for logs and the status.
pub(crate) fn redact_path(text: &str, script: &std::path::Path) -> String {
    let full = script.to_string_lossy();
    if full.is_empty() {
        return text.to_string();
    }
    text.replace(full.as_ref(), SCRIPT_FILE)
}

async fn call<B>(conn: &zbus::Connection, method: &str, body: &B) -> Result<zbus::Message, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let call = conn.call_method(
        Some("org.kde.KWin"),
        "/Scripting",
        Some("org.kde.kwin.Scripting"),
        method,
        body,
    );
    match tokio::time::timeout(DBUS_TIMEOUT, call).await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(err)) => Err(format!("KWin {method}: {err}")),
        Err(_) => Err(format!("KWin {method} did not answer")),
    }
}

/// Unload the last copy, load the script and start it.
async fn reload(conn: &zbus::Connection, script: &std::path::Path) -> Result<(), String> {
    let _ = call(conn, "unloadScript", &(PLUGIN,)).await;
    let path = script.to_string_lossy().into_owned();
    let reply = call(conn, "loadScript", &(path, PLUGIN)).await?;
    let id: i32 = reply
        .body()
        .deserialize()
        .map_err(|err| format!("KWin loadScript reply: {err}"))?;
    load_result(id)?;
    call(conn, "start", &()).await?;
    Ok(())
}

async fn listen(
    conn: zbus::Connection,
    state: Arc<Mutex<KwinState>>,
    owner: Arc<Mutex<Option<String>>>,
) {
    let mut stream = zbus::MessageStream::from(&conn);
    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let is_report = header.message_type() == zbus::message::Type::MethodCall
            && header.interface().is_some_and(|i| i.as_str() == INTERFACE)
            && header.member().is_some_and(|m| m.as_str() == REPORT_MEMBER);
        if !is_report {
            continue;
        }
        let sender = header.sender().map(|s| s.as_str().to_string());
        let kwin = owner.lock().unwrap_or_else(|err| err.into_inner()).clone();
        if !report_from_kwin(sender.as_deref(), kwin.as_deref()) {
            continue;
        }
        if let Ok(body) = msg.body().deserialize::<(String, String)>() {
            state
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .note_report(parse_kwin_report(body), Instant::now());
        }
        let _ = conn.reply(&header, &()).await;
    }
}

/// Longest wait for the unload at shutdown.
const SHUTDOWN_UNLOAD_WAIT: Duration = Duration::from_secs(1);

static LISTENER: Mutex<Option<tokio::task::AbortHandle>> = Mutex::new(None);

/// Unload the script at shutdown. The reload task is stopped first, so no
/// reload can load it again after this. Bounded; nothing to unload when
/// the bus never connected.
pub(crate) async fn shutdown() {
    if let Some(task) = RELOADER.get() {
        task.abort();
    }
    if let Some(task) = LISTENER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
    {
        task.abort();
    }
    let conn = CONNECTION
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take();
    if let Some(conn) = conn {
        let unload = call(&conn, "unloadScript", &(PLUGIN,));
        let _ = tokio::time::timeout(SHUTDOWN_UNLOAD_WAIT, unload).await;
    }
}
