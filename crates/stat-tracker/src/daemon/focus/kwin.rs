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
//! fresh report; a report older than [`KWIN_STALE`] is unreadable. No bus,
//! a rejected script or no report yet is unreadable too, so reads go on.
//! The script is unloaded at shutdown ([`shutdown`]).

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

    pub(crate) fn note_error(&mut self, why: String) {
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
            let why = match &self.error {
                Some(err) => format!("the KWin focus script's last report is stale ({err})"),
                None => "the KWin focus script's last report is stale".into(),
            };
            return FocusRead::Unreadable(why);
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

static CONNECTION: std::sync::OnceLock<zbus::Connection> = std::sync::OnceLock::new();

/// Start the bus task and return the source it feeds.
pub(crate) fn start(data_dir: &std::path::Path) -> KwinSource {
    let state = Arc::new(Mutex::new(KwinState::default()));
    tokio::spawn(run_script(Arc::clone(&state), data_dir.join(SCRIPT_FILE)));
    KwinSource::from_state(state)
}

fn note_error(state: &Mutex<KwinState>, why: String) {
    state
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .note_error(why);
}

async fn run_script(state: Arc<Mutex<KwinState>>, script: std::path::PathBuf) {
    let conn = match tokio::time::timeout(DBUS_TIMEOUT, zbus::Connection::session()).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(err)) => return note_error(&state, format!("no session bus: {err}")),
        Err(_) => return note_error(&state, "the session bus did not answer".into()),
    };
    if let Err(err) = conn.request_name(BUS_NAME).await {
        return note_error(&state, format!("cannot own {BUS_NAME}: {err}"));
    }
    if let Err(err) = std::fs::write(&script, kwin_script()) {
        return note_error(&state, format!("cannot write the KWin focus script: {err}"));
    }
    let _ = CONNECTION.set(conn.clone());
    tokio::spawn(listen(conn.clone(), Arc::clone(&state)));
    let mut warned = false;
    loop {
        match reload(&conn, &script).await {
            Ok(()) => warned = false,
            Err(why) => {
                if !warned {
                    tracing::warn!(target: "scuffed_stat_tracker", reason = %why, "focus gate: the KWin focus script could not be loaded, reads go on");
                    warned = true;
                }
                note_error(&state, why);
            }
        }
        tokio::time::sleep(KWIN_REFRESH).await;
    }
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

async fn listen(conn: zbus::Connection, state: Arc<Mutex<KwinState>>) {
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

/// Unload the script at shutdown. Bounded; nothing to do when it never ran.
pub(crate) async fn shutdown() {
    if let Some(conn) = CONNECTION.get() {
        let unload = call(conn, "unloadScript", &(PLUGIN,));
        let _ = tokio::time::timeout(SHUTDOWN_UNLOAD_WAIT, unload).await;
    }
}
