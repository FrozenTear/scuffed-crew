//! What the tracker is doing, as the window shows it.
//!
//! One [`HeaderStatus`] feeds the dot and words in the 0.6 top bar and the
//! dot in the 0.6 Settings strip. The strip and the Tracker service row
//! say "Tracker running" or "Tracker stopped" from whether it runs
//! ([`strip_label`]), so a rejected token never hides that. The classic
//! (0.5) header keeps its own text ("Last game HH:MM"), its 0.5 dot rules
//! ([`classify_classic_status`]: a capture the app runs is mint, a failed
//! one red) and its 0.5 dot colours ([`classic_status_color`]).

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use iced::Color;

use crate::app::TrackerApp;
use crate::model::Game;
use crate::theme;

/// What the tracker is doing, for the top bar, the Settings strip and the
/// classic header dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderStatus {
    /// The tracker runs and Overwatch is open (menus included).
    Capturing,
    /// The tracker runs and waits for Overwatch, or this is the demo.
    Waiting,
    /// The tracker service is not running.
    Down,
    /// The website answered 401 or 403 for the sync token.
    TokenRejected,
}

/// Every status, for tests that walk all of them.
pub const ALL: [HeaderStatus; 4] = [
    HeaderStatus::Capturing,
    HeaderStatus::Waiting,
    HeaderStatus::Down,
    HeaderStatus::TokenRejected,
];

#[derive(Debug, Clone, Copy)]
pub struct HeaderStatusInput {
    pub demo: bool,
    pub daemon_running: bool,
    pub game_running: bool,
    pub sync_token_rejected: bool,
}

/// A failed test capture (Capture now, Check the picture) is not part of
/// this: its error shows under Check the picture, and the tracker still
/// runs. A capture the GUI runs itself is not a game either.
pub fn classify_header_status(input: HeaderStatusInput) -> HeaderStatus {
    if input.sync_token_rejected {
        return HeaderStatus::TokenRejected;
    }
    if input.demo {
        return HeaderStatus::Waiting;
    }
    match (input.daemon_running, input.game_running) {
        (false, _) => HeaderStatus::Down,
        (true, true) => HeaderStatus::Capturing,
        (true, false) => HeaderStatus::Waiting,
    }
}

/// Dot colour in the 0.6 top bar and Settings strip.
pub fn header_status_color(status: HeaderStatus) -> Color {
    match status {
        // Running is green, whether Overwatch is open or the tracker waits
        // for it. The words say which.
        HeaderStatus::Capturing | HeaderStatus::Waiting => theme::OK,
        // Amber is kept for real warnings only.
        HeaderStatus::TokenRejected => theme::WARN,
        HeaderStatus::Down => theme::DANGER,
    }
}

/// Dot colour in the classic (0.5) header, unchanged from 0.5: mint while
/// a game is open or Capture now runs, amber while idle, red when stopped
/// or after a failed capture ([`classify_classic_status`]).
pub fn classic_status_color(status: HeaderStatus) -> Color {
    match status {
        HeaderStatus::Capturing => theme::OK,
        HeaderStatus::Waiting | HeaderStatus::TokenRejected => theme::WARN,
        HeaderStatus::Down => theme::DANGER,
    }
}

/// Top bar words on a narrow window: a short label, never a bare time.
pub fn short_label(status: HeaderStatus) -> &'static str {
    match status {
        HeaderStatus::Capturing => "In game",
        HeaderStatus::Waiting => "Ready",
        HeaderStatus::Down => "Stopped",
        HeaderStatus::TokenRejected => "Token rejected",
    }
}

/// Top bar words on a wide window, before "· last game".
pub fn long_label(status: HeaderStatus) -> &'static str {
    match status {
        HeaderStatus::Capturing => "Watching Overwatch",
        HeaderStatus::Waiting => "Tracker ready",
        HeaderStatus::Down => "Tracker stopped",
        HeaderStatus::TokenRejected => "Website token rejected",
    }
}

/// Words in the Settings strip and the Tracker service row (spec p6:
/// "Tracker running"): whether the tracker runs, whatever the website
/// does with the token. A rejected token turns the strip's dot amber, and
/// the Website account card says why.
pub fn strip_label(running: bool) -> &'static str {
    if running {
        "Tracker running"
    } else {
        "Tracker stopped"
    }
}

/// What the classic header dot reads besides [`HeaderStatusInput`].
#[derive(Debug, Clone, Copy)]
pub struct ClassicStatusInput {
    pub shared: HeaderStatusInput,
    /// Capture now runs.
    pub capturing: bool,
    /// The last test capture failed.
    pub capture_error: bool,
}

