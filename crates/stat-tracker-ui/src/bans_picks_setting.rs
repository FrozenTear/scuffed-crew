//! Opening Settings from a game whose bans were not read because the
//! reader was off, or because the pack failed: jump the Settings pane to
//! the Read bans and picks row and mark it with a 1 px accent outline.

use std::time::{Duration, Instant};

use iced::advanced::widget::operation::{self, Operation, Outcome};
use iced::advanced::widget::{Id, operation::scrollable::AbsoluteOffset};
use iced::{Event, Rectangle, Task, Vector, mouse, touch};

use crate::app::Message;
use crate::focus_status::PANE_SCROLL_ID;
use stat_tracker::model::BansPicksStatus;

/// The Read bans and picks row on the 0.6 Settings page.
pub const ROW_ID: &str = "settings-bans-picks-row";
/// How long the row stays marked after "Open Settings" opened it.
pub const HIGHLIGHT_FOR: Duration = Duration::from_secs(2);
/// Room left above the row when the pane jumps to it, enough that the
/// bottom of the row above stays visible and it is clear the pane is
/// partway down Capture and reading.
pub const ROW_MARGIN: f32 = 56.0;
/// How far the 1 px accent outline sits outside the row's content, so
/// it frames the row instead of hugging the label and help. The outline
/// uses the gaps and card padding around the row, so the label and
/// toggle stay lined up with the rows around it.
pub const ROW_OUTLINE_PAD: f32 = 8.0;

/// A reader-off or pack-failed game row scrolls to and marks the switch.
/// Other "Open Settings" reasons open the section and stop there.
pub fn scrolls_to_row(status: Option<&BansPicksStatus>) -> bool {
    matches!(
        status,
        Some(BansPicksStatus::ReaderOff | BansPicksStatus::PackFailed)
    )
}

/// Whether `row` is already fully inside `viewport`.
pub fn row_fully_visible(viewport: Rectangle, row: Rectangle) -> bool {
    let row_bottom = row.y + row.height;
    let view_bottom = viewport.y + viewport.height;
    row.y >= viewport.y && row_bottom <= view_bottom
}

/// The pane offset that puts `row` in view with `margin` px of space
/// above its top (the accent outline sits on that top), or `None` when
/// the whole row is already visible at `translation_y` (a taller
/// window, or the pane already sitting at this offset). The jump is an
/// absolute offset, not a smooth scroll.
///
/// Layout bounds are untranslated, so a translation of `T` draws the
/// row at `row.y - T`. Visibility is checked on that drawn position.
/// The jump target stays absolute: we want the drawn top to land
/// `margin` px below the visible pane top (`viewport.y`), so
/// `T = row.y - viewport.y - margin`.
pub fn reveal_offset(
    viewport: Rectangle,
    row: Rectangle,
    margin: f32,
    translation_y: f32,
) -> Option<f32> {
    let drawn = Rectangle {
        y: row.y - translation_y,
        ..row
    };
    if row_fully_visible(viewport, drawn) {
        return None;
    }
    Some((row.y - viewport.y - margin).max(0.0))
}

/// A 2 s mark on the Read bans and picks row: on until it times out, or
/// until a click or scroll clears it in one step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowMark {
    until: Option<Instant>,
}

impl RowMark {
    /// Start (or restart) the mark at `now`.
    pub fn start(&mut self, now: Instant) {
        self.until = Some(now + HIGHLIGHT_FOR);
    }

    /// Whether the mark is still on at `now`.
    pub fn is_on(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now < until)
    }

    /// Drop the mark at once: no fade.
    pub fn clear(&mut self) {
        self.until = None;
    }
}

/// A user click or scroll, which must clear the mark at once.
pub fn is_click_or_scroll(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(mouse::Event::ButtonPressed(_))
            | Event::Mouse(mouse::Event::WheelScrolled { .. })
            | Event::Touch(touch::Event::FingerPressed { .. })
    )
}

