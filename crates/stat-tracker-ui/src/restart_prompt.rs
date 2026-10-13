//! "The app was updated. Restart now?" (alpha.7 G3).
//!
//! `install.sh`, `bootstrap.sh` and the in-app updater replace
//! `<prefix>/bin/stat-tracker-gui`, but a running window keeps the old code.
//! On each Tick the GUI stats the binary "Restart now" would start (the same
//! [`crate::instance::relaunch_target`] path). Only when its mtime, inode,
//! device or size changes does it run that binary once with `--build-id`,
//! off the UI thread, with a short timeout. It asks to restart only when the
//! installed id differs from [`BUILD_ID`], the id compiled into this
//! process. The id covers the GUI and the lib code it compiles (see
//! `build_id_hash.rs`), so a release asks only when one of those changed. A
//! release that only touches the daemon's `main.rs`, the version or the
//! changelog does not ask. Most releases change some shared lib code, and
//! those do ask.
//!
//! No probe runs while an in-app update is installing, and no prompt shows
//! then. "Later" hides the prompt until the installed binary changes again.
//! A failed probe is retried with a backoff ([`PROBE_BACKOFF`]); a binary
//! that changes again gets a fresh set of tries.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime};

use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::Message;
use crate::model::Screen;
use crate::theme::{self, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_META, TEXT, TEXT_2};
use crate::update::UpdateProgress;

/// Hash of the GUI's own inputs, from `build.rs`.
pub const BUILD_ID: &str = env!("SST_GUI_BUILD_ID");

/// Arguments for the probe. `--help` is a guard for a binary from before
/// G3: it does not know `--build-id`, and without `--help` it would start
/// as a second GUI, and could even replace this one. With it, such a binary
/// prints its usage and exits; that output is not an id, so it counts as a
/// different build.
pub const PROBE_ARGS: [&str; 2] = ["--build-id", "--help"];

/// How long the probe may run before it is killed.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Waits before the next probe of the same binary after 1, 2, 3 failures.
/// A terminal install can put the GUI binary in place before the libraries
/// it needs, so the first probes may fail for a while.
pub const PROBE_BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(5),
    Duration::from_secs(30),
];

/// Failed probes of one binary before giving up on it until it changes:
/// the first try plus one retry per [`PROBE_BACKOFF`] step.
pub const MAX_PROBE_FAILURES: u8 = PROBE_BACKOFF.len() as u8 + 1;

pub const PROMPT_TEXT: &str = "The app was updated. Restart now?";
pub const RESTART_LABEL: &str = "Restart now";
pub const LATER_LABEL: &str = "Later";

/// True for a 64-character lowercase hex id.
pub fn is_build_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// What a stat of the installed binary tells apart. An atomic install
/// (`cp` to a temp file, then `mv`) changes the inode; an in-place copy
/// changes mtime or size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinStamp {
    pub modified: Option<SystemTime>,
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
}

/// One `stat` (follows symlinks). `None` when the file is missing.
pub fn stamp(path: &Path) -> Option<BinStamp> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(BinStamp {
        modified: meta.modified().ok(),
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.size(),
    })
}

/// Whether the installed binary matches this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Same,
    Differs,
    /// The probe kept failing. Neither prompt nor "no restart needed".
    Unknown,
}

/// What the update banner should say after an in-app install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartNeed {
    /// Not checked yet (or the check failed): offer Restart now, as before G3.
    Unknown,
    Needed,
    NotNeeded,
}

/// Compare the probe's output with this process's id.
pub fn verdict_for(own_id: &str, stdout: &str) -> Verdict {
    let first = stdout.lines().next().unwrap_or("").trim();
    if first == own_id {
        Verdict::Same
    } else {
        Verdict::Differs
    }
}

/// Failed probes of one stamp, and when the next try may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Retry {
    stamp: BinStamp,
    failures: u8,
    not_before: Instant,
}