/// The classic header dot, with the 0.5 rules: a rejected token first,
/// then a failed test capture or a stopped tracker is Down, then a capture
/// the app runs or a game open is Capturing.
pub fn classify_classic_status(input: ClassicStatusInput) -> HeaderStatus {
    let shared = input.shared;
    if shared.sync_token_rejected {
        return HeaderStatus::TokenRejected;
    }
    if input.capture_error || (!shared.demo && !shared.daemon_running) {
        return HeaderStatus::Down;
    }
    let live = input.capturing || (!shared.demo && shared.daemon_running && shared.game_running);
    if live {
        HeaderStatus::Capturing
    } else {
        HeaderStatus::Waiting
    }
}

pub(crate) fn classic_status_for(app: &TrackerApp) -> HeaderStatus {
    classify_classic_status(ClassicStatusInput {
        shared: header_input(app),
        capturing: app.capturing,
        capture_error: app.preview_error.is_some(),
    })
}

fn header_input(app: &TrackerApp) -> HeaderStatusInput {
    HeaderStatusInput {
        demo: app.fixture.is_some(),
        daemon_running: app.daemon.running(),
        game_running: app.game_running,
        sync_token_rejected: sync_token_rejected(app),
    }
}

pub(crate) fn header_status_for(app: &TrackerApp) -> HeaderStatus {
    classify_header_status(header_input(app))
}

/// The website answered 401 or 403 for the saved token. The top bar, the
/// Settings strip and the Website account card all read this. It is the
/// value [`TrackerApp::refresh_token_rejected`] last read, so a frame does
/// not read `auth_pause.json`.
pub(crate) fn sync_token_rejected(app: &TrackerApp) -> bool {
    app.page_state.token_rejected
}

/// Read `auth_pause.json` for the saved pair.
fn read_token_rejected(app: &TrackerApp) -> bool {
    app.saved_config.sync.as_ref().is_some_and(|sync| {
        stat_tracker::sync::auth_pause_matches(&app.data_dir, &sync.server_url, &sync.token)
    })
}

impl TrackerApp {
    /// Re-read whether the website refused the saved token: at start, once
    /// a tick and whenever the health line is rebuilt.
    pub(crate) fn refresh_token_rejected(&mut self) {
        self.page_state.token_rejected = read_token_rejected(self);
    }

    /// Rebuild the health line and the token state after something they
    /// read changed.
    pub(crate) fn refresh_health(&mut self) {
        self.health_status = self.health_now();
        self.refresh_token_rejected();
    }
}

/// When the newest game was played, for the top bar and the Settings
/// strip: "00:40" today, "Oct 18, 23:14" on an earlier day, with the year
/// when it is not this year. `now` is the app clock.
pub fn last_game_when(games: &[Game], now: DateTime<Utc>) -> Option<String> {
    games.first().map(|g| when_in(g.played_at, now, &Local))
}

fn when_in<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let at = at.with_timezone(tz);
    let now = now.with_timezone(tz);
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else if at.year() == now.year() {
        at.format("%b %-d, %H:%M").to_string()
    } else {
        at.format("%b %-d %Y, %H:%M").to_string()
    }
}

