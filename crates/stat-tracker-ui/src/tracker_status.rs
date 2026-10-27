//! What the tracker is doing, as the window shows it.
//!
//! One [`HeaderStatus`] feeds the dot and words in the 0.6 top bar, the
//! dot and words in the 0.6 Settings strip, and the dot in the classic
//! (0.5) header, so the three cannot disagree about whether the tracker
//! runs. The classic header keeps its own text ("Last game HH:MM") and its
//! 0.5 dot colours ([`classic_status_color`]).

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
/// a game is open, amber while idle, so the dot beside "Last game HH:MM"
/// still tells the two apart.
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

/// Words in the Settings strip (spec p6: "Tracker running").
pub fn strip_label(status: HeaderStatus) -> &'static str {
    match status {
        HeaderStatus::Capturing | HeaderStatus::Waiting => "Tracker running",
        HeaderStatus::Down => "Tracker stopped",
        HeaderStatus::TokenRejected => "Website token rejected",
    }
}

pub(crate) fn header_status_for(app: &TrackerApp) -> HeaderStatus {
    let sync_token_rejected = app.saved_config.sync.as_ref().is_some_and(|sync| {
        stat_tracker::sync::auth_pause_matches(&app.data_dir, &sync.server_url, &sync.token)
    });
    classify_header_status(HeaderStatusInput {
        demo: app.fixture.is_some(),
        daemon_running: app.daemon.running(),
        game_running: app.game_running,
        sync_token_rejected,
    })
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
            assert_eq!(
                strip_label(status) == "Tracker stopped",
                stopped,
                "{status:?}"
            );
            let rejected = status == HeaderStatus::TokenRejected;
            assert_eq!(short_label(status).contains("rejected"), rejected);
            assert_eq!(long_label(status).contains("rejected"), rejected);
            assert_eq!(strip_label(status).contains("rejected"), rejected);
            for text in [short_label(status), long_label(status), strip_label(status)] {
                assert!(!text.is_empty());
                assert!(!text.chars().any(|c| c.is_ascii_digit()), "no bare time");
            }
        }
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
