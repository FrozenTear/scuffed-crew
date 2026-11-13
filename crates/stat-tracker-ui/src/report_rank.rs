//! The "Include rank screen" box of a game report, and the preview lines
//! for files a report left out to stay within the site's limits
//! (`report-bundle.md` sections 3 and 8).

use iced::Element;
use iced::widget::{checkbox, column, text};
use stat_tracker::report::ReportZip;

use super::{ReportDialog, ReportMessage, msg};
use crate::app::Message;
use crate::theme::{FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT_2, TEXT_3};

/// Label of the "Include rank screen" box.
pub const RANK_BOX_LABEL: &str = "Include rank screen";
/// The line under it.
pub const RANK_BOX_HINT: &str =
    "Sends only two cut-outs of the rank screen: the rank panel and the queue banner. They help improve the rank reader.";

/// The "Include rank screen" box of a game report, and its hint.
pub(super) fn rank_box(d: &ReportDialog) -> Element<'_, Message> {
    let mut rank = checkbox(d.rank_included)
        .label(RANK_BOX_LABEL)
        .size(18)
        .text_size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .style(crate::uninstall::checkbox_style);
    if !d.busy() && !d.sent() {
        rank = rank.on_toggle(|on| msg(ReportMessage::RankIncluded(on)));
    }
    column![
        rank,
        text(RANK_BOX_HINT)
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(2)
    .into()
}

/// One short plain line per file the report left out, such as
/// "Rank screen left out: over the size limit."
pub fn left_out_lines(zip: &ReportZip) -> Vec<String> {
    zip.left_out.iter().map(|l| l.preview_line()).collect()
}

/// The preview's left-out lines, under the file list.
pub(super) fn left_out_text<'a>(line: String) -> Element<'a, Message> {
    text(line)
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_2)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stat_tracker::report::{LeftOut, LeftOutReason};

    fn zip(left_out: Vec<LeftOut>) -> ReportZip {
        ReportZip {
            bytes: Vec::new(),
            manifest_json: String::new(),
            files: Vec::new(),
            log_text: String::new(),
            images: Vec::new(),
            left_out,
        }
    }

    /// The hint says exactly what leaves the PC: two cut-outs, never the
    /// whole rank screen.
    #[test]
    fn the_rank_box_hint_names_the_two_cut_outs() {
        assert_eq!(
            RANK_BOX_HINT,
            "Sends only two cut-outs of the rank screen: the rank panel and the queue banner. They help improve the rank reader."
        );
    }

    #[test]
    fn the_preview_says_which_rank_crop_was_left_out_and_why() {
        assert!(left_out_lines(&zip(Vec::new())).is_empty());
        let lines = left_out_lines(&zip(vec![
            LeftOut {
                path: "crops/rank.png".into(),
                reason: LeftOutReason::OverSizeLimit,
            },
            LeftOut {
                path: "crops/rank_banner.png".into(),
                reason: LeftOutReason::TooManyFiles,
            },
        ]));
        assert_eq!(
            lines,
            [
                "Rank screen left out: over the size limit.",
                "Rank banner left out: too many files for one report.",
            ]
        );
        for line in &lines {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }
}
