//! Turned-down candidates of the pre-game saves: the numbers of each check
//! a tick turned down ([`Rejects`]), one rate limited debug line per label
//! and the last such frame kept as `rejected_<label>.png` in the game's
//! folder ([`RejectLimiter`]).

use super::*;

/// At most one debug line a second per label for turned-down candidates.
pub(crate) const REJECT_LOG_EVERY: Duration = Duration::from_secs(1);

/// The same numbers again (rounded as logged) are logged only this long
/// after the last line of that label.
pub(crate) const REJECT_LOG_SAME: Duration = Duration::from_secs(30);

/// A `rejected_<label>.png` is written at most this often per label.
pub(crate) const REJECTED_FRAME_EVERY: Duration = Duration::from_secs(10);

/// What a turned-down candidate was taken for: its debug line says so, and
/// the last one of each label is kept as `rejected_<label>.png` in the
/// game's folder, replaced by newer ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectLabel {
    Loading,
    BanResults,
    Roster,
}

impl RejectLabel {
    pub(crate) const ALL: [Self; 3] = [Self::Loading, Self::BanResults, Self::Roster];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::BanResults => "ban_results",
            Self::Roster => "roster",
        }
    }

    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::Loading => "rejected_loading.png",
            Self::BanResults => "rejected_ban_results.png",
            Self::Roster => "rejected_roster.png",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Loading => 0,
            Self::BanResults => 1,
            Self::Roster => 2,
        }
    }

    pub(crate) fn bit(self) -> u8 {
        1 << self.index()
    }
}

/// A loading candidate turned down: the frame passed the loading slab
/// check, then the text rule ([`detect::loading_screen::TextBox`]: A the
/// glyph samples, B the text width, C the word ratio, and the spinner) or
/// the hero-ban gate turned it away.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LoadingReject {
    /// `None`: fewer glyph samples than rule A needs (no text yet, or a
    /// fade-in too dim for the glyph level).
    pub(crate) text: Option<detect::loading_screen::TextBox>,
    /// The text passed, but the frame passes the hero-ban gate.
    pub(crate) ban_gate: bool,
}

/// The checks a tick ran and turned down, with their numbers, by label.
/// Only near misses count for the roster (at least one part passed), so a
/// game frame is no roster candidate.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Rejects {
    pub(crate) loading: Option<LoadingReject>,
    pub(crate) ban_results: Option<super::pregame_ban_title::BanCheck>,
    pub(crate) roster: Option<detect::roster_screen::RosterScores>,
}

impl Rejects {
    /// Each turned-down label and its numbers, as the debug line says them
    /// (rounded, so a small drift is the same line).
    pub(crate) fn details(&self) -> Vec<(RejectLabel, String)> {
        let mut out = Vec::new();
        if let Some(l) = &self.loading {
            let text = match &l.text {
                None => "A samples under 24 (no text at the glyph level: fade-in)".to_string(),
                Some(t) => format!(
                    "A samples {}, B width {:.2}, C word ratio {}, words {}, spinner {}, centre x {:.2}, centre y {:.2}, height {:.2}",
                    t.samples,
                    t.width,
                    t.word_ratio
                        .map_or("none".to_string(), |r| format!("{r:.2}")),
                    t.words,
                    t.spinner,
                    t.centre_x,
                    t.centre_y,
                    t.height,
                ),
            };
            out.push((
                RejectLabel::Loading,
                format!("{text}; hero-ban gate {}", l.ban_gate),
            ));
        }
        if let Some(b) = &self.ban_results {
            let numbers = match (&b.shape, b.score) {
                (_, Some(score)) => format!(
                    "example score {score:.2} (min {})",
                    super::pregame_ban_title::BAN_TITLE_MATCH_MIN
                ),
                (Some(m), None) => format!(
                    "shape bright {:.2}, dark {:.2}, centroid {}, span {:.2}, top {:.2}, bottom {:.2}",
                    m.bright,
                    m.dark,
                    m.centroid.map_or("none".to_string(), |c| format!("{c:.2}")),
                    m.span,
                    m.top_bright,
                    m.bottom_bright,
                ),
                (None, None) => "no title region".to_string(),
            };
            out.push((RejectLabel::BanResults, numbers));
        }
        if let Some(r) = &self.roster {
            out.push((RejectLabel::Roster, r.describe()));
        }
        out
    }

