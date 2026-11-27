//! The Games table's Rank cell: what the rank reader read on the rank update
//! screen after the game ([`Game::rank_update`]). The words are the site's
//! (`crates/app/src/pages/stats/rank.rs`), copied here so the tracker and
//! the site say the same thing for the same game: "Diamond 2", "45%",
//! "+23%", and one sentence for the tooltip.
//!
//! The values shown are the ones the upload sends the site for this game
//! (`stat_tracker::sync` `upload_rank`), shown as the site shows them: a
//! value only with a confidence the upload keeps (finite, above 0, at most
//! 1), text only when it is at most 64 characters once trimmed, the tier
//! word in title case (the pack may spell it in capitals), the division
//! 1..=5, progress and change -100..=100, the role only when the upload
//! would send it (`Open` only with an open queue title, a role icon never
//! with one), and the queue as the queue chip names it: the `rank.queue`
//! the upload sends (`games_queue`). A game with nothing to show keeps the
//! plain "-" the column had before ranks were read, and so does a game
//! whose confident title names a mode that is not Competitive (Quick
//! Play, Stadium...), as the upload sends no rank for it.

use iced::widget::text::Wrapping;
use iced::widget::{column, container, text, tooltip};
use iced::{Element, Padding};

use crate::app::Message;
use crate::games_table::pill_box;
use crate::model::Game;
use crate::theme::{BORDER, SIZE_LABEL, SIZE_META, TEXT, TEXT_2, TEXT_3};
use stat_tracker::model::{RankNumber, RankText};

/// A missing or unsure role, as the site words it.
pub const ROLE_UNKNOWN: &str = "Role unknown";

/// The tooltip's second sentence when progress is below 0.
pub const DEMOTION_PROTECTION: &str = "Negative progress means demotion protection.";

/// The tooltip text size (the bans and picks tooltips use the same).
const TIP_SIZE: f32 = 12.0;

/// What one game's rank shows: every value that is in range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RankShown {
    /// "Tank", "Damage", "Support" or "Open queue".
    pub role: Option<&'static str>,
    /// Tier name in title case, e.g. "Diamond".
    pub tier: Option<String>,
    /// 1..=5.
    pub division: Option<u8>,
    /// -100..=100. A negative value is demotion protection and shows as is.
    pub progress: Option<i32>,
    /// Progress points gained or lost this game, -100..=100.
    pub change: Option<i32>,
    /// The queue in the chip's words (`games_queue::label`), when the
    /// upload sends one.
    pub queue: Option<String>,
}

impl RankShown {
    /// None when the game has no rank, no rank value the cell can show
    /// (a rank text, progress or change), or a confident title of a mode
    /// that is not Competitive, for which the upload sends no rank.
    pub fn from_game(g: &Game) -> Option<Self> {
        let rank = g.rank_update.as_ref()?;
        let title = g.queue_title.as_ref().and_then(|t| t.confident_value());
        if title.is_some_and(stat_tracker::rank_queue::title_rules_out_competitive) {
            return None;
        }
        let open_queue = title.is_some_and(stat_tracker::rank_queue::title_is_open);
        // A word or number the upload sends: one read with a confidence it
        // keeps ([`sure`]), and text of a length it keeps ([`upload_text`]).
        fn word(w: &Option<RankText>) -> Option<&str> {
            w.as_ref()
                .filter(|w| sure(w.confidence).is_some())
                .map(|w| w.value.as_str())
        }
        let num = |n: &Option<RankNumber>| {
            n.as_ref()
                .filter(|n| sure(n.confidence).is_some())
                .and_then(|n| i32::try_from(n.value).ok())
        };
        let shown = Self {
            role: word(&rank.role).and_then(|r| role_name(r, open_queue)),
            tier: word(&rank.role_rank)
                .and_then(upload_text)
                .and_then(|t| clean_text(&t))
                .map(|t| title_case(&t.to_lowercase())),
            division: num(&rank.division)
                .and_then(|d| u8::try_from(d).ok())
                .filter(|d| (1..=5).contains(d)),
            progress: num(&rank.progress_pct).filter(|p| (-100..=100).contains(p)),
            change: num(&rank.progress_change).filter(|c| (-100..=100).contains(c)),
            queue: crate::games_detail::queue_value(g).map(str::to_string),
        };
        (!shown.badge_parts().is_empty()).then_some(shown)
    }

    /// "Diamond 2", "Diamond", "Division 2", or None.
    pub fn rank_text(&self) -> Option<String> {
        match (&self.tier, self.division) {
            (Some(t), Some(d)) => Some(format!("{t} {d}")),
            (Some(t), None) => Some(t.clone()),
            (None, Some(d)) => Some(format!("Division {d}")),
            (None, None) => None,
        }
    }

    /// "Support, 5V5 ROLE QUEUE", or the role alone. See [`context_label`].
    pub fn context(&self) -> String {
        context_label(self.role, self.queue.as_deref())
    }

