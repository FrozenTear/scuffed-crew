use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CreateEventRequest {
    pub title: String,
    pub day_of_week: u8,
    pub time: String,
    pub timezone: String,
    pub is_recurring: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// When true, event appears on public surfaces. Default false if omitted (client old).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_public: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttendanceEntry {
    pub member_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchAttendanceRequest {
    pub occurrence_date: String,
    pub entries: Vec<AttendanceEntry>,
}
