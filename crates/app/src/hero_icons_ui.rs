//! Copy, ids and small rules for the hero icon screens.
//!
//! The server side is `/api/hero-icons` (see `docs/api/hero-icons.md`). Icons
//! are private training data. A member sees and deletes their own. Officers
//! see only a count per member. Admins see the count and can delete.
//!
//! Every element id here is built from a record id plus a field name, so ids
//! stay stable across renders and never come from a global counter.

use chrono::{DateTime, Utc};
use dioxus::prelude::spawn;
use scuffed_api_client::ClientError;
use scuffed_types::api::HERO_ICON_ERR_DISABLED;

/// Give focus back to a button that is being mounted again after the confirm
/// strip closes. The shared strip asks for focus on the next tick, which can
/// run before the trigger is back in the page, so this waits a few frames.
pub fn focus_when_rendered(id: String) {
    spawn(async move {
        #[cfg(feature = "web")]
        {
            gloo_timers::future::TimeoutFuture::new(60).await;
        }
        crate::components::ui::inline_confirm::focus_element_by_id(&id);
    });
}

/// What a member wants done with icons already on the server when they turn
/// sharing off. Keep is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptOutChoice {
    #[default]
    Keep,
    Delete,
}

impl OptOutChoice {
    pub fn as_str(self) -> &'static str {
        match self {
            OptOutChoice::Keep => "keep",
            OptOutChoice::Delete => "delete",
        }
    }

    /// Radio value back to a choice. Unknown text is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "keep" => Some(OptOutChoice::Keep),
            "delete" => Some(OptOutChoice::Delete),
            _ => None,
        }
    }
}

/// Turning sharing off only needs a question when icons are stored.
pub fn opt_out_needs_question(icon_count: u32) -> bool {
    icon_count > 0
}

/// "1 icon" or "12 icons".
pub fn icons_phrase(n: u32) -> String {
    if n == 1 {
        "1 icon".to_string()
    } else {
        format!("{n} icons")
    }
}

pub const SECTION_INTRO: &str = "Hero icons you send are private training data for the tracker's icon matching. Officers see only how many you have sent. Admins can open your icons while sharing is on. When sharing is off, nobody but you can open them. Admins can still delete them.";

pub const STORE_DISABLED: &str = "Hero icon sharing is not switched on for this server.";

pub const LOADING: &str = "Loading hero icon settings...";

pub const SWITCH_LABEL: &str = "Share hero icons for tracker training";

pub const SWITCH_HINT_ON: &str =
    "Sharing is on. Turn it off at any time. You choose what happens to icons already sent.";

pub const SWITCH_HINT_OFF: &str = "Sharing is off. No icons are accepted from you.";

pub const KEPT_HIDDEN_NOTE: &str = "Your icons are kept on the server and hidden while sharing is off. Admins can delete them but cannot open them.";

pub const NONE_SENT: &str = "You have not sent any icons yet.";

pub fn opt_out_question(icon_count: u32) -> String {
    format!(
        "You have {} on the server. What should happen to them?",
        icons_phrase(icon_count)
    )
}

pub const KEEP_LABEL: &str = "Keep my icons. Nobody can open them. Admins can still delete them.";
pub const DELETE_LABEL: &str = "Delete my icons from the server.";

pub fn delete_all_title(icon_count: u32) -> String {
    format!("Delete all {}?", icons_phrase(icon_count))
}

pub const DELETE_ALL_BODY: &str =
    "This removes them from the server for good. Your sharing choice stays as it is.";

/// Text for the live region after a successful opt-in change.
pub fn status_opted_in() -> String {
    "Sharing is on.".to_string()
}

pub fn status_opted_out(choice: OptOutChoice, kept_or_deleted: u32) -> String {
    match choice {
        OptOutChoice::Keep if kept_or_deleted == 0 => "Sharing is off.".to_string(),
        OptOutChoice::Keep => format!(
            "Sharing is off. {} kept and hidden.",
            capitalize_first(&icons_phrase(kept_or_deleted))
        ),
        OptOutChoice::Delete => {
            format!("Sharing is off. Deleted {}.", icons_phrase(kept_or_deleted))
        }
    }
}