    /// The short visible pieces: rank, progress, change.
    pub fn badge_parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(rank) = self.rank_text() {
            parts.push(rank);
        }
        if let Some(p) = self.progress {
            parts.push(format!("{p}%"));
        }
        if let Some(c) = self.change {
            parts.push(change_text(c));
        }
        parts
    }

    /// The tooltip: one sentence, and with negative progress a second one
    /// that says what it means.
    pub fn sentence(&self) -> String {
        let mut parts = vec![self.context()];
        if let Some(rank) = self.rank_text() {
            parts.push(rank);
        }
        if let Some(p) = self.progress {
            parts.push(format!("{p}% progress"));
        }
        if let Some(c) = self.change {
            parts.push(format!("{} this game", change_text(c)));
        }
        let rank = format!("Rank: {}", parts.join(", "));
        if self.progress.is_some_and(|p| p < 0) {
            format!("{rank}. {DEMOTION_PROTECTION}")
        } else {
            rank
        }
    }

    /// The cell's lines: the rank text, then progress and change ("45%
    /// +23%") smaller under it. One number alone says which it is, in the
    /// tooltip's words ("45% progress", "-5% this game"). Without a rank
    /// text, the numbers are the one line.
    pub fn lines(&self) -> Vec<String> {
        let numbers = match (self.progress, self.change) {
            (Some(p), Some(c)) => Some(format!("{p}% {}", change_text(c))),
            (Some(p), None) => Some(format!("{p}% progress")),
            (None, Some(c)) => Some(format!("{} this game", change_text(c))),
            (None, None) => None,
        };
        self.rank_text().into_iter().chain(numbers).collect()
    }
}

/// The stored role as the site shows it, or None when the upload would not
/// send it: the site's `RankRole` spellings only, `Open` only with an open
/// queue title, and a role icon never with one.
fn role_name(value: &str, open_queue: bool) -> Option<&'static str> {
    use scuffed_types::api::RankRole;
    match stat_tracker::rank_queue::parse_rank_role(value)? {
        RankRole::Tank if !open_queue => Some("Tank"),
        RankRole::Damage if !open_queue => Some("Damage"),
        RankRole::Support if !open_queue => Some("Support"),
        RankRole::Open if open_queue => Some(OPEN_ROLE),
        _ => None,
    }
}

/// Role and queue for display: "Support, 5V5 ROLE QUEUE". The queue is
/// spelled as its chip and the Map tooltip spell it (`games_queue`). A
/// missing queue is left out. The Open role comes only with an open queue
/// title, so beside an open queue it shows the queue alone, "6V6 OPEN
/// QUEUE" or, when the size is unknown, "Open queue, size unknown": open
/// queue is said once and never next to "queue unknown".
pub fn context_label(role: Option<&str>, queue: Option<&str>) -> String {
    use crate::games_queue::{OPEN_5V5, OPEN_6V6, OPEN_SIZE_UNKNOWN};
    let role = role.unwrap_or(ROLE_UNKNOWN);
    match queue {
        Some(q) if role == OPEN_ROLE && [OPEN_5V5, OPEN_6V6, OPEN_SIZE_UNKNOWN].contains(&q) => {
            q.to_string()
        }
        Some(q) if !q.eq_ignore_ascii_case(role) => format!("{role}, {q}"),
        _ => role.to_string(),
    }
}

/// The Open role as the cell names it.
const OPEN_ROLE: &str = "Open queue";

/// "+23%", "-15%", and "±0%" for no change, so every change shows a sign.
pub fn change_text(change: i32) -> String {
    match change {
        0 => "±0%".to_string(),
        c if c > 0 => format!("+{c}%"),
        c => format!("{c}%"),
    }
}

/// A confidence the upload keeps: finite, above 0, at most 1. Copied from
/// `upload_rank`'s `sure` (`crates/stat-tracker/src/sync/wire.rs`), which
/// the GUI cannot reach.
fn sure(confidence: f32) -> Option<f32> {
    (confidence.is_finite() && confidence > 0.0 && confidence <= 1.0).then_some(confidence)
}

/// Text the upload sends: trimmed, not empty, at most
/// `scuffed_types::api::RANK_STRING_MAX_CHARS` characters. Copied from
/// `upload_rank`'s `text` (`crates/stat-tracker/src/sync/wire.rs`).
fn upload_text(v: &str) -> Option<String> {
    let v = v.trim();
    (!v.is_empty() && v.chars().count() <= scuffed_types::api::RANK_STRING_MAX_CHARS)
        .then(|| v.to_string())
}

/// Trimmed, single spaced, or None when blank.
fn clean_text(text: &str) -> Option<String> {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!joined.is_empty()).then_some(joined)
}

/// "DIAMOND" to "Diamond" (lowered first by the caller, as the site does).
fn title_case(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The Rank cell: the rank text with progress and change under it, and the
/// sentence as its tooltip. A game with no rank draws "-" as before.
pub fn cell(g: &Game) -> Element<'static, Message> {
    let Some(shown) = RankShown::from_game(g) else {
        return text("-").size(SIZE_META).color(TEXT_3).into();
    };
    let mut lines = shown.lines().into_iter();
    let mut body = column![];
    if let Some(top) = lines.next() {
        body = body.push(
            text(top)
                .size(SIZE_META)
                .color(TEXT_2)
                .wrapping(Wrapping::None),
        );
    }
    for line in lines {
        body = body.push(
            text(line)
                .size(SIZE_LABEL)
                .color(TEXT_3)
                .wrapping(Wrapping::None),
        );
    }
    let tip = container(text(shown.sentence()).size(TIP_SIZE).color(TEXT))
        .padding(Padding::from([4, 8]))
        .max_width(360)
        .style(pill_box(BORDER));
    tooltip(body, tip, tooltip::Position::Top).gap(4).into()
}

#[cfg(test)]
#[path = "games_rank_tests.rs"]
mod tests;
