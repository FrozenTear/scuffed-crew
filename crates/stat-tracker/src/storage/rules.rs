//! Pure row rules, testable without a store: manual edits
//! ([`MatchEdit`]), hero segments, majorities, and [`latest_per_game`].

use serde::{Deserialize, Serialize};
use surrealdb_types::Datetime as SurrealDatetime;

use super::{HeroSegment, PersonalMatch, SEGMENT_CONFIRM, SEGMENT_DISMISS, SegmentResolution};

/// A stored outcome label that names a result (`victory`, `defeat`, `draw`).
pub(super) fn outcome_is_decided(label: &str) -> bool {
    label
        .parse::<crate::model::MatchOutcome>()
        .is_ok_and(crate::model::MatchOutcome::is_decided)
}

/// A manual correction to a game's stats. Only `Some(_)` fields are applied;
/// each corresponds to an OCR-captured field that stays untouched underneath.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchEdit {
    pub hero: Option<String>,
    pub role: Option<String>,
    pub map_name: Option<String>,
    pub outcome: Option<String>,
    pub elims: Option<u32>,
    pub deaths: Option<u32>,
    pub assists: Option<u32>,
    pub damage: Option<u32>,
    pub healing: Option<u32>,
    pub mitigation: Option<u32>,
}

impl MatchEdit {
    /// True when no field is set — an empty edit is a no-op.
    pub fn is_empty(&self) -> bool {
        *self == MatchEdit::default()
    }
}

fn mark_edited(fields: &mut Vec<String>, name: &str) {
    if !fields.iter().any(|f| f == name) {
        fields.push(name.to_string());
    }
}

/// The member's own hero pick on a session's rows: the newest row that
/// carries a `corrected_hero`. [`LocalStore::edit_match`] writes the same
/// pick onto every row the session had at that time.
///
/// [`LocalStore::edit_match`]: super::LocalStore::edit_match
pub fn member_hero_edit(rows: &[PersonalMatch]) -> Option<&PersonalMatch> {
    rows.iter().rev().find(|m| m.corrected_hero.is_some())
}

/// Copy a member's hero pick from `edited` (a row of the same session) onto
/// a capture row written after the edit, so the pick is never overridden by
/// that row's read: `corrected_hero`, `corrected_role` when the edit set one,
/// the `hero` / `role` edit marks, `edited_at`, and the cleared `hero`
/// suspect. The row's own read (`hero`, `role`) stays as read. Returns
/// whether `edited` carried a pick.
pub fn carry_member_hero_edit(edited: &PersonalMatch, row: &mut PersonalMatch) -> bool {
    let Some(hero) = edited.corrected_hero.clone() else {
        return false;
    };
    row.corrected_hero = Some(hero);
    mark_edited(&mut row.edited_fields, "hero");
    crate::suspect::clear_edited_suspects(&mut row.suspect_fields, "hero");
    if let Some(role) = edited.corrected_role.clone() {
        row.corrected_role = Some(role);
        mark_edited(&mut row.edited_fields, "role");
    }
    if row.edited_at.is_none() {
        row.edited_at = edited.edited_at;
    }
    true
}