/// The classic header's capture line. It keeps the 0.5 copy.
pub fn classic_live_status(games: &[Game]) -> String {
    if let Some(g) = games.first() {
        format!("Last game {}", crate::model::local_hm(g.played_at))
    } else {
        "Waiting for a capture".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn input(demo: bool, daemon: bool, game: bool) -> HeaderStatusInput {
        HeaderStatusInput {
            demo,
            daemon_running: daemon,
            game_running: game,
            sync_token_rejected: false,
        }
    }

    #[test]
    fn header_dot_follows_capture_state() {
        assert_eq!(
            classify_header_status(input(false, true, true)),
            HeaderStatus::Capturing
        );
        assert_eq!(header_status_color(HeaderStatus::Capturing), theme::OK);
        assert_eq!(
            classify_header_status(input(false, true, false)),
            HeaderStatus::Waiting,
            "daemon up, no game, is idle"
        );
        assert_eq!(
            classify_header_status(input(true, false, true)),
            HeaderStatus::Waiting,
            "demo mode is not a live capture, even though the fixture reports a game"
        );
        assert_eq!(
            header_status_color(HeaderStatus::Waiting),
            theme::OK,
            "running and idle (\"Tracker running\" / \"Ready\") is green"
        );
        assert_eq!(
            classify_header_status(input(false, false, false)),
            HeaderStatus::Down
        );
        assert_eq!(
            classify_header_status(input(false, false, true)),
            HeaderStatus::Down,
            "Overwatch open with no tracker is still stopped"
        );
        assert_eq!(header_status_color(HeaderStatus::Down), theme::DANGER);

        let mut rejected = input(false, true, true);
        rejected.sync_token_rejected = true;
        assert_eq!(
            classify_header_status(rejected),
            HeaderStatus::TokenRejected
        );
        assert_eq!(
            header_status_color(HeaderStatus::TokenRejected),
            theme::WARN,
            "token rejection reuses the warning colour"
        );
        assert_ne!(
            header_status_color(HeaderStatus::Waiting),
            theme::WARN,
            "amber is only for warnings, not a running tracker"
        );
        assert_ne!(
            header_status_color(HeaderStatus::Down),
            theme::OK,
            "a stopped tracker must not look running"
        );
    }

    #[test]
    fn classic_header_keeps_the_0_5_dot() {
        assert_eq!(classic_status_color(HeaderStatus::Capturing), theme::OK);
        assert_eq!(classic_status_color(HeaderStatus::Waiting), theme::WARN);
        assert_ne!(
            classic_status_color(HeaderStatus::Waiting),
            theme::OK,
            "idle must not stay the old always-mint dot"
        );
        assert_eq!(
            classic_status_color(HeaderStatus::TokenRejected),
            theme::WARN
        );
        assert_eq!(classic_status_color(HeaderStatus::Down), theme::DANGER);
    }

    #[test]
    fn bar_and_strip_agree_on_every_status() {
        for status in ALL {
            let stopped = status == HeaderStatus::Down;
            assert_eq!(short_label(status) == "Stopped", stopped, "{status:?}");
            assert_eq!(
                long_label(status) == "Tracker stopped",
                stopped,
                "{status:?}"
            );
            let rejected = status == HeaderStatus::TokenRejected;
            assert_eq!(short_label(status).contains("rejected"), rejected);
            assert_eq!(long_label(status).contains("rejected"), rejected);
            for text in [short_label(status), long_label(status)] {
                assert!(!text.is_empty());
                assert!(!text.chars().any(|c| c.is_ascii_digit()), "no bare time");
            }
        }
        assert_eq!(strip_label(true), "Tracker running");
        assert_eq!(strip_label(false), "Tracker stopped");
    }

    /// The classic dot keeps the 0.5 rules for a capture the app runs and
    /// one that failed; the 0.6 views leave both out.
    #[test]
    fn classic_dot_keeps_the_0_5_capture_rules() {
        let classic = |shared, capturing, capture_error| {
            classify_classic_status(ClassicStatusInput {
                shared,
                capturing,
                capture_error,
            })
        };
        let idle = input(false, true, false);
        assert_eq!(classic(idle, false, false), HeaderStatus::Waiting);
        assert_eq!(classic(idle, true, false), HeaderStatus::Capturing);
        assert_eq!(classic(idle, false, true), HeaderStatus::Down);
        assert_eq!(
            classic(input(true, false, false), true, false),
            HeaderStatus::Capturing
        );
        assert_eq!(
            classic(input(true, false, true), false, false),
            HeaderStatus::Waiting
        );
        assert_eq!(
            classic(input(false, true, true), false, false),
            HeaderStatus::Capturing
        );
        assert_eq!(
            classic(input(false, false, false), false, false),
            HeaderStatus::Down
        );
        let mut rejected = idle;
        rejected.sync_token_rejected = true;
        assert_eq!(classic(rejected, false, true), HeaderStatus::TokenRejected);
        assert_eq!(classify_header_status(idle), HeaderStatus::Waiting);
    }

    #[test]
    fn last_game_adds_the_day_when_not_today() {
        let tz = FixedOffset::east_opt(2 * 3600).unwrap();
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        // 22:40 UTC is 00:40 local on the 19th, the same local day as now.
        let now = at("2026-10-19T10:00:00Z");
        assert_eq!(when_in(at("2026-10-18T22:40:00Z"), now, &tz), "00:40");
        // One minute earlier is 23:59 local on the 18th: another day.
        assert_eq!(
            when_in(at("2026-10-18T21:59:00Z"), now, &tz),
            "Oct 18, 23:59"
        );
        assert_eq!(
            when_in(at("2025-12-31T20:00:00Z"), now, &tz),
            "Dec 31 2025, 22:00"
        );
        assert_eq!(last_game_when(&[], now), None);
    }

    #[test]
    fn classic_line_keeps_its_copy() {
        assert_eq!(classic_live_status(&[]), "Waiting for a capture");
    }
}