    pub(super) fn labels(&self) -> impl Iterator<Item = RejectLabel> + '_ {
        RejectLabel::ALL.into_iter().filter(|l| match l {
            RejectLabel::Loading => self.loading.is_some(),
            RejectLabel::BanResults => self.ban_results.is_some(),
            RejectLabel::Roster => self.roster.is_some(),
        })
    }
}

/// A classified tick: what it saw and what it turned down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Classified {
    pub(crate) seen: Seen,
    pub(crate) rejects: Rejects,
}

impl From<Seen> for Classified {
    fn from(seen: Seen) -> Self {
        Self {
            seen,
            rejects: Rejects::default(),
        }
    }
}

/// Rate limits for turned-down candidates, per label: at most one debug
/// line per [`REJECT_LOG_EVERY`], and the same numbers again only after
/// [`REJECT_LOG_SAME`]; at most one `rejected_<label>.png` write per
/// [`REJECTED_FRAME_EVERY`].
#[derive(Debug, Default)]
pub(crate) struct RejectLimiter {
    logged: [Option<(Instant, String)>; 3],
    written: [Option<Instant>; 3],
}

impl RejectLimiter {
    /// Whether a line with `detail` for `label` is logged at `now`; when
    /// it is, it counts as the last one.
    pub(crate) fn log_due(&mut self, label: RejectLabel, now: Instant, detail: &str) -> bool {
        let slot = &mut self.logged[label.index()];
        let due = slot.as_ref().is_none_or(|(at, last)| {
            let since = now.saturating_duration_since(*at);
            since >= REJECT_LOG_EVERY && (last != detail || since >= REJECT_LOG_SAME)
        });
        if due {
            *slot = Some((now, detail.to_string()));
        }
        due
    }

    /// Whether a `rejected_<label>.png` may be written at `now`.
    pub(crate) fn frame_due(&self, label: RejectLabel, now: Instant) -> bool {
        self.written[label.index()]
            .is_none_or(|at| now.saturating_duration_since(at) >= REJECTED_FRAME_EVERY)
    }

    pub(crate) fn frame_written(&mut self, label: RejectLabel, now: Instant) {
        self.written[label.index()] = Some(now);
    }

    /// [`RejectLabel::bit`]s of the labels [`Self::frame_due`] at `now`.
    pub(super) fn due_bits(&self, now: Instant) -> u8 {
        RejectLabel::ALL
            .into_iter()
            .filter(|l| self.frame_due(*l, now))
            .fold(0, |bits, l| bits | l.bit())
    }
}

impl<F: Clone> PregameTracker<F> {
    /// Log each turned-down check of this tick and keep its frame as
    /// `rejected_<label>.png`, rate limited, in the open game only.
    pub(super) fn note_rejects(
        &mut self,
        now: Instant,
        rejects: &Rejects,
        frame: Option<F>,
        shots: &mut Vec<PregameOp<F>>,
    ) {
        if self.game_open(now).is_none() {
            return;
        }
        for (label, detail) in rejects.details() {
            self.turned_down(now, label, &detail, frame.clone(), shots);
        }
    }

    /// One turned-down `label` candidate with its numbers `detail`: one
    /// debug line, and `frame` written over `rejected_<label>.png` in the
    /// open game's folder, each when the limiter lets it.
    pub(super) fn turned_down(
        &mut self,
        now: Instant,
        label: RejectLabel,
        detail: &str,
        frame: Option<F>,
        shots: &mut Vec<PregameOp<F>>,
    ) {
        if self.rejects.log_due(label, now, detail) {
            tracing::debug!(
                target: "scuffed_stat_tracker",
                label = label.name(),
                detail,
                "pre-game candidate turned down"
            );
        }
        let Some(folder) = self.game_open(now).map(|g| g.folder.clone()) else {
            return;
        };
        if let Some(frame) = frame
            && self.rejects.frame_due(label, now)
        {
            self.rejects.frame_written(label, now);
            shots.push(PregameOp::Save(PregameShot {
                folder,
                name: label.file_name(),
                frame,
            }));
        }
    }
}