/// The baseline at launch: the installed binary as stamped now, taken to be
/// this build. Not when this process runs from that same path and the file
/// there is no longer the one it was started from (replaced between exec
/// and now): then there is no baseline, and the first Tick probes it.
/// `own` is `(dev, ino)` of `/proc/self/exe`.
pub fn launch_baseline(
    installed: Option<BinStamp>,
    running_from_it: bool,
    own: Option<(u64, u64)>,
) -> Option<BinStamp> {
    let stamp = installed?;
    if running_from_it && own.is_some_and(|own| own != (stamp.dev, stamp.ino)) {
        return None;
    }
    Some(stamp)
}

/// `(dev, ino)` of the file this process was started from.
pub fn own_exe_identity() -> Option<(u64, u64)> {
    let meta = std::fs::metadata("/proc/self/exe").ok()?;
    Some((meta.dev(), meta.ino()))
}

#[derive(Debug, Clone)]
pub struct RestartWatch {
    own_id: String,
    /// Stamp of the newest verdict. At startup, the [`launch_baseline`].
    checked: Option<BinStamp>,
    verdict: Verdict,
    /// The stamp seen on the latest Tick.
    latest: Option<BinStamp>,
    /// A probe in flight, for this stamp.
    probing: Option<BinStamp>,
    /// Failures of the stamp now being retried. Reset by any other stamp.
    retry: Option<Retry>,
    /// "Later" was pressed for this stamp.
    dismissed: Option<BinStamp>,
}

impl RestartWatch {
    pub fn new(own_id: impl Into<String>, installed: Option<BinStamp>) -> Self {
        Self {
            own_id: own_id.into(),
            checked: installed,
            verdict: Verdict::Same,
            latest: installed,
            probing: None,
            retry: None,
            dismissed: None,
        }
    }

    /// Feed one Tick's stat. Returns the stamp to probe, if a probe should
    /// start now. Does nothing while an in-app update is installing: the
    /// change is picked up on the first Tick after it ends.
    pub fn on_tick(&mut self, now: Option<BinStamp>, update_running: bool) -> Option<BinStamp> {
        self.on_tick_at(now, update_running, Instant::now())
    }

    /// [`Self::on_tick`] at a given time, for the backoff.
    pub fn on_tick_at(
        &mut self,
        now: Option<BinStamp>,
        update_running: bool,
        at: Instant,
    ) -> Option<BinStamp> {
        if update_running {
            self.on_update_started();
            return None;
        }
        self.latest = now;
        if self.probing.is_some() {
            return None;
        }
        let stamp = now?;
        if Some(stamp) == self.checked {
            return None;
        }
        if let Some(retry) = self.retry
            && retry.stamp == stamp
            && at < retry.not_before
        {
            return None;
        }
        self.probing = Some(stamp);
        Some(stamp)
    }

    /// An in-app install is about to replace files. Until the next Tick
    /// after it ends, the installed build is unknown, so the banner does
    /// not say "no restart needed" from the verdict taken before it.
    pub fn on_update_started(&mut self) {
        self.latest = None;
    }

    /// Record a probe's result. `Ok` holds the binary's stdout.
    pub fn on_probe(&mut self, stamp: BinStamp, result: Result<String, String>) {
        self.on_probe_at(stamp, result, Instant::now());
    }

    /// [`Self::on_probe`] at a given time, for the backoff.
    pub fn on_probe_at(&mut self, stamp: BinStamp, result: Result<String, String>, at: Instant) {
        if self.probing == Some(stamp) {
            self.probing = None;
        }
        match result {
            Ok(stdout) => {
                self.retry = None;
                self.checked = Some(stamp);
                self.verdict = verdict_for(&self.own_id, &stdout);
            }
            Err(e) => {
                let failures = match self.retry {
                    Some(retry) if retry.stamp == stamp => retry.failures.saturating_add(1),
                    _ => 1,
                };
                tracing::warn!(error = %e, failures, "could not read the installed GUI's build id");
                if failures >= MAX_PROBE_FAILURES {
                    // Given up until the binary changes again; a new stamp
                    // starts from zero failures.
                    self.retry = None;
                    self.checked = Some(stamp);
                    self.verdict = Verdict::Unknown;
                } else {
                    let wait = PROBE_BACKOFF[usize::from(failures - 1)];
                    self.retry = Some(Retry {
                        stamp,
                        failures,
                        not_before: at + wait,
                    });
                }
            }
        }
    }

