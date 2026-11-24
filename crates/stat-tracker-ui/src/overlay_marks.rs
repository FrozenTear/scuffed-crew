//! Helpers for the labels the Tonight strip, the page indicator, and the
//! safe-page footer pass to the view. Tests that call `view` and then
//! read these helpers check the labels the view was given; they do not
//! parse the view tree.

use crate::model::Outcome;
use crate::overlay::outcome_letter;
use crate::overlay_pages::{self, PageState};

/// Newest games kept as squares in the Tonight strip; the rest is "+N".
pub(crate) const TONIGHT_STRIP_MAX: usize = 13;

/// The Tonight pip letter, so colour is never the only cue for that page.
pub(crate) const TONIGHT_PIP: &str = "T";

/// The squares to draw (newest first) and, when there are more than
/// [`TONIGHT_STRIP_MAX`], the count hidden behind "+N".
pub(crate) fn tonight_strip_cells(outcomes: &[Outcome]) -> (&[Outcome], Option<usize>) {
    if outcomes.len() <= TONIGHT_STRIP_MAX {
        (outcomes, None)
    } else {
        (
            &outcomes[..TONIGHT_STRIP_MAX],
            Some(outcomes.len() - TONIGHT_STRIP_MAX),
        )
    }
}

/// Labels the Tonight strip draws: a letter per shown square, or "-", and
/// "+N" when games past the cap are hidden.
pub(crate) fn tonight_strip_labels(outcomes: &[Outcome]) -> Vec<String> {
    if outcomes.is_empty() {
        return vec!["-".into()];
    }
    let (shown, extra) = tonight_strip_cells(outcomes);
    let mut labels: Vec<String> = shown
        .iter()
        .map(|o| outcome_letter(*o).to_string())
        .collect();
    if let Some(n) = extra {
        labels.push(format!("+{n}"));
    }
    labels
}

/// Labels the page indicator draws: the Tonight pip, the page name, and
/// the held tag when it is on.
pub(crate) fn page_indicator_labels(pages: &PageState) -> Vec<&'static str> {
    let ind = overlay_pages::indicator(pages);
    let mut labels = Vec::new();
    if ind.dot.is_some() {
        labels.push(TONIGHT_PIP);
    }
    labels.push(ind.name);
    if ind.held {
        labels.push(overlay_pages::HELD_TAG);
    }
    labels
}

/// The safe-page footer line the view is given, or `None` when the hold
/// is off.
pub(crate) fn safe_page_footer_label(pages: &PageState, bind: &str) -> Option<String> {
    pages.held.then(|| overlay_pages::safe_page_footer(bind))
}