/// Overlay a [`MatchEdit`] onto a row: for every `Some(_)` field, set the
/// matching `corrected_*` value (OCR fields untouched) and record the field
/// name. Re-editing accumulates the union of edited fields and overwrites
/// earlier corrections for the same field. Returns whether anything changed;
/// stamps `edited_at` only then.
pub(super) fn apply_match_edit(
    m: &mut PersonalMatch,
    edit: &MatchEdit,
    now: SurrealDatetime,
) -> bool {
    let mut changed = false;
    if let Some(v) = &edit.hero {
        m.corrected_hero = Some(v.clone());
        mark_edited(&mut m.edited_fields, "hero");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "hero");
        changed = true;
        let guessed = crate::parse::guess_role_public(v);
        if edit.role.is_none()
            && !crate::parse::hero_is_unknown_label(v)
            && guessed != m.display_role()
        {
            m.corrected_role = Some(guessed);
            mark_edited(&mut m.edited_fields, "role");
        }
    }
    if let Some(v) = &edit.role {
        m.corrected_role = Some(v.clone());
        mark_edited(&mut m.edited_fields, "role");
        changed = true;
    }
    if let Some(v) = &edit.map_name {
        m.corrected_map_name = Some(v.clone());
        mark_edited(&mut m.edited_fields, "map_name");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "map_name");
        changed = true;
        let derived = crate::parse::stored_game_mode(v);
        if !derived.is_empty() {
            m.game_mode = derived;
        }
    }
    if let Some(v) = &edit.outcome {
        m.corrected_outcome = Some(v.clone());
        mark_edited(&mut m.edited_fields, "outcome");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "outcome");
        changed = true;
    }
    if let Some(v) = edit.elims {
        m.corrected_elims = Some(v);
        mark_edited(&mut m.edited_fields, "elims");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "elims");
        changed = true;
    }
    if let Some(v) = edit.deaths {
        m.corrected_deaths = Some(v);
        mark_edited(&mut m.edited_fields, "deaths");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "deaths");
        changed = true;
    }
    if let Some(v) = edit.assists {
        m.corrected_assists = Some(v);
        mark_edited(&mut m.edited_fields, "assists");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "assists");
        changed = true;
    }
    if let Some(v) = edit.damage {
        m.corrected_damage = Some(v);
        mark_edited(&mut m.edited_fields, "damage");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "damage");
        changed = true;
    }
    if let Some(v) = edit.healing {
        m.corrected_healing = Some(v);
        mark_edited(&mut m.edited_fields, "healing");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "healing");
        changed = true;
    }
    if let Some(v) = edit.mitigation {
        m.corrected_mitigation = Some(v);
        mark_edited(&mut m.edited_fields, "mitigation");
        crate::suspect::clear_edited_suspects(&mut m.suspect_fields, "mitigation");
        changed = true;
    }
    if changed {
        m.edited_at = Some(now);
    }
    changed
}

/// Gather the union of `segment_resolutions` across a session's snapshots,
/// deduped by segment index (last row wins) and sorted. The daemon stamps
/// resolutions uniformly onto every row, so in practice all rows agree — this
/// merge just tolerates a row that missed a write.
pub fn collect_segment_resolutions(snapshots: &[PersonalMatch]) -> Vec<SegmentResolution> {
    let mut by_index: std::collections::BTreeMap<u32, String> = std::collections::BTreeMap::new();
    for m in snapshots {
        for r in &m.segment_resolutions {
            by_index.insert(r.segment, r.action.clone());
        }
    }
    by_index
        .into_iter()
        .map(|(segment, action)| SegmentResolution { segment, action })
        .collect()
}

/// Aggregate a session's snapshots into consecutive same-hero runs (sorted by
/// capture time here, so any input order works). "Unknown"/empty hero reads
/// neither open nor split a run. A run of ≥2 snapshots is confirmed; a
/// 1-snapshot run is recorded but unconfirmed. Manual `resolutions` (by
/// segment index) are then applied: confirm forces `confirmed`, dismiss marks
/// the segment a misread.
pub fn derive_hero_segments(
    snapshots: &[PersonalMatch],
    resolutions: &[SegmentResolution],
) -> Vec<HeroSegment> {
    let mut ordered: Vec<&PersonalMatch> = snapshots.iter().collect();
    ordered.sort_by_key(|m| {
        let dt: chrono::DateTime<chrono::Utc> = m.played_at.into();
        dt
    });
    let mut segments: Vec<HeroSegment> = Vec::new();
    for m in ordered {
        if m.hero.is_empty() || m.hero == "Unknown" {
            continue;
        }
        match segments.last_mut() {
            Some(seg) if seg.hero == m.hero => {
                seg.last_seen = m.played_at;
                seg.snapshots += 1;
            }
            _ => segments.push(HeroSegment {
                hero: m.hero.clone(),
                role: m.role.clone(),
                first_seen: m.played_at,
                last_seen: m.played_at,
                snapshots: 1,
                confirmed: false,
                resolution: None,
            }),
        }
    }
    for seg in &mut segments {
        seg.confirmed = seg.snapshots >= 2;
    }
    for r in resolutions {
        if let Some(seg) = segments.get_mut(r.segment as usize) {
            seg.resolution = Some(r.action.clone());
            match r.action.as_str() {
                SEGMENT_CONFIRM => seg.confirmed = true,
                SEGMENT_DISMISS => seg.confirmed = false,
                _ => {}
            }
        }
    }
    segments
}