    /// Whether the installed binary needs a restart, as far as known now.
    pub fn need(&self) -> RestartNeed {
        if self.latest.is_none() || self.latest != self.checked {
            return RestartNeed::Unknown;
        }
        match self.verdict {
            Verdict::Same => RestartNeed::NotNeeded,
            Verdict::Differs => RestartNeed::Needed,
            Verdict::Unknown => RestartNeed::Unknown,
        }
    }

    /// A different build is installed and "Later" was not pressed for it.
    pub fn prompt_pending(&self) -> bool {
        self.need() == RestartNeed::Needed && self.dismissed != self.checked
    }

    /// "Later": hide until the installed binary changes again.
    pub fn later(&mut self) {
        self.dismissed = self.checked;
    }
}

/// Which of the two prompts to draw. Never both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptLayout {
    /// The update banner (Overview, Settings).
    pub banner: bool,
    /// The restart bar.
    pub bar: bool,
}

/// One prompt on screen in every state.
/// - While an update installs: the banner only, no restart bar.
/// - After an in-app install: the banner's own Restart now where it is
///   drawn, the bar on the other screens.
/// - Otherwise (Idle, Failed): a pending restart wins and the banner is
///   hidden. Its offer is stale once another build is installed, for
///   example from the banner's Copy command in a terminal.
pub fn prompt_layout(
    watch: &RestartWatch,
    progress: &UpdateProgress,
    banner_screen: bool,
) -> PromptLayout {
    let pending = watch.prompt_pending();
    match progress {
        UpdateProgress::Running => PromptLayout {
            banner: banner_screen,
            bar: false,
        },
        UpdateProgress::Succeeded(_) => PromptLayout {
            banner: banner_screen,
            bar: pending && !banner_screen,
        },
        UpdateProgress::Idle | UpdateProgress::Failed(_) => PromptLayout {
            banner: banner_screen && !pending,
            bar: pending,
        },
    }
}

/// Draw the restart bar? See [`prompt_layout`].
pub fn bar_visible(watch: &RestartWatch, progress: &UpdateProgress, banner_screen: bool) -> bool {
    prompt_layout(watch, progress, banner_screen).bar
}

/// The binary "Restart now" starts ([`crate::instance::spawn_replacement`]
/// with the same prefix), and so the one to watch.
pub fn watched_binary(prefix: &Path, own_exe: &Path) -> PathBuf {
    crate::instance::relaunch_target(Some(prefix), own_exe)
}

/// The update banner is drawn on Overview and Settings when an update is
/// known.
pub fn update_banner_on_screen(update_known: bool, screen: Screen) -> bool {
    update_known && matches!(screen, Screen::Overview | Screen::Settings)
}

/// After an in-app install the toast reads "Installed vX. Press Restart
/// now..." until the installed build is read. Returns the new toast when
/// the answer changed and that toast is still up.
pub fn retoast(
    progress: &UpdateProgress,
    toast: Option<&str>,
    before: RestartNeed,
    after: RestartNeed,
) -> Option<String> {
    let UpdateProgress::Succeeded(msg) = progress else {
        return None;
    };
    if before == after || toast != Some(crate::update::success_line(msg, before).as_str()) {
        return None;
    }
    Some(crate::update::success_line(msg, after))
}

