//! Copy, ids and the consent wire shape for the Training crops section.
//!
//! The server side is `/api/training-crops/consent` (see
//! `docs/api/training-crops.md`). The site is the only place a member changes
//! consent. The tracker shows it read-only and links here.
//!
//! Only four kinds exist on this screen: `hero_icon`, `stat_digits`,
//! `map_name` and `rank`. `hero_icon` consent is the existing hero icon
//! opt-in. The site never shows or sends any other kind. The consent shape is
//! a local struct with exactly those four fields, so the switch helpers can work on
//! [`ConsentKind`] alone. Tests pin it to the shared
//! `scuffed_types::api::TrainingCropConsent`, so a change on the server side
//! fails here.
//!
//! A save sends only the kind that changed ([`consent_update`]). The server
//! keeps the kinds left out, so a stale second tab can never switch a kind
//! back on that was turned off elsewhere. The page then takes all four
//! values from the server's answer.
//!
//! Every element id is built from the member id plus the kind's wire name, so
//! ids stay stable across renders and never come from a global counter.

use scuffed_api_client::ClientError;
use scuffed_types::api::TrainingCropConsentUpdate;
use serde::{Deserialize, Serialize};

/// Read and write own consent. The site uses the session cookie.
pub const CONSENT_PATH: &str = "/api/training-crops/consent";

/// A crop kind a member can switch on or off here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentKind {
    HeroIcon,
    StatDigits,
    MapName,
    Rank,
}

impl ConsentKind {
    /// Every kind, in screen order.
    pub const ALL: [ConsentKind; 4] = [
        ConsentKind::HeroIcon,
        ConsentKind::StatDigits,
        ConsentKind::MapName,
        ConsentKind::Rank,
    ];

    /// The wire name, also used in element ids.
    pub fn key(self) -> &'static str {
        match self {
            ConsentKind::HeroIcon => "hero_icon",
            ConsentKind::StatDigits => "stat_digits",
            ConsentKind::MapName => "map_name",
            ConsentKind::Rank => "rank",
        }
    }

    /// The switch's visible name.
    pub fn label(self) -> &'static str {
        match self {
            ConsentKind::HeroIcon => "Share hero icons",
            ConsentKind::StatDigits => "Share stat digit crops",
            ConsentKind::MapName => "Share map name banners",
            ConsentKind::Rank => "Share rank screen crops",
        }
    }

    /// Short name used in status lines.
    pub fn noun(self) -> &'static str {
        match self {
            ConsentKind::HeroIcon => "Hero icons",
            ConsentKind::StatDigits => "Stat digit crops",
            ConsentKind::MapName => "Map name banners",
            ConsentKind::Rank => "Rank screen crops",
        }
    }

    /// What the tracker sends for this kind. Shown under the switch.
    pub fn hint(self) -> &'static str {
        match self {
            ConsentKind::HeroIcon => {
                "The hero icon the tracker matched for each player. Off by default."
            }
            ConsentKind::StatDigits => {
                "The numbers the tracker read from the stats panel. Off by default."
            }
            ConsentKind::MapName => {
                "The map name banner the tracker read at the start of a game. Off by default."
            }
            ConsentKind::Rank => {
                "The rank panel crop (role, tier, SR and percentile), to help train rank recognition. Off by default."
            }
        }
    }
}

/// `GET` and `PUT` answer for consent: all four kinds as stored. `rank` is
/// absent in answers from servers before that kind and reads as false, the
/// same as the shared type. Unknown keys in an answer are ignored. This is never sent; a save sends
/// [`consent_update`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropConsent {
    pub hero_icon: bool,
    pub stat_digits: bool,
    pub map_name: bool,
    #[serde(default)]
    pub rank: bool,
}

impl CropConsent {
    pub fn get(&self, kind: ConsentKind) -> bool {
        match kind {
            ConsentKind::HeroIcon => self.hero_icon,
            ConsentKind::StatDigits => self.stat_digits,
            ConsentKind::MapName => self.map_name,
            ConsentKind::Rank => self.rank,
        }
    }

