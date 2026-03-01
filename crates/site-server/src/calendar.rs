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
    let uid = format!("event-{}@{}", event.id, host);
    let rrule_day = day_to_rrule(event.day_of_week);

    // Parse time (expected "HH:MM" format)
    let time_parts: Vec<&str> = event.time.split(':').collect();
    let hour: u32 = time_parts.first().and_then(|h| h.parse().ok()).unwrap_or(20);
    let minute: u32 = time_parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);

    // DTSTART with TZID
    let dtstart = format!("DTSTART;TZID={}:{:04}{:02}{:02}T{:02}{:02}00",
        event.timezone, 2026, 1, 1, hour, minute);

    // Calculate end time
    let end_minutes = hour * 60 + minute + event.duration_minutes;
    let end_hour = (end_minutes / 60) % 24;
    let end_min = end_minutes % 60;
    let dtend = format!("DTEND;TZID={}:{:04}{:02}{:02}T{:02}{:02}00",
        event.timezone, 2026, 1, 1, end_hour, end_min);

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
        vevent.push_str(&format!("DESCRIPTION:Team: {}\r\n", team_id));
    }

    vevent.push_str("END:VEVENT\r\n");
    vevent
}

/// Escape special characters for iCalendar text values.
fn escape_ical(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// Generate a complete ICS calendar from a list of events.
pub fn generate_ical(events: &[Event], host: &str, calendar_name: &str) -> String {
    let mut ical = String::new();
    ical.push_str("BEGIN:VCALENDAR\r\n");
    ical.push_str("VERSION:2.0\r\n");
    ical.push_str("PRODID:-//The Scuffed Crew//scuffed-site-server//EN\r\n");
    ical.push_str(&format!("X-WR-CALNAME:{}\r\n", escape_ical(calendar_name)));
    ical.push_str("CALSCALE:GREGORIAN\r\n");
    ical.push_str("METHOD:PUBLISH\r\n");

    for event in events {
        ical.push_str(&generate_vevent(event, host));
    }

    ical.push_str("END:VCALENDAR\r\n");
    ical
}