/// The prompt's buttons. "Restart now" is the update banner's
/// [`Message::RestartGui`] path.
pub fn actions() -> [(&'static str, Message); 2] {
    [
        (RESTART_LABEL, Message::RestartGui),
        (LATER_LABEL, Message::RestartLater),
    ]
}

/// Run `path --build-id --help` and return its stdout. Killed after
/// [`PROBE_TIMEOUT`].
pub async fn probe_build_id(path: PathBuf) -> Result<String, String> {
    probe_build_id_within(path, PROBE_TIMEOUT).await
}

async fn probe_build_id_within(path: PathBuf, limit: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(&path);
    cmd.args(PROBE_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let out = match tokio::time::timeout(limit, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("could not run {}: {e}", path.display())),
        Err(_) => return Err(format!("{} --build-id timed out", path.display())),
    };
    if !out.status.success() {
        return Err(format!(
            "{} --build-id exited with {}",
            path.display(),
            out.status
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The prompt, in the toast bar's style.
pub fn bar<'a>() -> Element<'a, Message> {
    let [(restart_label, restart), (later_label, later)] = actions();
    let line = row![
        text(PROMPT_TEXT)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT)
            .width(Fill),
        button(
            text(restart_label)
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT)
        )
        .padding(Padding::from([4, 10]))
        .style(theme::chip(true))
        .on_press(restart),
        button(
            text(later_label)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2)
        )
        .padding(Padding::from([4, 10]))
        .style(theme::ghost_btn())
        .on_press(later),
    ]
    .align_y(Alignment::Center)
    .spacing(12);
    container(line)
        .padding(Padding::from([8, 14]))
        .width(Fill)
        .style(|_t| container::Style {
            background: Some(iced::Background::Color(theme::SURFACE)),
            border: iced::Border {
                color: theme::ACCENT,
                width: 1.0,
                radius: theme::inner_radius(),
            },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn stamp_n(n: u64) -> BinStamp {
        BinStamp {
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(n)),
            dev: 1,
            ino: n,
            len: 100,
        }
    }

    fn id_line(id: &str) -> Result<String, String> {
        Ok(format!("{id}\n"))
    }

    #[test]
    fn restart_prompt_shows_when_a_changed_binary_has_another_id() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        assert_eq!(watch.on_tick(Some(stamp_n(1)), false), None, "unchanged");
        assert!(!watch.prompt_pending());
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        assert_eq!(probe, stamp_n(2));
        assert_eq!(watch.need(), RestartNeed::Unknown, "while probing");
        assert_eq!(watch.on_tick(Some(stamp_n(2)), false), None, "one probe");
        watch.on_probe(probe, id_line(OTHER));
        assert_eq!(watch.need(), RestartNeed::Needed);
        assert!(watch.prompt_pending());
        assert!(bar_visible(&watch, &UpdateProgress::Idle, false));
        assert_eq!(watch.on_tick(Some(stamp_n(2)), false), None, "no re-probe");
    }

    #[test]
    fn restart_prompt_stays_hidden_when_the_id_is_the_same() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        watch.on_probe(probe, id_line(OWN));
        assert_eq!(watch.need(), RestartNeed::NotNeeded);
        assert!(!watch.prompt_pending());
        assert!(!bar_visible(&watch, &UpdateProgress::Idle, false));
    }

    #[test]
    fn restart_prompt_never_shows_or_probes_while_an_update_runs() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        assert_eq!(watch.need(), RestartNeed::NotNeeded, "as launched");
        watch.on_update_started();
        assert_eq!(watch.need(), RestartNeed::Unknown, "install started");
        assert_eq!(watch.on_tick(Some(stamp_n(2)), true), None, "no probe");
        assert_eq!(watch.need(), RestartNeed::Unknown, "install running");
        // Picked up once the install ends.
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe after");
        watch.on_probe(probe, id_line(OTHER));
        assert!(watch.prompt_pending());
        assert!(!bar_visible(&watch, &UpdateProgress::Running, false));
        assert!(bar_visible(
            &watch,
            &UpdateProgress::Failed("x".into()),
            false
        ));
    }

    #[test]
    fn restart_prompt_install_that_left_the_gui_alone_needs_no_restart() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        watch.on_update_started();
        assert_eq!(watch.on_tick(Some(stamp_n(1)), true), None);
        assert_eq!(watch.on_tick(Some(stamp_n(1)), false), None, "no probe");
        assert_eq!(watch.need(), RestartNeed::NotNeeded);
        assert!(!watch.prompt_pending());
    }

    #[test]
    fn restart_prompt_is_one_prompt_after_an_in_app_update() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        watch.on_probe(probe, id_line(OTHER));
        let done = UpdateProgress::Succeeded("Installed v1.".into());
        assert!(!bar_visible(&watch, &done, true), "the banner offers it");
        assert!(bar_visible(&watch, &done, false), "banner not on screen");
    }

    #[test]
    fn restart_prompt_later_hides_until_the_next_change() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        watch.on_probe(probe, id_line(OTHER));
        watch.later();
        assert!(!watch.prompt_pending());
        assert!(!bar_visible(&watch, &UpdateProgress::Idle, false));
        assert_eq!(watch.on_tick(Some(stamp_n(2)), false), None);
        assert!(!watch.prompt_pending(), "same binary, still hidden");
        let probe = watch.on_tick(Some(stamp_n(3)), false).expect("new binary");
        watch.on_probe(probe, id_line(OTHER));
        assert!(watch.prompt_pending(), "a later install asks again");
    }

    #[test]
    fn restart_prompt_clears_when_the_install_goes_back_to_this_build() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        watch.on_probe(probe, id_line(OTHER));
        assert!(watch.prompt_pending());
        let probe = watch.on_tick(Some(stamp_n(3)), false).expect("probe");
        assert!(!watch.prompt_pending(), "unknown while the new one is read");
        watch.on_probe(probe, id_line(OWN));
        assert!(!watch.prompt_pending());
    }

    #[test]
    fn restart_prompt_treats_a_binary_without_build_id_as_different() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        watch.on_probe(probe, Ok("stat-tracker-gui, Scuffed Crew tracker\n".into()));
        assert!(watch.prompt_pending());
    }

    fn secs(t0: Instant, s: u64) -> Instant {
        t0 + Duration::from_secs(s)
    }

    #[test]
    fn restart_prompt_retries_a_failed_probe_then_gives_up() {
        let t0 = Instant::now();
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let mut at = t0;
        for step in 0..MAX_PROBE_FAILURES {
            let probe = watch
                .on_tick_at(Some(stamp_n(2)), false, at)
                .expect("retry");
            watch.on_probe_at(probe, Err("timed out".into()), at);
            assert!(!watch.prompt_pending());
            if let Some(wait) = PROBE_BACKOFF.get(usize::from(step)) {
                at += *wait;
            }
        }
        assert_eq!(
            watch.on_tick_at(Some(stamp_n(2)), false, secs(at, 3600)),
            None,
            "gave up"
        );
        assert_eq!(watch.need(), RestartNeed::Unknown);
        let probe = watch
            .on_tick_at(Some(stamp_n(3)), false, at)
            .expect("next change");
        watch.on_probe_at(probe, id_line(OTHER), at);
        assert!(watch.prompt_pending());
    }

    #[test]
    fn restart_prompt_shows_once_the_install_finishes_after_two_failed_probes() {
        // install.sh puts the GUI binary in place before its libraries; the
        // first probes fail, then the finished install answers.
        let t0 = Instant::now();
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch
            .on_tick_at(Some(stamp_n(2)), false, t0)
            .expect("try 1");
        watch.on_probe_at(probe, Err("exited with 127".into()), t0);
        assert_eq!(
            watch.on_tick_at(Some(stamp_n(2)), false, t0),
            None,
            "backs off after one failure"
        );
        let at = secs(t0, 1);
        let probe = watch
            .on_tick_at(Some(stamp_n(2)), false, at)
            .expect("try 2");
        watch.on_probe_at(probe, Err("exited with 127".into()), at);
        assert_eq!(
            watch.on_tick_at(Some(stamp_n(2)), false, secs(t0, 3)),
            None,
            "waits 5 s after two failures"
        );
        let at = secs(t0, 6);
        let probe = watch
            .on_tick_at(Some(stamp_n(2)), false, at)
            .expect("try 3");
        watch.on_probe_at(probe, id_line(OTHER), at);
        assert!(watch.prompt_pending(), "the finished install asks");
    }

    #[test]
    fn restart_prompt_failures_do_not_carry_to_a_new_binary() {
        let t0 = Instant::now();
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = watch.on_tick_at(Some(stamp_n(2)), false, t0).expect("A");
        watch.on_probe_at(probe, Err("bad".into()), t0);
        // Binary B lands; it gets every try, with no wait left from A.
        let mut at = t0;
        for step in 0..MAX_PROBE_FAILURES - 1 {
            let probe = watch
                .on_tick_at(Some(stamp_n(3)), false, at)
                .expect("B retry");
            watch.on_probe_at(probe, Err("bad".into()), at);
            at += PROBE_BACKOFF[usize::from(step)];
        }
        let probe = watch
            .on_tick_at(Some(stamp_n(3)), false, at)
            .expect("B is not given up early");
        watch.on_probe_at(probe, id_line(OTHER), at);
        assert!(watch.prompt_pending());
    }

    #[test]
    fn restart_prompt_launch_baseline_probes_a_binary_replaced_before_start() {
        let installed = stamp_n(5);
        assert_eq!(
            launch_baseline(Some(installed), true, Some((1, 5))),
            Some(installed),
            "running the installed file"
        );
        assert_eq!(
            launch_baseline(Some(installed), true, Some((1, 4))),
            None,
            "replaced between exec and start"
        );
        assert_eq!(
            launch_baseline(Some(installed), false, Some((1, 4))),
            Some(installed),
            "a dev build elsewhere keeps the installed one as baseline"
        );
        assert_eq!(launch_baseline(None, true, Some((1, 5))), None);
        let mut watch =
            RestartWatch::new(OWN, launch_baseline(Some(installed), true, Some((1, 4))));
        let probe = watch
            .on_tick(Some(installed), false)
            .expect("probe at start");
        watch.on_probe(probe, id_line(OTHER));
        assert!(watch.prompt_pending());
    }

    #[test]
    fn restart_prompt_is_one_prompt_on_screen_in_every_state() {
        let mut pending = RestartWatch::new(OWN, Some(stamp_n(1)));
        let probe = pending.on_tick(Some(stamp_n(2)), false).expect("probe");
        pending.on_probe(probe, id_line(OTHER));
        let quiet = RestartWatch::new(OWN, Some(stamp_n(1)));
        let states = [
            UpdateProgress::Idle,
            UpdateProgress::Running,
            UpdateProgress::Succeeded("Installed v1.".into()),
            UpdateProgress::Failed("no".into()),
        ];
        for watch in [&pending, &quiet] {
            for progress in &states {
                for banner_screen in [false, true] {
                    let layout = prompt_layout(watch, progress, banner_screen);
                    assert!(
                        !(layout.banner && layout.bar),
                        "{progress:?} banner_screen={banner_screen}"
                    );
                    if !banner_screen {
                        assert!(!layout.banner);
                    }
                }
            }
        }
        // A terminal install while the banner offers "Update now".
        let idle = prompt_layout(&pending, &UpdateProgress::Idle, true);
        assert_eq!(
            idle,
            PromptLayout {
                banner: false,
                bar: true
            }
        );
        let failed = prompt_layout(&pending, &UpdateProgress::Failed("no".into()), true);
        assert!(failed.bar && !failed.banner);
        // Nothing pending: the banner stays.
        assert!(prompt_layout(&quiet, &UpdateProgress::Idle, true).banner);
    }

    #[test]
    fn restart_prompt_rechecks_when_the_binary_changed_during_a_probe() {
        let mut watch = RestartWatch::new(OWN, Some(stamp_n(1)));
        let first = watch.on_tick(Some(stamp_n(2)), false).expect("probe");
        assert_eq!(watch.on_tick(Some(stamp_n(3)), false), None, "in flight");
        watch.on_probe(first, id_line(OWN));
        assert_eq!(watch.need(), RestartNeed::Unknown, "stale verdict");
        let second = watch.on_tick(Some(stamp_n(3)), false).expect("re-probe");
        assert_eq!(second, stamp_n(3));
        watch.on_probe(second, id_line(OTHER));
        assert!(watch.prompt_pending());
    }

    #[test]
    fn restart_prompt_watches_the_binary_restart_now_starts() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("prefix");
        let installed = prefix.join("bin").join(crate::instance::GUI_BIN);
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, b"gui").unwrap();
        let plan = crate::update::UpdatePlan::Ready {
            prefix: prefix.clone(),
            tag: "stat-tracker-v1.0.0".into(),
        };
        let restart_prefix = crate::update::restart_prefix(&plan);
        assert_eq!(restart_prefix, prefix, "RestartGui's prefix");
        let running = dir.path().join("elsewhere").join(crate::instance::GUI_BIN);
        assert_eq!(watched_binary(&restart_prefix, &running), installed);
        assert_eq!(
            watched_binary(&restart_prefix, &running),
            crate::instance::relaunch_target(Some(&restart_prefix), &running)
        );
    }

    #[test]
    fn restart_prompt_banner_screens() {
        assert!(update_banner_on_screen(true, Screen::Overview));
        assert!(update_banner_on_screen(true, Screen::Settings));
        assert!(!update_banner_on_screen(true, Screen::Games));
        assert!(!update_banner_on_screen(false, Screen::Overview));
    }

    #[test]
    fn restart_prompt_retoasts_a_same_build_install() {
        let done = UpdateProgress::Succeeded("Installed v1. done".into());
        let shown = crate::update::success_line("Installed v1. done", RestartNeed::Unknown);
        assert!(shown.contains("Restart now"));
        assert_eq!(
            retoast(
                &done,
                Some(&shown),
                RestartNeed::Unknown,
                RestartNeed::NotNeeded
            )
            .as_deref(),
            Some("Installed v1. done.\nThe app itself did not change, so no restart is needed.")
        );
        assert_eq!(
            retoast(
                &done,
                Some(&shown),
                RestartNeed::Unknown,
                RestartNeed::Unknown
            ),
            None
        );
        assert_eq!(
            retoast(
                &done,
                Some("Map saved"),
                RestartNeed::Unknown,
                RestartNeed::NotNeeded
            ),
            None,
            "a later toast stays"
        );
        assert_eq!(
            retoast(
                &UpdateProgress::Idle,
                Some(&shown),
                RestartNeed::Unknown,
                RestartNeed::NotNeeded
            ),
            None
        );
    }

    #[test]
    fn restart_prompt_buttons_restart_through_restart_gui() {
        let [(restart, restart_msg), (later, later_msg)] = actions();
        assert_eq!(restart, "Restart now");
        assert!(matches!(restart_msg, Message::RestartGui));
        assert_eq!(later, "Later");
        assert!(matches!(later_msg, Message::RestartLater));
        assert_eq!(PROMPT_TEXT, "The app was updated. Restart now?");
        for copy in [PROMPT_TEXT, RESTART_LABEL, LATER_LABEL] {
            assert!(!copy.contains('\u{2014}') && !copy.contains('\u{2013}'));
        }
    }

    #[test]
    fn stamp_sees_an_atomic_replace() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("stat-tracker-gui");
        std::fs::write(&bin, b"old").unwrap();
        let before = stamp(&bin).expect("stamp");
        assert_eq!(stamp(&bin), Some(before), "stable");
        let tmp = dir.path().join(".stat-tracker-gui.new");
        std::fs::write(&tmp, b"new").unwrap();
        std::fs::rename(&tmp, &bin).unwrap();
        assert_ne!(stamp(&bin), Some(before));
        assert_eq!(stamp(&dir.path().join("missing")), None);
    }

    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[tokio::test]
    async fn probe_passes_the_probe_args_and_returns_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let bin = script(dir.path(), "gui", "echo \"$@\"");
        let out = probe_build_id(bin).await.expect("probe");
        assert_eq!(out, "--build-id --help\n");
    }

    #[tokio::test]
    async fn probe_fails_on_a_bad_exit_and_on_a_hang() {
        let dir = tempfile::tempdir().unwrap();
        let bad = script(dir.path(), "bad", "exit 3");
        assert!(probe_build_id(bad).await.is_err());
        let hang = script(dir.path(), "hang", "exec sleep 30");
        let start = std::time::Instant::now();
        let err = probe_build_id_within(hang, Duration::from_millis(200))
            .await
            .expect_err("timed out");
        assert!(err.contains("timed out"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(10));
        let missing = dir.path().join("missing");
        assert!(probe_build_id(missing).await.is_err());
    }

    #[test]
    fn embedded_build_id_is_a_build_id() {
        assert!(is_build_id(BUILD_ID), "{BUILD_ID}");
        assert!(!is_build_id("stat-tracker-gui 0.5.0"));
    }
}
