use scuffed_db::Event;

/// Map day_of_week (0=Monday..6=Sunday) to RRULE BYDAY abbreviation.
///
/// Values outside 0..=6 are legacy rows. Create and update reject them.
/// The feed still emits Monday so an old row does not drop out of the calendar.
fn day_to_rrule(day: u8) -> &'static str {
    match day {
        0 => "MO",
        1 => "TU",
        2 => "WE",
        3 => "TH",
        4 => "FR",
        5 => "SA",
        6 => "SU",
        _ => "MO",
    }
}

/// Longest duration create and update accept: 24 hours.
///
/// The schedule form omits `duration_minutes` (the handler default is 120).
/// Stored seeds and tests use 60, 90, or 120. A weekly clock-time slot longer
/// than a day is rejected so the ICS sum cannot overflow.
pub(crate) const MAX_EVENT_DURATION_MINUTES: u32 = 24 * 60;

const MINUTES_PER_DAY: u32 = 24 * 60;

/// Why a create or update clock is rejected.
pub(crate) enum ScheduleError {
    Time,
    Duration,
    EndsBeforeStart,
}

impl ScheduleError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            ScheduleError::Time => "time must be HH:MM from 00:00 to 23:59",
            ScheduleError::Duration => "duration_minutes must be from 1 to 1440",
            ScheduleError::EndsBeforeStart => "end time must be after the start time",
        }
    }
}

/// `day_of_week` stored by this API: 0 = Monday through 6 = Sunday.
///
/// Accepts the full JSON integer range. Negatives and values above 255 are
/// the same error as 7 through 255, so the route can answer 400 for all of them.
pub(crate) fn check_day_of_week(day: i128) -> Result<u8, &'static str> {
    match u8::try_from(day) {
        Ok(day) if day <= 6 => Ok(day),
        _ => Err("day_of_week must be from 0 to 6"),
    }
}