    /// A copy with one kind set.
    pub fn with(mut self, kind: ConsentKind, on: bool) -> Self {
        match kind {
            ConsentKind::HeroIcon => self.hero_icon = on,
            ConsentKind::StatDigits => self.stat_digits = on,
            ConsentKind::MapName => self.map_name = on,
            ConsentKind::Rank => self.rank = on,
        }
        self
    }
}

/// `PUT` body for one switch: only `kind`, so the server keeps the other kinds
/// as stored.
pub fn consent_update(kind: ConsentKind, on: bool) -> TrainingCropConsentUpdate {
    let mut update = TrainingCropConsentUpdate::default();
    match kind {
        ConsentKind::HeroIcon => update.hero_icon = Some(on),
        ConsentKind::StatDigits => update.stat_digits = Some(on),
        ConsentKind::MapName => update.map_name = Some(on),
        ConsentKind::Rank => update.rank = Some(on),
    }
    update
}

/// Plain sentence for a failed consent load or save. Same as the hero icon
/// copy (the one mapping, logged) except a 503, which names training crops
/// rather than hero icons. Never shows the raw server body.
pub fn consent_failure_text(what: &str, err: &ClientError) -> String {
    match err.http_status() {
        Some(503) => {
            crate::util::log_client_error(what, err);
            format!("{what} failed. {CONSENT_UNAVAILABLE}")
        }
        _ => crate::hero_icons_ui::failure_text(what, err),
    }
}

pub const CONSENT_UNAVAILABLE: &str = "Training crops are not switched on for this server.";

pub const HEADING: &str = "Training crops";

pub const INTRO: &str = "The stat tracker can send small PNG crops of what it reads, so its readers can be trained on real games. Crops are sent by the tracker with its sync token. They are never whole screens. Each kind is off by default, and this page is the only place to change it.";

pub const WHO_SEES: &str = "Crops are private. Officers see only how many you have sent. Admins can open crops of a kind while it is on. When a kind is off, nobody but you can open those crops. Admins can still delete them.";

pub const GROUP_LEGEND: &str = "What the tracker may send";

pub const LOADING: &str = "Loading your choices...";

pub fn saving_status(kind: ConsentKind) -> String {
    format!("Saving {}...", kind.noun().to_lowercase())
}

/// Live region text after a save. `detail` adds what happened to stored
/// hero icons when that kind was turned off.
pub fn saved_status(kind: ConsentKind, on: bool, detail: Option<&str>) -> String {
    let state = if on { "on" } else { "off" };
    match detail {
        Some(d) => format!("Saved. {} {state}. {d}", kind.noun()),
        None => format!("Saved. {} {state}.", kind.noun()),
    }
}

/// Live region text when a save failed and the switch went back.
pub fn failed_status(kind: ConsentKind, failure: &str) -> String {
    format!("{failure} {} are back to how they were.", kind.noun())
}

/// The failed part of a hero icon opt-out whose consent save worked but
/// whose delete did not. It goes to the alert region, not the saved note.
///
/// One next step per reason, and never two "try again"s (Vision nit 3 on
/// 748): no answer says to check the connection, then use Delete all my
/// icons; a 401 says to sign in again first; a 403 or 503 has no next step
/// (Delete all would fail the same way, and a 503 hides it); an unreadable
/// answer says to reload (the icons may be gone); a reason-less 5xx says to
/// wait. Any other answer keeps the server's reason and names the button.
/// The raw error goes to the console.
pub fn delete_failed_part(err: &ClientError) -> String {
    use crate::client_error::{SERVER_TROUBLE_MESSAGE, UNREACHABLE_SENTENCE, user_message};
    const LEAD: &str = "Deleting your icons failed.";
    const BUTTON: &str = "use Delete all my icons.";
    match err {
        ClientError::Network(_) => {
            crate::util::log_client_error(LEAD, err);
            format!("{LEAD} {UNREACHABLE_SENTENCE} Check your connection, then {BUTTON}")
        }
        ClientError::Http { status: 401, .. } => {
            crate::util::log_client_error(LEAD, err);
            format!("{LEAD} Your session has ended. Sign in again, then {BUTTON}")
        }
        ClientError::Http {
            status: 403 | 503, ..
        }
        | ClientError::Deserialize(_) => {
            crate::hero_icons_ui::failure_text("Deleting your icons", err)
        }
        ClientError::Http { .. } if user_message(err) == SERVER_TROUBLE_MESSAGE => {
            crate::util::log_client_error(LEAD, err);
            format!("{LEAD} The server had a problem. Wait a moment, then {BUTTON}")
        }
        ClientError::Http { .. } => crate::client_error::two_sentences(
            &crate::hero_icons_ui::failure_text("Deleting your icons", err),
            "Use Delete all my icons to try again.",
        ),
    }
}

