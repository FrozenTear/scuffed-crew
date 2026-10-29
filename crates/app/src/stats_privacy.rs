//! Copy and request shape for who can see a member's tracker stats.
//!
//! The server side is `GET` and `PUT /api/stats/settings` and the 403 and 400
//! answers of `/api/stats/member/:id`. A `PUT` that leaves out `player_name`
//! clears the stored name, so every save here sends the current one.

use scuffed_api_client::ClientError;
use scuffed_types::api::{StatsVisibility, UpdateMemberSettingsRequest};

use crate::components::is_http_status;

/// Picker order, widest audience first.
pub const OPTIONS: [StatsVisibility; 3] = [
    StatsVisibility::Public,
    StatsVisibility::Members,
    StatsVisibility::Private,
];

pub fn label(v: StatsVisibility) -> &'static str {
    match v {
        StatsVisibility::Public => "Public",
        StatsVisibility::Members => "Members",
        StatsVisibility::Private => "Private",
    }
}

pub fn hint(v: StatsVisibility) -> &'static str {
    match v {
        StatsVisibility::Public => "Anyone can see your stats, signed in or not.",
        StatsVisibility::Members => {
            "Only signed-in crew members can see your stats. You stay off public leaderboards."
        }
        StatsVisibility::Private => {
            "Only you and officers can see your stats. You are off every leaderboard."
        }
    }
}

/// One line for the top of My Stats.
pub fn notice(v: StatsVisibility) -> &'static str {
    match v {
        StatsVisibility::Public => "Your stats are public.",
        StatsVisibility::Members => "Only signed-in crew members can see these stats.",
        StatsVisibility::Private => "Only you and officers can see these stats.",
    }
}

/// Select value back to a visibility. Unknown text is `None`.
pub fn parse(value: &str) -> Option<StatsVisibility> {
    OPTIONS.into_iter().find(|v| v.as_str() == value)
}

/// Body for `PUT /api/stats/settings`: the stored name plus the new choice.
pub fn settings_body(
    player_name: Option<String>,
    visibility: StatsVisibility,
) -> UpdateMemberSettingsRequest {
    UpdateMemberSettingsRequest {
        player_name,
        stats_visibility: Some(visibility),
    }
}

/// Toast copy for a failed settings save.
pub fn save_error(err: &ClientError) -> String {
    match err.http_status() {
        Some(400) => "That option isn't valid. Reload the page and try again.".to_string(),
        _ => format!("Save failed: {err}"),
    }
}

/// Friendly copy for a refused member stats fetch, from a `use_api*` error
/// string. `None` means it is not a stats privacy answer.
pub fn member_stats_denied(error: &str) -> Option<&'static str> {
    if is_http_status(error, 403) {
        if error.contains("stats_private") {
            return Some("This member keeps their stats private.");
        }
        if error.contains("stats_members_only") {
            return Some("These stats are only visible to signed-in crew members.");
        }
    }
    if is_http_status(error, 400) {
        return Some("These stats can't be shown. The member link may be wrong.");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_sends_name_and_visibility() {
        let body = settings_body(Some("FROZEN".into()), StatsVisibility::Private);
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"player_name":"FROZEN","stats_visibility":"private"}"#
        );
        let no_name = settings_body(None, StatsVisibility::Members);
        assert_eq!(
            serde_json::to_string(&no_name).unwrap(),
            r#"{"player_name":null,"stats_visibility":"members"}"#
        );
    }

    #[test]
    fn parse_round_trips_every_option() {
        for v in OPTIONS {
            assert_eq!(parse(v.as_str()), Some(v));
        }
        assert_eq!(parse("everyone"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn copy_has_no_dashes() {
        for v in OPTIONS {
            for text in [label(v), hint(v), notice(v)] {
                assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
            }
        }
    }

    #[test]
    fn member_stats_denial_maps_server_errors() {
        assert_eq!(
            member_stats_denied("HTTP error 403: stats_private"),
            Some("This member keeps their stats private.")
        );
        assert_eq!(
            member_stats_denied("HTTP error 403: stats_members_only"),
            Some("These stats are only visible to signed-in crew members.")
        );
        assert!(member_stats_denied("HTTP error 400: bad id").is_some());
        // The generic 403 stays with the membership gate.
        assert_eq!(
            member_stats_denied("HTTP error 403: Membership inactive"),
            None
        );
        assert_eq!(member_stats_denied("HTTP error 500: Internal error"), None);
        assert_eq!(member_stats_denied("Network error: offline"), None);
    }

    #[test]
    fn save_error_names_a_bad_option() {
        let bad = ClientError::http(400, r#"{"error":"invalid_stats_visibility"}"#, None);
        assert!(save_error(&bad).contains("isn't valid"));
        let down = ClientError::http(500, r#"{"error":"Internal error"}"#, None);
        assert_eq!(
            save_error(&down),
            "Save failed: HTTP error 500: Internal error"
        );
    }
}
