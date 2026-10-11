use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    pub timezone: String,
    pub duration_minutes: u32,
    pub is_recurring: bool,
    pub team_id: Option<String>,
    pub created_by: String,
    pub is_active: bool,
    /// When false, hidden from public overview, team pages, and public ICS.
    #[serde(default)]
    pub is_public: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RsvpStatus {
    Yes,
    Maybe,
    No,
}

impl std::fmt::Display for RsvpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RsvpStatus::Yes => write!(f, "yes"),
            RsvpStatus::Maybe => write!(f, "maybe"),
            RsvpStatus::No => write!(f, "no"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRsvp {
    pub id: String,
    pub member_id: String,
    pub event_id: String,
    pub status: RsvpStatus,
    pub responded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsvpSummary {
    pub event_id: String,
    pub yes_count: u32,
    pub maybe_count: u32,
    pub no_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendanceStatus {
    Attended,
    NoShow,
    Excused,
}

impl std::fmt::Display for AttendanceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttendanceStatus::Attended => write!(f, "attended"),
            AttendanceStatus::NoShow => write!(f, "no_show"),
            AttendanceStatus::Excused => write!(f, "excused"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAttendance {
    pub id: String,
    pub member_id: String,
    pub event_id: String,
    pub occurrence_date: DateTime<Utc>,
    pub status: AttendanceStatus,
    pub marked_by: String,
    pub marked_at: DateTime<Utc>,
}

/// Body of `GET /api/members/{id}/attendance/stats`.
///
/// The handler always sends these names. Counts are zero when the member has
/// no attendance rows. `total` is attended + no_show + excused. Rate is
/// computed with [`AttendanceStats::attendance_rate`], not stored on the wire.
/// Missing count fields deserialize as zero so an empty record is not a parse
/// error.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AttendanceStats {
    #[serde(default)]
    pub member_id: String,
    #[serde(default)]
    pub attended: u32,
    #[serde(default)]
    pub no_show: u32,
    #[serde(default)]
    pub excused: u32,
    #[serde(default)]
    pub total: u32,
}

impl AttendanceStats {
    /// Percent of recorded events marked attended. Zero when `total` is 0.
    pub fn attendance_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            f64::from(self.attended) * 100.0 / f64::from(self.total)
        }
    }
}

#[cfg(test)]
mod attendance_stats_tests {
    use super::AttendanceStats;

    /// JSON `Database::get_member_attendance_stats` serializes for the route.
    const WITH_ROWS: &str = r#"{"member_id":"m1","attended":4,"no_show":1,"excused":2,"total":7}"#;
    const ZERO_EVENTS: &str =
        r#"{"member_id":"m1","attended":0,"no_show":0,"excused":0,"total":0}"#;

    #[test]
    fn deserializes_server_attendance_stats_json() {
        let stats: AttendanceStats = serde_json::from_str(WITH_ROWS).unwrap();
        assert_eq!(
            stats,
            AttendanceStats {
                member_id: "m1".into(),
                attended: 4,
                no_show: 1,
                excused: 2,
                total: 7,
            }
        );
        let rate = stats.attendance_rate();
        assert!((rate - (400.0 / 7.0)).abs() < 1e-9);

        let encoded = serde_json::to_string(&stats).unwrap();
        assert_eq!(encoded, WITH_ROWS);
        assert!(!encoded.contains("total_events"));
        assert!(!encoded.contains("absent"));
        assert!(!encoded.contains("attendance_rate"));
    }

    #[test]
    fn zero_events_deserialize_to_zeros() {
        let stats: AttendanceStats = serde_json::from_str(ZERO_EVENTS).unwrap();
        assert_eq!(stats.member_id, "m1");
        assert_eq!(stats.attended, 0);
        assert_eq!(stats.no_show, 0);
        assert_eq!(stats.excused, 0);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.attendance_rate(), 0.0);
        assert_eq!(serde_json::to_string(&stats).unwrap(), ZERO_EVENTS);
    }

    #[test]
    fn omitted_counts_default_to_zero() {
        let stats: AttendanceStats = serde_json::from_str(r#"{"member_id":"m-none"}"#).unwrap();
        assert_eq!(
            stats,
            AttendanceStats {
                member_id: "m-none".into(),
                ..AttendanceStats::default()
            }
        );
        assert_eq!(stats.attendance_rate(), 0.0);

        let empty: AttendanceStats = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, AttendanceStats::default());
    }
}
