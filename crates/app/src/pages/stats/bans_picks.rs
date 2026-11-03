//! Bans and picks on one personal game.
//!
//! The game body (`GET /api/stats/me/matches/{id}`) may carry an optional
//! `bans_picks` field, an object or null:
//!
//! ```json
//! { "bans": [{ "hero": "mauga", "team": "own" }], "lobby_ban": "ana",
//!   "picks": ["wrecking-ball", null], "recognizer": "cv-v1" }
//! ```
//!
//! Hero values are hero pack keys, or null when a slot could not be read or
//! the hero is unknown to the server. `team` is `own`, `enemy` or null. Bans
//! are up to 4. Picks are the uploader's own team only, own row first, up to 6
//! because the reader checks six rows (the extra is null in a 5 player game).
//! `ban_vote` is accepted but not shown. This is the
//! shape API proposed; the local types below mirror it until API's change
//! lands, then they should be swapped for the shared ones.
//!
//! The field is optional all the way down. Absent, null, empty, or every slot
//! unreadable renders nothing, and a value that does not parse is dropped
//! instead of failing the whole game. Extra keys are ignored, so the server can
//! add keys later without breaking this page. A string holding the object's
//! JSON is read too.
//!
//! Hero art is not shipped with the site yet, so heroes show as names.

use dioxus::prelude::*;
use scuffed_types::heroes::{canonical_hero, hero_key_to_name, is_placeholder_hero};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub(super) const SECTION_TITLE: &str = "Bans and picks";
const OWN_BANS_TITLE: &str = "Your team's bans";
const ENEMY_BANS_TITLE: &str = "Enemy team's bans";
const OTHER_BANS_TITLE: &str = "Bans";
const OTHER_BANS_SIDE_UNKNOWN_TITLE: &str = "Bans, side not known";
const LOBBY_BAN_TITLE: &str = "Lobby ban";
// Picks are the uploader's own team only.
const PICKS_TITLE: &str = "Team picks";
/// The key the server sends when a player showed no preferred hero.
const NO_PREFERENCE_KEY: &str = "no_preference";
/// Shown for a slot that holds the no preference key.
pub(super) const NO_PREFERENCE_TEXT: &str = "No preference";
/// A slot with no hero: not read, or a hero the server does not know.
pub(super) const UNKNOWN_TEXT: &str = "Unknown";

/// One ban. `team` is `own` or `enemy`; null, missing or anything else means
/// the side is not known.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub(super) struct BanRead {
    #[serde(default)]
    hero: Option<String>,
    #[serde(default)]
    team: Option<String>,
}

/// The `bans_picks` field.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub(super) struct BansPicks {
    #[serde(default)]
    bans: Option<Vec<BanRead>>,
    #[serde(default)]
    lobby_ban: Option<String>,
    /// The uploader's own team only, own row first. Up to 6 entries because the
    /// reader checks six rows, so a 5 player game ends in null.
    #[serde(default)]
    picks: Option<Vec<Option<String>>>,
    /// Accepted so the field parses. Not shown on the page yet.
    #[serde(default)]
    #[allow(dead_code)]
    ban_vote: Option<Vec<Option<String>>>,
}

/// Read the field leniently: null, an object, or a string holding the
/// object's JSON. Anything else, or an object of the wrong shape, is None.
pub(super) fn lenient<'de, D: Deserializer<'de>>(d: D) -> Result<Option<BansPicks>, D::Error> {
    let raw = Value::deserialize(d)?;
    let object = match raw {
        Value::String(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::Null),
        other => other,
    };
    if !object.is_object() {
        return Ok(None);
    }
    Ok(serde_json::from_value(object).ok())
}

/// What one slot shows.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Shown {
    Hero(String),
    /// The reader saw the "no preferred hero" icon. Not the same as unknown.
    NoPreference,
    Unknown,
}