/// Scroll the Settings pane to the Read bans and picks row after the
/// page is laid out. Nothing happens when the row is already fully in
/// view, or when it is not on the page.
pub fn scroll_to_row() -> Task<Message> {
    iced::advanced::widget::operate(RevealRow {
        viewport: None,
        row: None,
        translation: Vector::ZERO,
    })
}

/// Finds the pane and the row, then jumps the pane if the row is not
/// fully visible.
#[derive(Debug)]
struct RevealRow {
    viewport: Option<Rectangle>,
    row: Option<Rectangle>,
    translation: Vector,
}

impl<T: Send + 'static> Operation<T> for RevealRow {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&Id::new(ROW_ID)) {
            self.row = Some(bounds);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(PANE_SCROLL_ID)) {
            self.viewport = Some(bounds);
            self.translation = translation;
        }
    }

    fn finish(&self) -> Outcome<T> {
        match (self.viewport, self.row) {
            (Some(viewport), Some(row)) => {
                match reveal_offset(viewport, row, ROW_MARGIN, self.translation.y) {
                    Some(y) => Outcome::Chain(Box::new(operation::scrollable::scroll_to(
                        Id::new(PANE_SCROLL_ID),
                        AbsoluteOffset {
                            x: None,
                            y: Some(y),
                        },
                    ))),
                    None => Outcome::None,
                }
            }
            _ => Outcome::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use iced::Color;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rectangle {
        Rectangle {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn reader_off_and_pack_failed_scroll_to_the_row() {
        assert!(scrolls_to_row(Some(&BansPicksStatus::ReaderOff)));
        assert!(scrolls_to_row(Some(&BansPicksStatus::PackFailed)));
        assert!(!scrolls_to_row(Some(&BansPicksStatus::PackMissing {
            linked: false
        })));
        assert!(!scrolls_to_row(Some(&BansPicksStatus::PackMissing {
            linked: true
        })));
        assert!(!scrolls_to_row(Some(&BansPicksStatus::NoFrames)));
        assert!(!scrolls_to_row(Some(&BansPicksStatus::ReaderFailed)));
        assert!(!scrolls_to_row(Some(&BansPicksStatus::FrameSize {
            width: 1920,
            height: 1080
        })));
        assert!(!scrolls_to_row(Some(&BansPicksStatus::Ok)));
        assert!(!scrolls_to_row(None));
    }

    #[test]
    fn a_fully_visible_row_does_not_scroll() {
        let viewport = rect(0.0, 80.0, 640.0, 780.0);
        let row = rect(20.0, 200.0, 600.0, 61.0);
        assert!(row_fully_visible(viewport, row));
        assert_eq!(reveal_offset(viewport, row, ROW_MARGIN, 0.0), None);
    }

    #[test]
    fn a_fully_visible_row_flush_with_the_pane_top_does_not_scroll() {
        let viewport = rect(0.0, 80.0, 640.0, 780.0);
        // The whole row is already in view, sitting at the top of a
        // tall pane: do not jump, even though there is no room above
        // it for ROW_MARGIN.
        let row = rect(20.0, 80.0, 600.0, 61.0);
        assert!(row_fully_visible(viewport, row));
        assert_eq!(reveal_offset(viewport, row, ROW_MARGIN, 0.0), None);
    }

    #[test]
    fn a_row_below_the_fold_jumps_with_margin_above_it() {
        let viewport = rect(0.0, 80.0, 640.0, 780.0);
        // Viewport covers y=80..860; the row starts at 1014, as on the
        // default 1280x860 window.
        let row = rect(20.0, 1014.0, 600.0, 61.0);
        assert!(!row_fully_visible(viewport, row));
        let y = reveal_offset(viewport, row, ROW_MARGIN, 0.0).expect("row is below the fold");
        assert_eq!(y, 1014.0 - 80.0 - ROW_MARGIN);
        // After the jump, the row's top (and its outline) sits
        // ROW_MARGIN below the pane top, so the bottom of the row
        // above shows.
        assert_eq!(row.y - y, viewport.y + ROW_MARGIN);
    }

    #[test]
    fn a_clipped_row_still_jumps() {
        let viewport = rect(0.0, 80.0, 640.0, 780.0);
        // Bottom 20 px sit past y=860.
        let row = rect(20.0, 820.0, 600.0, 61.0);
        assert!(!row_fully_visible(viewport, row));
        assert_eq!(
            reveal_offset(viewport, row, ROW_MARGIN, 0.0),
            Some(820.0 - 80.0 - ROW_MARGIN)
        );
    }

    #[test]
    fn a_shared_pane_offset_is_applied_to_the_visibility_check() {
        // A tall pane covers y=80..1400. The row at 1014 is in view at
        // offset 0, but after Games has left the shared pane scrolled
        // 1000 px the row is drawn at y=14, above the pane top.
        let viewport = rect(0.0, 80.0, 640.0, 1320.0);
        let row = rect(20.0, 1014.0, 600.0, 61.0);
        assert_eq!(reveal_offset(viewport, row, ROW_MARGIN, 0.0), None);
        let y = reveal_offset(viewport, row, ROW_MARGIN, 1000.0).expect("scrolled away, so jump");
        assert_eq!(y, 1014.0 - 80.0 - ROW_MARGIN);
    }

    #[test]
    fn the_offset_never_goes_above_the_top() {
        let viewport = rect(0.0, 80.0, 640.0, 780.0);
        // Almost at the top, but the last pixel is clipped, so we jump;
        // the raw y - content - margin would be negative.
        let clipped = rect(20.0, 50.0, 600.0, 61.0);
        assert!(!row_fully_visible(viewport, clipped));
        assert_eq!(reveal_offset(viewport, clipped, ROW_MARGIN, 0.0), Some(0.0));
    }

    #[test]
    fn the_mark_clears_on_timeout_click_and_scroll() {
        let t0 = Instant::now();
        let mut mark = RowMark::default();
        assert!(!mark.is_on(t0));
        mark.start(t0);
        assert!(mark.is_on(t0));
        assert!(mark.is_on(at(t0, 1_999)));
        assert!(!mark.is_on(at(t0, 2_000)), "off at two seconds, no fade");
        assert!(!mark.is_on(at(t0, 2_001)));

        mark.start(t0);
        mark.clear();
        assert!(!mark.is_on(t0), "clear is immediate");
        assert!(!mark.is_on(at(t0, 500)));

        mark.start(t0);
        assert!(is_click_or_scroll(&Event::Mouse(
            mouse::Event::ButtonPressed(mouse::Button::Left)
        )));
        assert!(is_click_or_scroll(&Event::Mouse(
            mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            }
        )));
        assert!(is_click_or_scroll(&Event::Touch(
            touch::Event::FingerPressed {
                id: touch::Finger(0),
                position: iced::Point::ORIGIN,
            }
        )));
        assert!(!is_click_or_scroll(&Event::Mouse(
            mouse::Event::CursorMoved {
                position: iced::Point::ORIGIN,
            }
        )));
        mark.clear();
        assert!(!mark.is_on(t0));
    }

    fn lin(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(c: Color) -> f32 {
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (hi, lo) = {
            let (la, lb) = (luminance(a), luminance(b));
            if la > lb { (la, lb) } else { (lb, la) }
        };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn the_accent_outline_meets_three_to_one_on_the_surface() {
        // The row sits on SURFACE (the Capture and reading card). ACCENT
        // at #8f73ff against SURFACE at #1c1c25 is about 4.88:1.
        let ratio = contrast(theme::ACCENT, theme::SURFACE);
        assert!(ratio >= 3.0, "accent on surface is {ratio:.3}:1, need 3:1");
        let on_inner = contrast(theme::ACCENT, theme::SURFACE_2);
        assert!(
            on_inner >= 3.0,
            "accent on surface 2 is {on_inner:.3}:1, need 3:1"
        );
    }

    #[test]
    fn the_copy_has_no_long_dashes() {
        for text in [
            ROW_ID,
            crate::settings_cards::BANS_PICKS_LABEL,
            crate::settings_cards::BANS_PICKS_HELP,
        ] {
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text:?} has a long dash"
            );
        }
    }
}
