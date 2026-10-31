//! Classifying a polled frame for the pre-game saves ([`Seen`]), with the
//! numbers of each check it turned down ([`Classified`]).

use super::*;

/// Classify a polled frame for the tracker, on the blocking thread, with
/// the process's words-v1 reader for the select title and `examples` for
/// the ban-results title.
#[cfg(test)]
pub(crate) fn classify(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
    examples: &super::pregame_ban_title::BanTitleExamples,
) -> Seen {
    classify_checked(want, phase, rgb, examples).seen
}

/// [`classify`] with the numbers of each check it turned down.
pub(crate) fn classify_checked(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
    examples: &super::pregame_ban_title::BanTitleExamples,
) -> Classified {
    classify_checked_with(
        want,
        phase,
        rgb,
        |rgb| {
            shows_select_title(shadow::read_phrase(
                stat_tracker::shadow::result::phrase::Phrase::SelectAPreferredHero,
                rgb,
            ))
        },
        |rgb| examples.check(rgb),
    )
}

/// [`classify`] with the two title reads as arguments (tests).
///
/// The loading check runs only when `want.loading` and the frame is no
/// phase, and a frame passing the hero-ban pixel gate (a ban screen before
/// its read settles) is never the loading screen. The cheap loading check
/// runs first. The map-vote gate is not used: a vote screen's navy is too
/// bright to pass the loading check's dark bands, so it only turned away a
/// loading screen whose black is tinted navy. The hero-select gate is not
/// used either: its colour spread can pass on the loading screen's glyphs.
#[cfg(test)]
pub(crate) fn classify_with(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
    select_title: impl FnOnce(&image::RgbImage) -> bool,
    ban_title: impl FnOnce(&image::RgbImage) -> bool,
) -> Seen {
    classify_checked_with(want, phase, rgb, select_title, |rgb| {
        super::pregame_ban_title::BanCheck {
            pass: ban_title(rgb),
            ..Default::default()
        }
    })
    .seen
}

/// [`classify_with`] with the ban-results title's numbers, and the numbers
/// of each check it turned down ([`Rejects`]). The roster layout check
/// ([`detect::roster_screen`]) runs last, only when `want.roster`: the
/// ban-results title, a select title or the loading screen comes first.
pub(crate) fn classify_checked_with(
    want: PregameWant,
    phase: &detect::GamePhase,
    rgb: &image::RgbImage,
    select_title: impl FnOnce(&image::RgbImage) -> bool,
    ban_title: impl FnOnce(&image::RgbImage) -> super::pregame_ban_title::BanCheck,
) -> Classified {
    if let Some(p) = PregamePhase::of(phase) {
        return Seen::Phase(p).into();
    }
    let mut rejects = Rejects::default();
    // [`detect::loading_screen::is_entering_game`], with its numbers kept
    // for a frame that passes the slab and fails the text.
    if want.loading && detect::loading_screen::is_loading_screen(rgb) {
        let text = detect::loading_screen::text_box(rgb);
        let entering = text.is_some_and(|t| t.is_entering_game_text());
        let ban_gate = entering && detect::match_start::passes_ban_gate(rgb);
        if entering && !ban_gate {
            return Seen::Loading.into();
        }
        rejects.loading = Some(LoadingReject { text, ban_gate });
    }
    let seen = |seen| Classified { seen, rejects };
    if want.select_title && select_title(rgb) {
        return seen(Seen::SelectTitle);
    }
    if want.ban_title {
        let check = ban_title(rgb);
        if check.pass {
            return seen(Seen::BanResults);
        }
        // A near miss only: a roster or plain frame in the wait is no
        // ban-results candidate.
        rejects.ban_results = Some(check).filter(|c| c.near_miss());
    }
    if want.roster {
        let scores = detect::roster_screen::scores(rgb);
        if scores.is_roster() {
            return Classified {
                seen: Seen::Roster,
                rejects,
            };
        }
        if scores.near_miss() {
            rejects.roster = Some(scores);
        }
    }
    Classified {
        seen: Seen::Other,
        rejects,
    }
}

/// Whether a words-v1 read of SELECT A PREFERRED HERO says the hero select
/// is on the frame. The mask is the pack's own cut of the select title
/// region (Vision's 0.41,0.165,0.565,0.2 at 1440p), so no new reference
/// crops are needed. Conservative on purpose: a suspect read (the title
/// half drawn while the screen slides in) counts too, since a frame that
/// is turned away here costs at most the results frame, while one let
/// through is a select frame saved as the results. No mask loaded (`None`)
/// is no title.
pub(crate) fn shows_select_title(
    read: Option<stat_tracker::shadow::result::phrase::PhraseRead>,
) -> bool {
    use stat_tracker::shadow::result::phrase::PhraseRead;
    matches!(read, Some(PhraseRead::Sure(_) | PhraseRead::Suspect(_)))
}

/// Whether a tick seeing `seen` should hand its frame to the tracker.
pub(crate) fn keeps_frame(want: PregameWant, seen: Seen) -> bool {
    match seen {
        Seen::Phase(_) => true,
        Seen::Loading | Seen::BanResults => want.other_frame,
        Seen::Roster => want.roster,
        Seen::Other | Seen::SelectTitle => false,
    }
}

/// [`keeps_frame`], and a tick that turned down a candidate whose
/// `rejected_<label>.png` may be written now keeps its frame too.
pub(crate) fn keeps_checked(want: PregameWant, c: &Classified) -> bool {
    keeps_frame(want, c.seen) || c.rejects.labels().any(|l| want.rejected & l.bit() != 0)
}