impl Shown {
    fn from_key(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => Shown::Unknown,
            Some(NO_PREFERENCE_KEY) => Shown::NoPreference,
            Some(key) if is_placeholder_hero(key) => Shown::Unknown,
            Some(key) => Shown::Hero(
                hero_key_to_name(key)
                    .map(str::to_string)
                    .unwrap_or_else(|| canonical_hero(key)),
            ),
        }
    }

    /// The words on screen and for screen readers.
    pub(super) fn text(&self) -> &str {
        match self {
            Shown::Hero(name) => name,
            Shown::NoPreference => NO_PREFERENCE_TEXT,
            Shown::Unknown => UNKNOWN_TEXT,
        }
    }

    /// True for anything the reader saw, a hero or the no preference icon.
    fn is_read(&self) -> bool {
        !matches!(self, Shown::Unknown)
    }

    /// Placeholders are drawn quieter than heroes.
    fn is_hero(&self) -> bool {
        matches!(self, Shown::Hero(_))
    }
}

/// One titled list. `key` is the id suffix, so ids come from the match id and
/// the group, never from a counter.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Group {
    pub(super) key: &'static str,
    pub(super) title: &'static str,
    pub(super) items: Vec<Shown>,
}

fn group(key: &'static str, title: &'static str, items: Vec<Shown>) -> Option<Group> {
    // Slots keep their place (the picks list starts with the local player),
    // but a list where nothing was read has nothing to show.
    items
        .iter()
        .any(Shown::is_read)
        .then_some(Group { key, title, items })
}

