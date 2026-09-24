use scuffed_db::Event;

/// Map day_of_week (0=Monday..6=Sunday) to RRULE BYDAY abbreviation.
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

/// Generate a VEVENT block for a single event.
fn generate_vevent(event: &Event, host: &str) -> String {
    // Id and host are not TEXT values, so escaping is wrong here — strip
    // breaks instead so a crafted id cannot start a new property line.
    let uid = format!(
        "event-{}@{}",
        strip_ical_breaks(&event.id),
        strip_ical_breaks(host)
    );
    let rrule_day = day_to_rrule(event.day_of_week);

    // Parse time (expected "HH:MM" format)
    let time_parts: Vec<&str> = event.time.split(':').collect();
    let hour: u32 = time_parts
        .first()
        .and_then(|h| h.parse().ok())
        .unwrap_or(20);
    let minute: u32 = time_parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);

    // TZID is a parameter, not a TEXT value. Backslash-escaping does not
    // neutralize CR/LF or `;`/`:`, so only a safe IANA-name charset is emitted.
    let tzid = sanitize_tzid(&event.timezone);
    let dtstart = format!(
        "DTSTART;TZID={}:{:04}{:02}{:02}T{:02}{:02}00",
        tzid, 2026, 1, 1, hour, minute
    );

    // Calculate end time
    let end_minutes = hour * 60 + minute + event.duration_minutes;
    let end_hour = (end_minutes / 60) % 24;
    let end_min = end_minutes % 60;
    let dtend = format!(
        "DTEND;TZID={}:{:04}{:02}{:02}T{:02}{:02}00",
        tzid, 2026, 1, 1, end_hour, end_min
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
    vevent
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
        ical.push_str(&generate_vevent(event, host));
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
}
