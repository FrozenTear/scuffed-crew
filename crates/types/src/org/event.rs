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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttendanceStats {
    pub member_id: String,
    pub attended: u32,
    pub no_show: u32,
    pub excused: u32,
    pub total: u32,
}