/// The majority hero across a session's snapshots. Individual captures can
/// mislabel — the career panel shows the SPECTATED hero while the player is
/// dead, and portrait matching can misfire — but across 20+ captures the
/// player's real hero dominates. "Unknown" rows don't vote; ties break
/// alphabetically for determinism.
pub fn majority_hero(snapshots: &[PersonalMatch]) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in snapshots {
        if m.hero != "Unknown" && !m.hero.is_empty() {
            *counts.entry(m.hero.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(hero, _)| hero.to_string())
}

/// Majority map across a session's snapshots (empty reads don't vote).
/// Repair helper for sessions whose map was missed at creation or polluted
/// by misreads.
pub fn majority_map(snapshots: &[PersonalMatch]) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in snapshots {
        if !m.map_name.is_empty() {
            *counts.entry(m.map_name.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(map, _)| map.to_string())
}

/// `true` when this row has no name recognizer to send. An empty string is
/// the same as absent: the upload omits the field.
pub(super) fn name_recognizer_absent(id: &Option<String>) -> bool {
    id.as_deref().is_none_or(str::is_empty)
}

/// Collapse capture snapshots (multiple Tab presses during one match) to one
/// row per game. Snapshots of the same game share a `session_id`, and the
/// newest snapshot carries the final scoreboard — so with newest-first input
/// (the order of `get_all_matches`, the live snapshot, and `read_match_log`)
/// the first row seen per session wins. Rows without a session_id (legacy
/// data) pass through individually. Games whose rows predate the hero
/// timeline (empty `heroes_played`) get their segments derived on the fly
/// from the session snapshots present in the input, so legacy data shows the
/// same swap history and majority primary as fresh captures.
pub fn latest_per_game(matches: Vec<PersonalMatch>) -> Vec<PersonalMatch> {
    let mut seen = std::collections::HashSet::new();
    let mut legacy: std::collections::HashMap<String, Vec<PersonalMatch>> =
        std::collections::HashMap::new();
    let mut games: Vec<PersonalMatch> = Vec::new();
    for m in matches {
        if m.session_id.is_empty() {
            games.push(m);
            continue;
        }
        if seen.insert(m.session_id.clone()) {
            if m.heroes_played.is_empty() {
                legacy.insert(m.session_id.clone(), vec![m.clone()]);
            }
            games.push(m);
        } else {
            // The newest snapshot is the uploaded row. If it did not read a
            // name, keep the recognizer from an earlier snapshot of the same
            // match so a resend does not clear it on the server.
            if let Some(game) = games.iter_mut().find(|g| g.session_id == m.session_id)
                && name_recognizer_absent(&game.name_recognizer)
                && !name_recognizer_absent(&m.name_recognizer)
            {
                game.name_recognizer = m.name_recognizer.clone();
            }
            if let Some(snaps) = legacy.get_mut(&m.session_id) {
                snaps.push(m);
            }
        }
    }
    for g in &mut games {
        if let Some(snaps) = legacy.remove(&g.session_id) {
            g.heroes_played = derive_hero_segments(&snaps, &g.segment_resolutions);
        }
    }
    games
}
