pub mod card;
pub mod create;

pub use card::PollCard;
pub use create::PollCreate;

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PollData {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub options: Vec<String>,
    pub close_at: Option<String>,
    pub allow_multiple: bool,
    pub created_by: String,
    pub created_at: String,
    pub is_active: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PollResultsData {
    pub poll: PollData,
    pub votes: Vec<PollOptionResultData>,
    pub total_votes: u32,
    pub my_votes: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct PollOptionResultData {
    pub option_index: u32,
    pub label: String,
    pub count: u32,
}

/// The close date of a poll, formatted by the page: the machine value for
/// `<time datetime>` and the visible text ("16 Nov 2026").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PollCloseDate {
    pub datetime: String,
    pub text: String,
}

/// "1 vote", "23 votes".
pub fn votes_text(count: u32) -> String {
    if count == 1 {
        "1 vote".to_string()
    } else {
        format!("{count} votes")
    }
}

/// Share of `total` as a whole percent, rounded to nearest. Zero when there
/// are no votes.
pub fn percent(count: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    let (count, total) = (u64::from(count), u64::from(total));
    ((count * 100 + total / 2) / total) as u32
}

/// True when the poll takes no more votes: it was closed, or its close time
/// is at or before `now` (an ISO-8601 UTC timestamp). The server refuses
/// votes in both cases. Timestamps are compared to the second.
pub fn is_closed(poll: &PollData, now: Option<&str>) -> bool {
    if !poll.is_active {
        return true;
    }
    match (poll.close_at.as_deref(), now) {
        (Some(close_at), Some(now)) => match (close_at.get(..19), now.get(..19)) {
            (Some(close_at), Some(now)) => close_at <= now,
            _ => false,
        },
        _ => false,
    }
}

/// What a poll card says when a vote, a vote removal or a poll removal
/// fails: `lead` names the action, the rest says why in words. The vote
/// route answers 409 for a closed poll or a vote that is already in.
pub fn poll_error_copy(lead: &str, err: &scuffed_api_client::ClientError) -> String {
    use scuffed_api_client::ClientError;
    let why = match err {
        ClientError::Http { status: 401, .. } => "Your session ended. Log in again.",
        ClientError::Http { status: 403, .. } => "Your account can't do that.",
        ClientError::Http { status: 404, .. } => "That poll no longer exists.",
        ClientError::Http { status: 409, .. } => {
            "The poll may have closed, or that vote is already in. Reload to see it."
        }
        ClientError::Http { status: 429, .. } => "Too many tries. Wait a moment.",
        // No answer, an unreadable answer and any other status: the one
        // mapping (QA on 748), never a guess at the connection.
        other => crate::client_error::message_without_reason(other),
    };
    format!("{lead} {why}")
}

/// The browser clock as an ISO-8601 UTC timestamp. `None` outside the
/// browser (native tests have no clock to trust).
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub fn now_iso() -> Option<String> {
    Some(String::from(js_sys::Date::new_0().to_iso_string()))
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
pub fn now_iso() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_errors_say_why_without_the_raw_client_error() {
        use scuffed_api_client::ClientError;
        let http = |status| ClientError::http(status, r#"{"error":"Poll is closed"}"#, None);
        assert_eq!(
            poll_error_copy("Vote failed.", &http(409)),
            "Vote failed. The poll may have closed, or that vote is already in. Reload to see it."
        );
        assert_eq!(
            poll_error_copy("Couldn't remove the poll.", &http(404)),
            "Couldn't remove the poll. That poll no longer exists."
        );
        assert_eq!(
            poll_error_copy("Vote failed.", &ClientError::Network("offline".into())),
            "Vote failed. Could not reach the server. Check your connection and try again."
        );
        for status in [400, 401, 403, 404, 409, 429, 500] {
            let copy = poll_error_copy("Vote failed.", &http(status));
            assert!(!copy.contains("Poll is closed"), "{copy}");
            assert!(!copy.contains(&status.to_string()), "{copy}");
            assert!(!copy.contains("connection"), "{copy}");
        }
        let card = include_str!("card.rs").split("mod tests").next().unwrap();
        assert!(
            !card.contains("failed: {e}"),
            "a raw client error reaches a toast"
        );
    }

    fn poll(is_active: bool, close_at: Option<&str>) -> PollData {
        PollData {
            id: "poll:1".to_string(),
            title: "T".to_string(),
            description: None,
            options: vec!["A".to_string(), "B".to_string()],
            close_at: close_at.map(str::to_string),
            allow_multiple: false,
            created_by: "member:1".to_string(),
            created_at: "2026-11-08T10:00:00Z".to_string(),
            is_active,
        }
    }

    #[test]
    fn percent_rounds_to_nearest_and_survives_zero_votes() {
        // The poll in the frame: 8, 12 and 3 of 23.
        assert_eq!(percent(8, 23), 35);
        assert_eq!(percent(12, 23), 52);
        assert_eq!(percent(3, 23), 13);
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(1, 1), 100);
        assert_eq!(percent(u32::MAX, u32::MAX), 100);
    }

    #[test]
    fn votes_are_counted_in_words() {
        assert_eq!(votes_text(0), "0 votes");
        assert_eq!(votes_text(1), "1 vote");
        assert_eq!(votes_text(23), "23 votes");
    }

    #[test]
    fn closed_by_flag_or_by_a_close_time_that_has_passed() {
        let now = Some("2026-11-16T20:00:00.000Z");
        assert!(is_closed(&poll(false, None), now));
        assert!(!is_closed(&poll(true, None), now));
        assert!(is_closed(&poll(true, Some("2026-11-16T20:00:00Z")), now));
        assert!(is_closed(&poll(true, Some("2026-11-16T19:59:59.5Z")), now));
        assert!(!is_closed(&poll(true, Some("2026-11-16T20:00:01Z")), now));
        // No clock: only the flag counts.
        assert!(!is_closed(&poll(true, Some("2020-01-01T00:00:00Z")), None));
        assert!(!is_closed(&poll(true, Some("soon")), now));
    }
}