/// What happened to stored hero icons after a consent save answered.
#[derive(Debug)]
pub enum IconsAfterSave {
    /// Not a hero icon save, sharing turned on, or nothing was stored.
    Untouched,
    /// Turned off and kept (hidden): how many.
    Kept(u32),
    /// Turned off and deleted: how many.
    Deleted(u32),
    /// Turned off with Delete chosen, and the delete failed.
    DeleteFailed(ClientError),
}

/// The consent line after a save answered (Vision nit 4 on 748): the page's
/// save callback only gathers the results and calls this, so every
/// combination is tested here. `save` is the saved state of `kind` (`Ok`)
/// or the save's error. What worked is the note; a failed delete is the
/// alert, never part of the note (issue 729).
pub fn line_after_save(
    kind: ConsentKind,
    save: Result<bool, &ClientError>,
    icons: IconsAfterSave,
) -> ConsentLine {
    use crate::hero_icons_ui::{OptOutChoice, opt_out_detail};
    let on = match save {
        Ok(on) => on,
        Err(err) => {
            let what = format!("Saving {}", kind.noun().to_lowercase());
            return ConsentLine::failure(failed_status(kind, &consent_failure_text(&what, err)));
        }
    };
    match icons {
        IconsAfterSave::Untouched => ConsentLine::saved(kind, on, None, None),
        IconsAfterSave::Kept(n) => ConsentLine::saved(
            kind,
            on,
            opt_out_detail(OptOutChoice::Keep, n).as_deref(),
            None,
        ),
        IconsAfterSave::Deleted(n) => ConsentLine::saved(
            kind,
            on,
            opt_out_detail(OptOutChoice::Delete, n).as_deref(),
            None,
        ),
        IconsAfterSave::DeleteFailed(err) => {
            ConsentLine::saved(kind, on, None, Some(delete_failed_part(&err)))
        }
    }
}

/// The consent live line shared by the four switches, as two parts so the
/// split never depends on the rendered text (issue 729):
/// - `note`: loading, saving or saved, a muted note in the `role="status"`
///   paragraph;
/// - `failure`: what failed and needs the user (a failed load, a failed
///   save, or the failed part of a save that partly worked), shown as an
///   error (icon and error colour) in the `role="alert"` paragraph.
///
/// A full failure has an empty note; a partial failure has both.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsentLine {
    pub note: String,
    pub failure: Option<String>,
}

impl ConsentLine {
    /// Loading, saving or saved. No failure.
    pub fn note(text: impl Into<String>) -> Self {
        Self {
            note: text.into(),
            failure: None,
        }
    }

    /// A load or save that failed outright. No note.
    pub fn failure(text: impl Into<String>) -> Self {
        Self {
            note: String::new(),
            failure: non_empty(text.into()),
        }
    }

    /// A save that partly worked: the note says what worked, the failure
    /// what did not.
    pub fn partial(note: impl Into<String>, failure: impl Into<String>) -> Self {
        Self {
            note: note.into(),
            failure: non_empty(failure.into()),
        }
    }

    /// The line after a save answered. `detail` is what else worked (icons
    /// kept or deleted); `failed_part` is what did not (the delete failed).
    pub fn saved(
        kind: ConsentKind,
        on: bool,
        detail: Option<&str>,
        failed_part: Option<String>,
    ) -> Self {
        let note = saved_status(kind, on, detail);
        match failed_part {
            Some(failed) => Self::partial(note, failed),
            None => Self::note(note),
        }
    }