impl BansPicks {
    /// The lists to show, in order. Empty when there is nothing to show.
    pub(super) fn groups(&self) -> Vec<Group> {
        let bans = self.bans.as_deref().unwrap_or(&[]);
        let side = |wanted: Option<&str>| -> Vec<Shown> {
            bans.iter()
                .filter(|ban| ban.team.as_deref() == wanted)
                .map(|ban| Shown::from_key(ban.hero.as_deref()))
                .collect()
        };
        let own = side(Some("own"));
        let enemy = side(Some("enemy"));
        let other: Vec<Shown> = bans
            .iter()
            .filter(|ban| !matches!(ban.team.as_deref(), Some("own" | "enemy")))
            .map(|ban| Shown::from_key(ban.hero.as_deref()))
            .collect();
        let sided = own.iter().chain(&enemy).any(Shown::is_read);
        let other_title = if sided {
            OTHER_BANS_SIDE_UNKNOWN_TITLE
        } else {
            OTHER_BANS_TITLE
        };
        let lobby = self
            .lobby_ban
            .as_deref()
            .map(|key| vec![Shown::from_key(Some(key))])
            .unwrap_or_default();
        let picks = self
            .picks
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|key| Shown::from_key(key.as_deref()))
            // The reader checks six rows, so a 5 player game ends in a null.
            // Only heroes that were read are listed.
            .filter(Shown::is_read)
            .collect();
        [
            group("bans-own", OWN_BANS_TITLE, own),
            group("bans-enemy", ENEMY_BANS_TITLE, enemy),
            group("bans-other", other_title, other),
            group("lobby-ban", LOBBY_BAN_TITLE, lobby),
            group("picks", PICKS_TITLE, picks),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// Anything that is not id-safe becomes an underscore. Same rule as the
/// unsure note ids on the History rows.
fn id_part(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Id of the section heading. Built from the match id only.
pub(super) fn heading_id(match_id: &str) -> String {
    format!("bans-picks-{}-heading", id_part(match_id))
}

/// Id of one list's heading. Built from the match id and the group.
pub(super) fn group_title_id(match_id: &str, group: &str) -> String {
    format!("bans-picks-{}-{}-title", id_part(match_id), id_part(group))
}

pub(super) const BANS_PICKS_CSS: &str = r#"
    .game-bans-picks {
        margin-top: 1rem;
        padding: 1rem 1.25rem;
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
    }
    .game-bans-picks .game-bp-heading {
        margin: 0 0 0.75rem;
        font-family: var(--font-head);
        font-size: 1rem;
        color: var(--text);
    }
    .game-bp-groups {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
        gap: 0.9rem 1.25rem;
    }
    .game-bp-group { min-width: 0; }
    .game-bp-group .game-bp-title {
        margin: 0 0 0.4rem;
        font-size: 0.75rem;
        font-weight: 600;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        color: var(--text-3);
    }
    .game-bp-list {
        display: flex;
        flex-wrap: wrap;
        gap: 0.4rem;
        margin: 0;
        padding: 0;
        list-style: none;
    }
    .game-bp-item {
        max-width: 100%;
        padding: 0.3rem 0.65rem;
        background: var(--surface-2);
        border: 1px solid var(--border);
        border-radius: 999px;
        font-size: 0.9rem;
        color: var(--text);
        overflow-wrap: anywhere;
    }
    .game-bp-item-muted { color: var(--text-3); }
"#;

/// The section for one game. Renders nothing when the field is absent, empty,
/// or has no readable slot, so an older game looks exactly as before.
pub(super) fn bans_picks_section(match_id: &str, field: Option<&BansPicks>) -> Element {
    let groups = field.map(BansPicks::groups).unwrap_or_default();
    if groups.is_empty() {
        return rsx! {};
    }
    let heading = heading_id(match_id);
    let section_id = format!("bans-picks-{}", id_part(match_id));
    rsx! {
        section { class: "game-bans-picks", id: "{section_id}", aria_labelledby: "{heading}",
            h2 { class: "game-bp-heading", id: "{heading}", "{SECTION_TITLE}" }
            div { class: "game-bp-groups",
                for g in groups {
                    {
                        let title_id = group_title_id(match_id, g.key);
                        rsx! {
                            div { class: "game-bp-group", key: "{g.key}",
                                h3 { class: "game-bp-title", id: "{title_id}", "{g.title}" }
                                ul { class: "game-bp-list", aria_labelledby: "{title_id}",
                                    for (i, item) in g.items.iter().enumerate() {
                                        li {
                                            class: if item.is_hero() { "game-bp-item" } else { "game-bp-item game-bp-item-muted" },
                                            key: "{i}",
                                            "{item.text()}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Option<BansPicks> {
        #[derive(Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "lenient")]
            bans_picks: Option<BansPicks>,
        }
        serde_json::from_str::<Row>(json).expect("row").bans_picks
    }

    const FULL: &str = r#"{"bans_picks": {
        "bans": [
            {"hero": "ana", "team": "own"},
            {"hero": "wrecking-ball", "team": "own"},
            {"hero": "soldier-76", "team": "enemy"},
            {"hero": null, "team": "enemy"}
        ],
        "lobby_ban": "mercy",
        "picks": ["kiriko", "dva", null],
        "ban_vote": ["ana", null, "mei"],
        "recognizer": "cv-v1"
    }}"#;

    fn render(match_id: &str, field: Option<BansPicks>) -> String {
        #[derive(Clone, PartialEq)]
        struct Props {
            id: String,
            field: Option<BansPicks>,
        }
        let mut dom = VirtualDom::new_with_props(
            |props: Props| bans_picks_section(&props.id, props.field.as_ref()),
            Props {
                id: match_id.to_string(),
                field,
            },
        );
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn titles(field: &BansPicks) -> Vec<&'static str> {
        field.groups().iter().map(|g| g.title).collect()
    }

    #[test]
    fn absent_null_and_empty_fields_render_nothing() {
        assert_eq!(parse(r#"{}"#), None);
        assert_eq!(parse(r#"{"bans_picks": null}"#), None);
        for json in [
            r#"{"bans_picks": {}}"#,
            r#"{"bans_picks": {"bans": [], "lobby_ban": null, "picks": []}}"#,
            r#"{"bans_picks": {"bans": null, "picks": null}}"#,
        ] {
            let field = parse(json).expect(json);
            assert!(field.groups().is_empty(), "{json}");
            assert_eq!(render("m1", Some(field)), "", "{json}");
        }
        assert_eq!(render("m1", None), "");
    }

    #[test]
    fn unreadable_only_slots_render_nothing() {
        let field = parse(
            r#"{"bans_picks": {"bans": [{"hero": null, "team": "own"}],
            "lobby_ban": null, "picks": [null, "skull_01"]}}"#,
        )
        .unwrap();
        assert!(field.groups().is_empty());
        assert_eq!(render("m1", Some(field)), "");
    }

    #[test]
    fn a_bad_value_is_dropped_and_never_fails_the_row() {
        for json in [
            r#"{"bans_picks": 5}"#,
            r#"{"bans_picks": true}"#,
            r#"{"bans_picks": []}"#,
            r#"{"bans_picks": "not json"}"#,
            r#"{"bans_picks": "5"}"#,
            r#"{"bans_picks": {"bans": "ana"}}"#,
            r#"{"bans_picks": {"picks": [1, 2]}}"#,
            r#"{"bans_picks": {"lobby_ban": {"hero": "ana"}}}"#,
        ] {
            assert_eq!(parse(json), None, "{json}");
        }
    }

    #[test]
    fn a_string_holding_the_object_reads_like_the_object() {
        let object = parse(FULL).expect("object");
        let inner =
            serde_json::to_string(&serde_json::from_str::<Value>(FULL).unwrap()["bans_picks"])
                .unwrap();
        let wrapped = serde_json::json!({ "bans_picks": inner }).to_string();
        assert_eq!(parse(&wrapped), Some(object));
    }

    #[test]
    fn full_field_groups_by_side_and_keeps_slot_order() {
        let field = parse(FULL).unwrap();
        assert_eq!(
            titles(&field),
            [
                OWN_BANS_TITLE,
                ENEMY_BANS_TITLE,
                LOBBY_BAN_TITLE,
                PICKS_TITLE
            ]
        );
        let groups = field.groups();
        let words = |g: &Group| {
            g.items
                .iter()
                .map(|s| s.text().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(words(&groups[0]), ["Ana", "Wrecking Ball"]);
        assert_eq!(words(&groups[1]), ["Soldier: 76", UNKNOWN_TEXT]);
        assert_eq!(words(&groups[2]), ["Mercy"]);
        assert_eq!(words(&groups[3]), ["Kiriko", "D.Va"]);
    }

    #[test]
    fn ban_vote_parses_but_is_not_shown() {
        let field = parse(FULL).unwrap();
        assert_eq!(field.ban_vote.as_ref().map(Vec::len), Some(3));
        let html = render("m1", Some(field));
        assert!(!html.contains("Mei") && !html.contains("vote"), "{html}");
        let only_vote = parse(r#"{"bans_picks": {"ban_vote": ["ana", "mei"]}}"#).unwrap();
        assert!(only_vote.groups().is_empty());
    }

    #[test]
    fn no_preference_shows_as_a_placeholder_and_null_picks_are_skipped() {
        let field = parse(
            r#"{"bans_picks": {"picks": ["ana", "no_preference", null, "mei", "no_preference"]}}"#,
        )
        .unwrap();
        let groups = field.groups();
        assert_eq!(groups.len(), 1);
        let words: Vec<&str> = groups[0].items.iter().map(Shown::text).collect();
        assert_eq!(
            words,
            ["Ana", NO_PREFERENCE_TEXT, "Mei", NO_PREFERENCE_TEXT]
        );
        assert_eq!(Shown::from_key(Some("no_preference")), Shown::NoPreference);
        // Not mistaken for an unknown hero key.
        assert!(
            !words
                .iter()
                .any(|w| w.contains("no preference") || *w == UNKNOWN_TEXT)
        );
        let html = render("m1", Some(field));
        assert_eq!(html.matches("No preference").count(), 2, "{html}");
        assert_eq!(html.matches("game-bp-item-muted").count(), 2, "{html}");
        // Only no preference slots still list, since the reader did see them.
        let lone = parse(r#"{"bans_picks": {"picks": ["no_preference", null]}}"#).unwrap();
        assert_eq!(lone.groups().len(), 1);
    }

    #[test]
    fn a_five_player_game_lists_only_the_picks_that_were_read() {
        let field =
            parse(r#"{"bans_picks": {"picks": ["ana", "mei", "dva", "mercy", "juno", null]}}"#)
                .unwrap();
        let groups = field.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, PICKS_TITLE);
        assert_eq!(groups[0].items.len(), 5);
        assert!(groups[0].items.iter().all(|s| s.text() != UNKNOWN_TEXT));
    }

    #[test]
    fn missing_side_gets_its_own_list() {
        let none =
            parse(r#"{"bans_picks": {"bans": [{"hero": "ana"}, {"hero": "mei", "team": null}]}}"#)
                .unwrap();
        assert_eq!(titles(&none), [OTHER_BANS_TITLE]);
        assert_eq!(none.groups()[0].items.len(), 2);

        let some = parse(
            r#"{"bans_picks": {"bans": [{"hero": "ana", "team": "own"},
            {"hero": "mei"}, {"hero": "juno", "team": "sideways"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            titles(&some),
            [OWN_BANS_TITLE, OTHER_BANS_SIDE_UNKNOWN_TITLE]
        );
        assert_eq!(some.groups()[1].items.len(), 2);
    }

    #[test]
    fn hero_keys_read_as_display_names_and_unknown_keys_stay_readable() {
        assert_eq!(Shown::from_key(Some("dva")).text(), "D.Va");
        assert_eq!(Shown::from_key(Some("lucio")).text(), "Lúcio");
        assert_eq!(Shown::from_key(Some("new-hero")).text(), "new hero");
        assert_eq!(Shown::from_key(Some("placeholder_01")), Shown::Unknown);
        assert_eq!(Shown::from_key(Some("  ")), Shown::Unknown);
        assert_eq!(Shown::from_key(None), Shown::Unknown);
    }

    #[test]
    fn markup_uses_headings_lists_and_names_from_data() {
        let html = render("rec:Abc 1", parse(FULL));
        assert!(html.contains("<section"), "{html}");
        assert!(
            html.contains("id=\"bans-picks-rec_abc_1-heading\""),
            "{html}"
        );
        assert!(
            html.contains("aria-labelledby=\"bans-picks-rec_abc_1-heading\""),
            "{html}"
        );
        assert_eq!(html.matches("<h2").count(), 1, "{html}");
        assert_eq!(html.matches("<h3").count(), 4, "{html}");
        assert_eq!(html.matches("<ul").count(), 4, "{html}");
        // Each list is named by its own heading.
        for group in ["bans-own", "bans-enemy", "lobby-ban", "picks"] {
            let id = group_title_id("rec:Abc 1", group);
            assert!(html.contains(&format!("id=\"{id}\"")), "{id}: {html}");
            assert!(
                html.contains(&format!("aria-labelledby=\"{id}\"")),
                "{id}: {html}"
            );
        }
        assert_eq!(html.matches("<li").count(), 7, "{html}");
        for text in [
            "Wrecking Ball",
            "Soldier: 76",
            "Lobby ban",
            "Team picks",
            UNKNOWN_TEXT,
        ] {
            assert!(html.contains(text), "{text}: {html}");
        }
        assert_eq!(html.matches("game-bp-item-muted").count(), 1, "{html}");
        // Nothing interactive, no dashes in the copy.
        assert!(!html.contains("<button") && !html.contains("<a "), "{html}");
        assert!(
            !html.contains('\u{2014}') && !html.contains('\u{2013}'),
            "{html}"
        );
    }

    #[test]
    fn ids_come_from_the_match_id_and_group_only() {
        assert_eq!(heading_id("abc"), "bans-picks-abc-heading");
        assert_eq!(
            group_title_id("abc", "bans-own"),
            "bans-picks-abc-bans-own-title"
        );
        assert_ne!(heading_id("a"), heading_id("b"));
        let one = render("a", parse(FULL));
        let again = render("a", parse(FULL));
        assert_eq!(one, again, "ids must not depend on render order");
        let other = render("b", parse(FULL));
        assert!(!other.contains("bans-picks-a-"), "{other}");
    }
}