pub fn status_deleted_all(deleted: u32) -> String {
    format!("Deleted {}.", icons_phrase(deleted))
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// True when the server answered that the icon store is switched off.
pub fn is_store_disabled(err: &ClientError) -> bool {
    err.http_status() == Some(503)
        && err
            .http_body()
            .is_some_and(|b| b.contains(HERO_ICON_ERR_DISABLED))
}

/// Plain sentence for a failed call. Never shows the raw server body.
pub fn failure_text(what: &str, err: &ClientError) -> String {
    match err.http_status() {
        Some(401) => format!("{what} failed. Sign in again and retry."),
        Some(403) => format!("{what} failed. You are not allowed to do that."),
        Some(503) => format!("{what} failed. Hero icons are not switched on for this server."),
        Some(_) => format!("{what} failed. Try again in a moment."),
        None => format!("{what} failed. Check your connection and try again."),
    }
}

/// Alt text for one thumbnail: the hero and the day it was sent.
pub fn icon_alt(hero: &str, uploaded_at: &DateTime<Utc>) -> String {
    format!("{hero} icon, sent {}", uploaded_at.format("%Y-%m-%d"))
}

// Member screen ids.

pub fn section_id(member_id: &str) -> String {
    format!("hero-icons-{member_id}")
}

pub fn switch_id(member_id: &str) -> String {
    format!("hero-icons-{member_id}-opt-in")
}

pub fn switch_hint_id(member_id: &str) -> String {
    format!("hero-icons-{member_id}-opt-in-hint")
}

pub fn status_id(member_id: &str) -> String {
    format!("hero-icons-{member_id}-status")
}

pub fn question_id(member_id: &str) -> String {
    format!("hero-icons-{member_id}-opt-out")
}

pub fn choice_id(member_id: &str, choice: OptOutChoice) -> String {
    format!("hero-icons-{member_id}-opt-out-{}", choice.as_str())
}

pub fn thumb_id(icon_id: &str) -> String {
    format!("hero-icon-{icon_id}-image")
}

/// Id prefix for the delete all strip. Its trigger button is `{prefix}-trigger`.
pub fn delete_all_prefix(member_id: &str) -> String {
    format!("hero-icons-{member_id}-delete-all")
}

// Admin screen ids.

pub fn admin_status_id() -> &'static str {
    "hero-icons-admin-status"
}

pub fn admin_count_id(member_id: &str) -> String {
    format!("hero-icons-admin-{member_id}-count")
}

/// Id prefix for one member's delete strip. The trigger is `{prefix}-trigger`.
pub fn admin_delete_prefix(member_id: &str) -> String {
    format!("hero-icons-admin-{member_id}-delete")
}

/// Name for a row. The server sends an empty string when a record has no name.
pub fn member_label(display_name: &str) -> String {
    let name = display_name.trim();
    if name.is_empty() {
        "Unnamed member".to_string()
    } else {
        name.to_string()
    }
}

pub fn admin_delete_aria(display_name: &str) -> String {
    format!("Delete all icons for {}", member_label(display_name))
}

/// Id of the visible "Sharing off" label in a row.
pub fn admin_sharing_id(member_id: &str) -> String {
    format!("hero-icons-admin-{member_id}-sharing")
}

pub const SHARING_OFF_LABEL: &str = "Sharing off";

/// True when the server marks the member as not sharing. Admins get these
/// rows (hidden icons only). Officers never do, so no filtering happens here.
pub fn sharing_is_off(row: &scuffed_types::api::HeroIconMemberCount) -> bool {
    !(row.opted_in || row.icon_contribution_opt_in)
}

pub fn admin_delete_title(display_name: &str, icon_count: u32) -> String {
    format!(
        "Delete {} of {}?",
        icons_phrase(icon_count),
        member_label(display_name)
    )
}

pub const ADMIN_DELETE_BODY: &str = "This removes them from the server for good.";

pub const ADMIN_DELETE_BODY_HIDDEN: &str = "Sharing is off for this member, so the icons are hidden. This removes them from the server for good.";

pub const ADMIN_INTRO: &str = "This page shows how many hero icons each member has sent for tracker training, not the icons themselves. Only admins can delete them. Admins also see members marked Sharing off who still have icons stored. Admins can delete those hidden icons but cannot open them.";

pub const ADMIN_EMPTY: &str = "No member has turned on hero icon sharing yet.";