    /// A Delete all my icons answered (Vision nits 2 and 5 on 748): the
    /// failed delete part of an earlier partial save is out of date, so it
    /// goes. A full load or save failure (no note) stays: it is not about
    /// the icons.
    pub fn drop_failed_part(&mut self) {
        if !self.note.is_empty() {
            self.failure = None;
        }
    }

    /// The failure text, empty when nothing failed.
    pub fn failure_text(&self) -> &str {
        self.failure.as_deref().unwrap_or("")
    }

    /// The `role="status"` paragraph's class: always the muted note.
    pub fn note_class(&self) -> &'static str {
        "account-status training-crops__status"
    }

    /// The `role="alert"` paragraph's class: the error modifier only while
    /// it carries a failure.
    pub fn alert_class(&self) -> &'static str {
        if self.failure.is_some() {
            "account-status training-crops__status is-error"
        } else {
            "account-status training-crops__status"
        }
    }
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

// Ids.

pub fn section_id(member_id: &str) -> String {
    format!("training-crops-{member_id}")
}

pub fn switch_id(member_id: &str, kind: ConsentKind) -> String {
    format!("training-crops-{member_id}-{}", kind.key())
}

pub fn label_id(member_id: &str, kind: ConsentKind) -> String {
    format!("{}-label", switch_id(member_id, kind))
}

pub fn hint_id(member_id: &str, kind: ConsentKind) -> String {
    format!("{}-hint", switch_id(member_id, kind))
}

pub fn consent_status_id(member_id: &str) -> String {
    format!("training-crops-{member_id}-consent-status")
}