/// `HH:MM` with hour 00-23 and minute 00-59.
///
/// The admin schedule control is an HTML time input. Its value is `HH:MM`
/// (the form default is `19:00`). Seconds are not part of that value.
pub(crate) fn parse_hhmm(time: &str) -> Option<(u32, u32)> {
    let (hour, minute) = time.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    if !hour.bytes().all(|b| b.is_ascii_digit()) || !minute.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Accept a schedule the API may store.
///
/// Duration `0` ends when it starts. Anything above 1440 minutes is rejected
/// even when the sum would still fit in a `u32`.
pub(crate) fn check_event_schedule(time: &str, duration_minutes: u32) -> Result<(), ScheduleError> {
    if parse_hhmm(time).is_none() {
        return Err(ScheduleError::Time);
    }
    if duration_minutes == 0 {
        return Err(ScheduleError::EndsBeforeStart);
    }
    if duration_minutes > MAX_EVENT_DURATION_MINUTES {
        return Err(ScheduleError::Duration);
    }
    event_instants(time, duration_minutes).ok_or(ScheduleError::EndsBeforeStart)?;
    Ok(())
}

/// Start clock and end instant, with the end date rolled forward past midnight.
///
/// Returns `(start_hour, start_minute, end_day_of_january, end_hour, end_minute)`.
/// The feed anchors on 1 January, so an end that would leave that month is
/// `None` instead of wrapping the clock onto an earlier time. Addition uses
/// checked math so a huge stored duration cannot overflow.
fn event_instants(time: &str, duration_minutes: u32) -> Option<(u32, u32, u32, u32, u32)> {
    let (hour, minute) = parse_hhmm(time)?;
    let start = hour.checked_mul(60)?.checked_add(minute)?;
    let end = start.checked_add(duration_minutes)?;
    if end <= start {
        return None;
    }
    let day = 1u32.checked_add(end / MINUTES_PER_DAY)?;
    if day > 31 {
        return None;
    }
    let end_clock = end % MINUTES_PER_DAY;
    Some((hour, minute, day, end_clock / 60, end_clock % 60))
}

/// Generate a VEVENT block for a single event.
///
/// `None` when the stored clock or duration cannot be written without
/// overflow or an end that is not after the start.
fn generate_vevent(event: &Event, host: &str) -> Option<String> {
    // Id and host are not TEXT values, so escaping is wrong here. Strip
    // breaks instead so a crafted id cannot start a new property line.
    let uid = format!(
        "event-{}@{}",
        strip_ical_breaks(&event.id),
        strip_ical_breaks(host)
    );
    let rrule_day = day_to_rrule(event.day_of_week);

    let (hour, minute, end_day, end_hour, end_min) =
        event_instants(&event.time, event.duration_minutes)?;

    // TZID is a parameter, not a TEXT value. Backslash-escaping does not
    // neutralize CR/LF or `;`/`:`, so only a safe IANA-name charset is emitted.
    let tzid = sanitize_tzid(&event.timezone);
    let dtstart = format!(
        "DTSTART;TZID={}:{:04}{:02}{:02}T{:02}{:02}00",
        tzid, 2026, 1, 1, hour, minute
    );
    let dtend = format!(
        "DTEND;TZID={}:{:04}{:02}{:02}T{:02}{:02}00",
        tzid, 2026, 1, end_day, end_hour, end_min
    );

    let mut vevent = String::new();
    vevent.push_str("BEGIN:VEVENT\r\n");
    vevent.push_str(&format!("UID:{}\r\n", uid));
    vevent.push_str(&format!("{}\r\n", dtstart));
    vevent.push_str(&format!("{}\r\n", dtend));
    vevent.push_str(&format!("SUMMARY:{}\r\n", escape_ical(&event.title)));

    if event.is_recurring {
        vevent.push_str(&format!("RRULE:FREQ=WEEKLY;BYDAY={}\r\n", rrule_day));
    }

    if let Some(ref team_id) = event.team_id {
        vevent.push_str(&format!("DESCRIPTION:Team: {}\r\n", escape_ical(team_id)));
    }

    vevent.push_str("END:VEVENT\r\n");
    Some(vevent)
}

/// Escape an iCalendar TEXT value (RFC 5545).
///
/// `\\`, `;`, and `,` are backslash-escaped. CR, LF, and Unicode line
/// separators become the two-character sequence `\` + `n` so they cannot
/// start a new content line. Other C0/C1 controls are dropped.
fn escape_ical(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' | '\u{2028}' | '\u{2029}' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Drop characters that would break an iCalendar content line.
fn strip_ical_breaks(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != '\u{2028}' && *c != '\u{2029}')
        .collect()
}

/// TZID parameter value: IANA-style names only (`America/New_York`, `Etc/GMT+1`).
/// Anything else is removed. Empty input falls back to `UTC`.
fn sanitize_tzid(tz: &str) -> String {
    let cleaned: String = tz
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
        .collect();
    if cleaned.is_empty() {
        "UTC".to_string()
    } else {
        cleaned
    }
}

/// Generate a complete ICS calendar from a list of events.
pub fn generate_ical(events: &[Event], host: &str, calendar_name: &str) -> String {
    let mut ical = String::new();
    ical.push_str("BEGIN:VCALENDAR\r\n");
    ical.push_str("VERSION:2.0\r\n");
    ical.push_str("PRODID:-//Clan Platform//site-server//EN\r\n");
    ical.push_str(&format!("X-WR-CALNAME:{}\r\n", escape_ical(calendar_name)));
    ical.push_str("CALSCALE:GREGORIAN\r\n");
    ical.push_str("METHOD:PUBLISH\r\n");

    for event in events {
        if let Some(block) = generate_vevent(event, host) {
            ical.push_str(&block);
        }
    }

    ical.push_str("END:VCALENDAR\r\n");
    ical
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(title: &str, timezone: &str, team_id: Option<&str>) -> Event {
        Event {
            id: "evt1\r\nUID:injected".into(),
            title: title.into(),
            day_of_week: 0,
            time: "20:00".into(),
            timezone: timezone.into(),
            duration_minutes: 90,
            is_recurring: true,
            team_id: team_id.map(str::to_string),
            created_by: "officer".into(),
            is_active: true,
            is_public: true,
        }
    }

    /// Structural CRLF is the only break. Injected property names stay inside
    /// the value that was supposed to hold them.
    fn assert_ics_lines_intact(ics: &str) {
        let flattened = ics.replace("\r\n", "\u{0}");
        assert!(
            !flattened.contains('\r') && !flattened.contains('\n'),
            "CR/LF outside content-line endings:\n{ics}"
        );
        let lines: Vec<&str> = ics.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert!(
            lines.iter().all(|l| !l.starts_with("X-EVIL")),
            "injected property line: {lines:?}"
        );
        assert_eq!(
            lines.iter().filter(|l| **l == "BEGIN:VEVENT").count(),
            1,
            "extra VEVENT from a field break: {lines:?}"
        );
    }

    #[test]
    fn cr_lf_tzid_and_description_cannot_inject_properties() {
        let ics = generate_ical(
            &[event(
                "Scrim\r\nX-EVIL:1",
                "UTC\r\nX-EVIL:tz",
                Some("alpha\r\nDESCRIPTION:pwned"),
            )],
            "clan.example\r\nX-EVIL:host",
            "Cup\r\nX-EVIL:name",
        );
        assert_ics_lines_intact(&ics);
        let lines: Vec<&str> = ics.split("\r\n").collect();
        assert!(
            lines.contains(&"SUMMARY:Scrim\\nX-EVIL:1"),
            "title newline must be escaped, got {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("DTSTART;TZID=UTCX-EVILtz:")),
            "TZID must drop CR/LF and colon, got {lines:?}"
        );
        assert!(
            lines.contains(&"DESCRIPTION:Team: alpha\\nDESCRIPTION:pwned"),
            "team id must be TEXT-escaped, got {lines:?}"
        );
        assert!(
            lines.contains(&"X-WR-CALNAME:Cup\\nX-EVIL:name"),
            "calendar name must be TEXT-escaped, got {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("UID:event-evt1UID:injected@clan.exampleX-EVIL:host")),
            "uid/host breaks must be stripped, got {lines:?}"
        );
    }

    #[test]
    fn text_escapes_backslash_semicolon_and_comma() {
        let ics = generate_ical(
            &[event(r"A\B, C; D", "Europe/Berlin", None)],
            "clan.example",
            "Name",
        );
        assert!(ics.contains(r"SUMMARY:A\\B\, C\; D"));
        assert!(ics.contains("DTSTART;TZID=Europe/Berlin:"));
    }

    #[test]
    fn empty_or_hostile_tzid_falls_back_without_a_break() {
        let ics = generate_ical(&[event("Ok", "\r\n;:\"", None)], "clan.example", "Name");
        assert_ics_lines_intact(&ics);
        assert!(ics.contains("DTSTART;TZID=UTC:"));
    }

    /// A late start plus the default 120 minute duration crosses midnight.
    /// DTEND must be the next calendar day, so the end stays after the start.
    #[test]
    fn overnight_duration_rolls_the_end_date() {
        let mut ev = event("Late practice", "UTC", None);
        ev.time = "23:00".into();
        ev.duration_minutes = 120;
        let ics = generate_ical(&[ev], "clan.example", "Name");
        assert!(ics.contains("DTSTART;TZID=UTC:20260101T230000"), "{ics}");
        assert!(
            ics.contains("DTEND;TZID=UTC:20260102T010000"),
            "end must be the next day, got {ics}"
        );
    }

    /// `u32::MAX` minutes used to overflow `hour * 60 + minute + duration` in debug.
    #[test]
    fn absurd_duration_does_not_overflow_the_ics_end() {
        let mut ev = event("Too long", "UTC", None);
        ev.time = "20:00".into();
        ev.duration_minutes = u32::MAX;
        let ics = generate_ical(&[ev], "clan.example", "Name");
        assert!(ics.contains("BEGIN:VCALENDAR"), "{ics}");
        assert!(
            !ics.contains("BEGIN:VEVENT"),
            "a duration that cannot be placed on the feed must be omitted, got {ics}"
        );
    }

    #[test]
    fn day_of_week_accepts_monday_through_sunday_only() {
        for day in 0i128..=6 {
            assert_eq!(check_day_of_week(day).unwrap(), day as u8, "{day}");
        }
        assert_eq!(
            check_day_of_week(7).unwrap_err(),
            "day_of_week must be from 0 to 6"
        );
        assert!(check_day_of_week(99).is_err());
        assert!(check_day_of_week(-1).is_err());
        assert!(check_day_of_week(256).is_err());
    }
}