pub const ADMIN_LOADING: &str = "Loading hero icon counts...";

pub fn admin_loaded(members: usize, sharing_off: usize) -> String {
    let base = match members {
        1 => "1 member listed.".to_string(),
        n => format!("{n} members listed."),
    };
    if sharing_off == 0 {
        base
    } else {
        format!("{base} {sharing_off} with sharing off.")
    }
}

pub fn admin_deleted(display_name: &str, deleted: u32) -> String {
    format!(
        "Deleted {} of {}.",
        icons_phrase(deleted),
        member_label(display_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn keep_is_the_default_choice() {
        assert_eq!(OptOutChoice::default(), OptOutChoice::Keep);
    }

    #[test]
    fn choice_round_trips_and_rejects_unknown_text() {
        for c in [OptOutChoice::Keep, OptOutChoice::Delete] {
            assert_eq!(OptOutChoice::parse(c.as_str()), Some(c));
        }
        assert_eq!(OptOutChoice::parse(""), None);
        assert_eq!(OptOutChoice::parse("Keep"), None);
    }

    #[test]
    fn only_a_member_with_icons_gets_the_question() {
        assert!(!opt_out_needs_question(0));
        assert!(opt_out_needs_question(1));
        assert!(opt_out_needs_question(500));
    }

    #[test]
    fn counts_read_in_the_singular_and_plural() {
        assert_eq!(icons_phrase(0), "0 icons");
        assert_eq!(icons_phrase(1), "1 icon");
        assert_eq!(icons_phrase(12), "12 icons");
        assert_eq!(
            opt_out_question(1),
            "You have 1 icon on the server. What should happen to them?"
        );
    }

    #[test]
    fn opt_out_status_says_what_happened_to_the_icons() {
        assert_eq!(status_opted_out(OptOutChoice::Keep, 0), "Sharing is off.");
        assert_eq!(
            status_opted_out(OptOutChoice::Keep, 3),
            "Sharing is off. 3 icons kept and hidden."
        );
        assert_eq!(
            status_opted_out(OptOutChoice::Delete, 1),
            "Sharing is off. Deleted 1 icon."
        );
        assert_eq!(status_deleted_all(0), "Deleted 0 icons.");
    }

    #[test]
    fn ids_come_from_record_ids_and_field_names() {
        assert_eq!(switch_id("m1"), "hero-icons-m1-opt-in");
        assert_eq!(switch_id("m1"), switch_id("m1"));
        assert_ne!(switch_id("m1"), switch_id("m2"));
        assert_eq!(
            choice_id("m1", OptOutChoice::Delete),
            "hero-icons-m1-opt-out-delete"
        );
        assert_eq!(thumb_id("abc"), "hero-icon-abc-image");
        assert_eq!(
            format!("{}-trigger", delete_all_prefix("m1")),
            "hero-icons-m1-delete-all-trigger"
        );
        assert_eq!(admin_count_id("k2"), "hero-icons-admin-k2-count");
        assert_eq!(
            format!("{}-trigger", admin_delete_prefix("k2")),
            "hero-icons-admin-k2-delete-trigger"
        );
    }

    #[test]
    fn alt_text_names_the_hero_and_the_day() {
        let at = Utc.with_ymd_and_hms(2026, 11, 2, 18, 4, 5).unwrap();
        assert_eq!(icon_alt("Ana", &at), "Ana icon, sent 2026-11-02");
    }

    #[test]
    fn a_member_without_a_name_still_gets_a_readable_label() {
        assert_eq!(member_label("  "), "Unnamed member");
        assert_eq!(member_label(" FrozenTear "), "FrozenTear");
        assert_eq!(admin_delete_aria(""), "Delete all icons for Unnamed member");
        assert_eq!(
            admin_deleted("FrozenTear", 2),
            "Deleted 2 icons of FrozenTear."
        );
    }

    #[test]
    fn store_disabled_needs_the_503_and_the_code() {
        let off = ClientError::http(503, r#"{"error":"hero_icons_disabled"}"#, None);
        assert!(is_store_disabled(&off));
        let other = ClientError::http(503, r#"{"error":"busy"}"#, None);
        assert!(!is_store_disabled(&other));
        let wrong_status = ClientError::http(500, "hero_icons_disabled", None);
        assert!(!is_store_disabled(&wrong_status));
    }

    #[test]
    fn failures_never_echo_the_server_body() {
        let err = ClientError::http(500, "secret internal detail", None);
        let text = failure_text("Delete", &err);
        assert!(!text.contains("secret"), "{text}");
        assert!(text.starts_with("Delete failed."), "{text}");
    }

    #[test]
    fn copy_has_no_long_dashes() {
        let all = [
            SECTION_INTRO,
            STORE_DISABLED,
            LOADING,
            SWITCH_LABEL,
            SWITCH_HINT_ON,
            SWITCH_HINT_OFF,
            KEPT_HIDDEN_NOTE,
            NONE_SENT,
            KEEP_LABEL,
            DELETE_LABEL,
            DELETE_ALL_BODY,
            ADMIN_DELETE_BODY,
            ADMIN_DELETE_BODY_HIDDEN,
            SHARING_OFF_LABEL,
            ADMIN_INTRO,
            ADMIN_EMPTY,
            ADMIN_LOADING,
        ];
        for text in all {
            assert!(
                !text.chars().any(|c| matches!(c, '\u{2014}' | '\u{2013}')),
                "{text}"
            );
        }
        for text in [
            opt_out_question(3),
            delete_all_title(2),
            status_opted_out(OptOutChoice::Keep, 2),
            admin_delete_title("FrozenTear", 2),
            admin_loaded(2, 1),
        ] {
            assert!(
                !text.chars().any(|c| matches!(c, '\u{2014}' | '\u{2013}')),
                "{text}"
            );
        }
    }

    #[test]
    fn member_counts_parse_as_a_cursor_page() {
        let page: crate::hooks::CursorPage<scuffed_types::api::HeroIconMemberCount> =
            serde_json::from_str(
                r#"{"data":[{"member_id":"m1","display_name":"Frost","icon_count":3,"icon_contribution_opt_in":true}],"next_cursor":"100"}"#,
            )
            .unwrap();
        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].icon_count, 3);
        assert_eq!(page.next_cursor.as_deref(), Some("100"));
        let last: crate::hooks::CursorPage<scuffed_types::api::HeroIconMemberCount> =
            serde_json::from_str(r#"{"data":[]}"#).unwrap();
        assert!(last.next_cursor.is_none());
    }

    #[test]
    fn member_intro_names_who_can_see_icons() {
        assert!(SECTION_INTRO.contains("Officers see only how many"));
        assert!(SECTION_INTRO.contains("Admins can open your icons while sharing is on"));
        assert!(SECTION_INTRO.contains("Admins can still delete them"));
        assert!(KEEP_LABEL.contains("Admins can still delete them"));
        assert!(KEPT_HIDDEN_NOTE.contains("delete them but cannot open them"));
    }

    #[test]
    fn opted_out_rows_parse_and_are_marked() {
        let page: crate::hooks::CursorPage<scuffed_types::api::HeroIconMemberCount> =
            serde_json::from_str(
                r#"{"data":[
                {"member_id":"m1","display_name":"Frost","icon_count":3,"opted_in":true,"icon_contribution_opt_in":true},
                {"member_id":"m2","display_name":"Ember","icon_count":2,"opted_in":false,"icon_contribution_opt_in":false}
                ],"next_cursor":null}"#,
            )
            .unwrap();
        assert!(!sharing_is_off(&page.data[0]));
        assert!(sharing_is_off(&page.data[1]));
        assert!(!page.data[1].opted_in);
    }

    #[test]
    fn opted_in_rows_from_an_older_server_are_not_marked_off() {
        let row: scuffed_types::api::HeroIconMemberCount = serde_json::from_str(
            r#"{"member_id":"m1","display_name":"Frost","icon_count":3,"icon_contribution_opt_in":true}"#,
        )
        .unwrap();
        assert!(!row.opted_in);
        assert!(!sharing_is_off(&row));
    }

    #[test]
    fn admin_labels_and_ids_come_from_the_member() {
        assert_eq!(admin_delete_aria("Ember"), "Delete all icons for Ember");
        assert_eq!(admin_sharing_id("k2"), "hero-icons-admin-k2-sharing");
        assert_eq!(admin_loaded(3, 1), "3 members listed. 1 with sharing off.");
        assert_eq!(admin_loaded(1, 0), "1 member listed.");
    }
}