/// The consent alert region's id, derived from the status id it sits next to.
pub fn consent_alert_id(status_id: &str) -> String {
    format!("{status_id}-alert")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    #[test]
    fn answer_shape_has_exactly_the_four_kinds() {
        let body = CropConsent {
            hero_icon: true,
            stat_digits: false,
            map_name: true,
            rank: true,
        };
        let value = serde_json::to_value(body).unwrap();
        assert_eq!(
            keys(&value),
            ["hero_icon", "map_name", "rank", "stat_digits"]
        );
        assert_eq!(
            value,
            serde_json::json!({"hero_icon": true, "stat_digits": false, "map_name": true, "rank": true})
        );
        let text = serde_json::to_string(&CropConsent::default()).unwrap();
        assert!(!text.contains("full_screen"), "{text}");
    }

    #[test]
    fn answer_with_four_kinds_parses() {
        let parsed: CropConsent = serde_json::from_str(
            r#"{"hero_icon":false,"stat_digits":true,"map_name":false,"rank":true}"#,
        )
        .unwrap();
        assert_eq!(
            parsed,
            CropConsent {
                hero_icon: false,
                stat_digits: true,
                map_name: false,
                rank: true,
            }
        );
    }

    #[test]
    fn rank_round_trips_and_defaults_false() {
        assert!(!CropConsent::default().rank);
        for on in [true, false] {
            let body = CropConsent::default().with(ConsentKind::Rank, on);
            let text = serde_json::to_string(&body).unwrap();
            let back: CropConsent = serde_json::from_str(&text).unwrap();
            assert_eq!(back, body);
            assert_eq!(back.rank, on);
            assert_eq!(back.get(ConsentKind::Rank), on);
        }
        // A server from before the rank kind leaves it out: it reads as off.
        let old: CropConsent =
            serde_json::from_str(r#"{"hero_icon":true,"stat_digits":true,"map_name":true}"#)
                .unwrap();
        assert!(!old.rank);
        assert!(!old.get(ConsentKind::Rank));
    }

    #[test]
    fn extra_keys_in_an_answer_are_ignored_and_never_sent_back() {
        let parsed: CropConsent = serde_json::from_str(
            r#"{"hero_icon":true,"stat_digits":false,"map_name":false,"full_screen":true}"#,
        )
        .unwrap();
        assert!(parsed.hero_icon);
        let value = serde_json::to_value(parsed).unwrap();
        assert_eq!(
            keys(&value),
            ["hero_icon", "map_name", "rank", "stat_digits"]
        );
    }

    #[test]
    fn an_answer_missing_a_kind_is_an_error() {
        let parsed = serde_json::from_str::<CropConsent>(r#"{"hero_icon":true,"map_name":true}"#);
        assert!(parsed.is_err());
        let parsed = serde_json::from_str::<CropConsent>(
            r#"{"hero_icon":"yes","stat_digits":false,"map_name":false}"#,
        );
        assert!(parsed.is_err());
        let parsed = serde_json::from_str::<CropConsent>(
            r#"{"hero_icon":true,"stat_digits":false,"map_name":false,"rank":"yes"}"#,
        );
        assert!(parsed.is_err());
    }

    #[test]
    fn shared_consent_answer_parses_into_the_local_shape() {
        let shared = scuffed_types::api::TrainingCropConsent {
            hero_icon: true,
            stat_digits: false,
            map_name: true,
            rank: true,
        };
        let parsed: CropConsent =
            serde_json::from_value(serde_json::to_value(shared).unwrap()).unwrap();
        assert_eq!(
            parsed,
            CropConsent {
                hero_icon: true,
                stat_digits: false,
                map_name: true,
                rank: true,
            }
        );
        // And back: the local shape parses into the shared one.
        let local = CropConsent::default().with(ConsentKind::Rank, true);
        let shared: scuffed_types::api::TrainingCropConsent =
            serde_json::from_value(serde_json::to_value(local).unwrap()).unwrap();
        assert!(shared.rank);
        assert!(!shared.hero_icon && !shared.stat_digits && !shared.map_name);
    }

    #[test]
    fn put_body_has_only_the_changed_kind() {
        for kind in ConsentKind::ALL {
            for on in [true, false] {
                let value = serde_json::to_value(consent_update(kind, on)).unwrap();
                assert_eq!(keys(&value), [kind.key()], "{kind:?} {on}");
                assert_eq!(value[kind.key()], serde_json::Value::Bool(on));
            }
        }
    }

    #[test]
    fn put_body_parses_into_the_shared_consent_update() {
        // The server refuses unknown fields, so this fails if the site ever
        // sends a key the shared type does not have.
        for kind in ConsentKind::ALL {
            let text = serde_json::to_string(&consent_update(kind, true)).unwrap();
            let update: TrainingCropConsentUpdate = serde_json::from_str(&text).unwrap();
            let set = [
                update.hero_icon,
                update.stat_digits,
                update.map_name,
                update.rank,
            ];
            assert_eq!(set.iter().filter(|v| v.is_some()).count(), 1, "{text}");
            assert_eq!(update, consent_update(kind, true));
        }
    }

    #[test]
    fn put_body_never_carries_removed_fields() {
        for kind in ConsentKind::ALL {
            for on in [true, false] {
                let text = serde_json::to_string(&consent_update(kind, on)).unwrap();
                for removed in ["full_screen", "colorblind_filter", "resolutions"] {
                    assert!(!text.contains(removed), "{text}");
                }
            }
        }
    }

    #[test]
    fn consent_503_names_training_crops_and_other_errors_match_hero_icons() {
        let off = ClientError::http(503, r#"{"error":"hero_icons_disabled"}"#, None);
        let text = consent_failure_text("Saving map name banners", &off);
        assert_eq!(
            text,
            "Saving map name banners failed. Training crops are not switched on for this server."
        );
        assert!(!text.contains("Hero icons"), "{text}");
        // The hero icon list keeps its own 503 copy.
        assert!(
            crate::hero_icons_ui::failure_text("Loading your hero icons", &off)
                .contains("Hero icons are not switched on")
        );
        for status in [401, 403, 500] {
            let err = ClientError::http(status, "secret internal detail", None);
            let text = consent_failure_text("Loading your choices", &err);
            assert_eq!(
                text,
                crate::hero_icons_ui::failure_text("Loading your choices", &err)
            );
            assert!(!text.contains("secret"), "{text}");
        }
    }

    #[test]
    fn everything_is_off_by_default() {
        let none = CropConsent::default();
        for kind in ConsentKind::ALL {
            assert!(!none.get(kind), "{kind:?}");
        }
    }

    #[test]
    fn with_sets_only_the_given_kind() {
        let base = CropConsent::default();
        for kind in ConsentKind::ALL {
            let next = base.with(kind, true);
            for other in ConsentKind::ALL {
                assert_eq!(next.get(other), other == kind, "{kind:?} {other:?}");
            }
            assert_eq!(next.with(kind, false), base);
        }
    }

    #[test]
    fn kinds_use_the_wire_names_and_no_full_screen() {
        let names: Vec<&str> = ConsentKind::ALL.iter().map(|k| k.key()).collect();
        assert_eq!(names, ["hero_icon", "stat_digits", "map_name", "rank"]);
        for kind in ConsentKind::ALL {
            let value = serde_json::to_value(CropConsent::default().with(kind, true)).unwrap();
            assert_eq!(value[kind.key()], serde_json::Value::Bool(true));
        }
    }

    #[test]
    fn ids_come_from_the_member_and_the_kind() {
        assert_eq!(
            switch_id("m1", ConsentKind::StatDigits),
            "training-crops-m1-stat_digits"
        );
        assert_eq!(
            label_id("m1", ConsentKind::MapName),
            "training-crops-m1-map_name-label"
        );
        assert_eq!(
            hint_id("m1", ConsentKind::HeroIcon),
            "training-crops-m1-hero_icon-hint"
        );
        assert_ne!(
            switch_id("m1", ConsentKind::HeroIcon),
            switch_id("m2", ConsentKind::HeroIcon)
        );
        assert_eq!(switch_id("m1", ConsentKind::Rank), "training-crops-m1-rank");
        assert_eq!(
            label_id("m1", ConsentKind::Rank),
            "training-crops-m1-rank-label"
        );
        assert_eq!(consent_status_id("m1"), "training-crops-m1-consent-status");
        assert_eq!(section_id("m1"), "training-crops-m1");
        // The alert id comes from the status id (data), not a counter: the
        // same input gives the same id every time, per member.
        assert_eq!(
            consent_alert_id(&consent_status_id("m1")),
            "training-crops-m1-consent-status-alert"
        );
        assert_eq!(
            consent_alert_id(&consent_status_id("m1")),
            consent_alert_id(&consent_status_id("m1"))
        );
        assert_ne!(
            consent_alert_id(&consent_status_id("m1")),
            consent_alert_id(&consent_status_id("m2"))
        );
        assert_eq!(consent_alert_id("s"), "s-alert");
    }

    #[test]
    fn consent_line_keeps_note_and_failure_apart() {
        let note = ConsentLine::note(LOADING);
        assert_eq!(note.note, LOADING);
        assert_eq!(note.failure, None);
        assert_eq!(note.failure_text(), "");
        assert_eq!(note.alert_class(), "account-status training-crops__status");
        assert_eq!(note.note_class(), "account-status training-crops__status");

        let full = ConsentLine::failure("Saving failed.");
        assert_eq!(full.note, "");
        assert_eq!(full.failure.as_deref(), Some("Saving failed."));
        assert_eq!(
            full.alert_class(),
            "account-status training-crops__status is-error"
        );
        assert_eq!(full.note_class(), "account-status training-crops__status");

        let partial = ConsentLine::saved(
            ConsentKind::HeroIcon,
            false,
            None,
            Some("Deleting your icons failed.".to_string()),
        );
        assert_eq!(partial.note, "Saved. Hero icons off.");
        assert_eq!(partial.failure_text(), "Deleting your icons failed.");
        assert_eq!(
            partial,
            ConsentLine::partial("Saved. Hero icons off.", "Deleting your icons failed.")
        );

        // A detail that worked stays in the note; nothing fails.
        let kept = ConsentLine::saved(
            ConsentKind::HeroIcon,
            false,
            Some("3 icons kept and hidden."),
            None,
        );
        assert_eq!(kept.note, "Saved. Hero icons off. 3 icons kept and hidden.");
        assert_eq!(kept.failure, None);

        // Empty failure text is no failure, so the icon never shows alone.
        assert_eq!(ConsentLine::failure("").failure, None);
        assert_eq!(ConsentLine::partial("Saved.", "").failure, None);
        assert_eq!(ConsentLine::default().failure, None);

        let failed = delete_failed_part(&ClientError::Network("offline".into()));
        assert!(
            failed.starts_with("Deleting your icons failed."),
            "{failed}"
        );
        // Vision nit 3 on 748: one next step, no doubled "try again".
        assert_eq!(
            failed,
            "Deleting your icons failed. Could not reach the server. Check your connection, then use Delete all my icons."
        );
        assert!(!failed.contains("offline"), "{failed}");
    }

    /// Vision nit 3 on 748: the failed delete part names one next step per
    /// reason, never repeats "try again" and never names a button that a
    /// retry cannot help with. Every case logs the raw error.
    #[test]
    fn delete_failed_part_has_one_next_step_per_reason() {
        use crate::client_error::UNREADABLE_ANSWER_MESSAGE;
        let _ = crate::util::take_logged();
        let cases: [(ClientError, String); 8] = [
            (
                ClientError::Network("JsValue(TypeError: Failed to fetch)".into()),
                "Deleting your icons failed. Could not reach the server. Check your connection, then use Delete all my icons.".into(),
            ),
            (
                ClientError::http(401, r#"{"error":"Unauthorized"}"#, None),
                "Deleting your icons failed. Your session has ended. Sign in again, then use Delete all my icons.".into(),
            ),
            (
                ClientError::http(403, r#"{"error":"Forbidden"}"#, None),
                "Deleting your icons failed. You are not allowed to do that.".into(),
            ),
            (
                ClientError::http(503, r#"{"error":"hero_icons_disabled"}"#, None),
                "Deleting your icons failed. Hero icons are not switched on for this server.".into(),
            ),
            (
                ClientError::Deserialize("expected value".into()),
                format!("Deleting your icons failed. {UNREADABLE_ANSWER_MESSAGE}"),
            ),
            (
                ClientError::http(502, "", None),
                "Deleting your icons failed. The server had a problem. Wait a moment, then use Delete all my icons.".into(),
            ),
            (
                ClientError::http(500, "<html>oops</html>", None),
                "Deleting your icons failed. The server had a problem. Wait a moment, then use Delete all my icons.".into(),
            ),
            (
                ClientError::http(409, r#"{"error":"a delete is already running"}"#, None),
                "Deleting your icons failed. HTTP error 409: a delete is already running. Use Delete all my icons to try again.".into(),
            ),
        ];
        for (err, want) in &cases {
            let text = delete_failed_part(err);
            assert_eq!(&text, want, "{err:?}");
            let lower = text.to_lowercase();
            assert!(lower.matches("try again").count() <= 1, "{text}");
            assert!(lower.matches("delete all my icons").count() <= 1, "{text}");
            assert!(!text.contains(".."), "{text}");
            assert!(!text.contains(". ."), "{text}");
        }
        assert_eq!(crate::util::take_logged().len(), cases.len());
    }

    /// Vision nit 4 on 748: the save callback's result-to-line mapping, per
    /// combination. The failed delete is only ever the alert, never the note
    /// (the mutant `detail = Some(delete_failed_part(&e))` fails here).
    #[test]
    fn line_after_save_maps_every_result() {
        let offline = || ClientError::Network("offline".into());
        // A save that worked, nothing else touched.
        for kind in ConsentKind::ALL {
            for on in [true, false] {
                assert_eq!(
                    line_after_save(kind, Ok(on), IconsAfterSave::Untouched),
                    ConsentLine::note(saved_status(kind, on, None)),
                    "{kind:?} {on}"
                );
            }
        }
        // Hero icons off, icons kept or deleted: the note says so, no failure.
        assert_eq!(
            line_after_save(ConsentKind::HeroIcon, Ok(false), IconsAfterSave::Kept(3)),
            ConsentLine::note("Saved. Hero icons off. 3 icons kept and hidden.")
        );
        assert_eq!(
            line_after_save(ConsentKind::HeroIcon, Ok(false), IconsAfterSave::Kept(0)),
            ConsentLine::note("Saved. Hero icons off.")
        );
        assert_eq!(
            line_after_save(ConsentKind::HeroIcon, Ok(false), IconsAfterSave::Deleted(2)),
            ConsentLine::note("Saved. Hero icons off. Deleted 2 icons.")
        );
        // Saved, delete failed: the note is only what worked, the failed
        // part is the alert.
        let partial = line_after_save(
            ConsentKind::HeroIcon,
            Ok(false),
            IconsAfterSave::DeleteFailed(offline()),
        );
        assert_eq!(partial.note, "Saved. Hero icons off.");
        assert_eq!(
            partial.failure.as_deref(),
            Some(delete_failed_part(&offline()).as_str())
        );
        assert!(!partial.note.contains("failed"), "{partial:?}");
        assert!(!partial.note.contains("Delete all"), "{partial:?}");
        // The save failed: the switch went back, the whole line is the alert.
        for kind in ConsentKind::ALL {
            let what = format!("Saving {}", kind.noun().to_lowercase());
            let failed = line_after_save(kind, Err(&offline()), IconsAfterSave::Untouched);
            assert_eq!(
                failed,
                ConsentLine::failure(failed_status(
                    kind,
                    &consent_failure_text(&what, &offline())
                ))
            );
            assert_eq!(failed.note, "", "{kind:?}");
            assert!(
                failed
                    .failure_text()
                    .ends_with("are back to how they were."),
                "{failed:?}"
            );
        }
        // A failed save ignores any icon result: nothing was saved.
        assert_eq!(
            line_after_save(
                ConsentKind::HeroIcon,
                Err(&offline()),
                IconsAfterSave::Deleted(2)
            ),
            line_after_save(
                ConsentKind::HeroIcon,
                Err(&offline()),
                IconsAfterSave::Untouched
            )
        );
    }

    #[test]
    fn status_lines_name_the_kind() {
        assert_eq!(
            saving_status(ConsentKind::StatDigits),
            "Saving stat digit crops..."
        );
        assert_eq!(
            saved_status(ConsentKind::MapName, true, None),
            "Saved. Map name banners on."
        );
        assert_eq!(
            saved_status(
                ConsentKind::HeroIcon,
                false,
                Some("3 icons kept and hidden.")
            ),
            "Saved. Hero icons off. 3 icons kept and hidden."
        );
        assert_eq!(
            failed_status(ConsentKind::HeroIcon, "Saving failed. Try again."),
            "Saving failed. Try again. Hero icons are back to how they were."
        );
        assert_eq!(
            saving_status(ConsentKind::Rank),
            "Saving rank screen crops..."
        );
        assert_eq!(
            saved_status(ConsentKind::Rank, false, None),
            "Saved. Rank screen crops off."
        );
    }

    #[test]
    fn copy_says_what_is_sent_and_how() {
        assert!(INTRO.contains("small PNG crops"));
        assert!(INTRO.contains("sync token"));
        assert!(INTRO.contains("never whole screens"));
        assert!(INTRO.contains("off by default"));
        for kind in ConsentKind::ALL {
            assert!(kind.hint().ends_with("Off by default."), "{kind:?}");
        }
        assert_eq!(ConsentKind::Rank.label(), "Share rank screen crops");
        let rank = ConsentKind::Rank.hint();
        for part in [
            "rank panel",
            "role",
            "tier",
            "SR",
            "percentile",
            "recognition",
        ] {
            assert!(rank.contains(part), "{rank}");
        }
    }

    #[test]
    fn copy_has_no_long_dashes_or_streamer() {
        let mut all: Vec<String> = [
            INTRO,
            WHO_SEES,
            GROUP_LEGEND,
            LOADING,
            HEADING,
            CONSENT_UNAVAILABLE,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        all.push(delete_failed_part(&ClientError::Network("x".into())));
        for kind in ConsentKind::ALL {
            all.extend([
                kind.label().to_string(),
                kind.hint().to_string(),
                saving_status(kind),
                saved_status(kind, false, Some("x")),
            ]);
        }
        for text in all {
            assert!(
                !text.chars().any(|c| matches!(c, '\u{2014}' | '\u{2013}')),
                "{text}"
            );
            assert!(!text.to_lowercase().contains("streamer"), "{text}");
            assert!(!text.contains("full_screen"), "{text}");
        }
    }
}
