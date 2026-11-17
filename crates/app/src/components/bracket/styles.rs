//! Styles for the bracket, its slots and the standings tables.
//!
//! The bracket follows the `d1-tournament-bracket` frames: round labels in one
//! header row, slots below, all inside a scroller that owns the sideways
//! overflow so the page never scrolls sideways. A `--bg` fade sits over the
//! right edge of the scroller and the next column peeks under it.

pub const BRACKET_STYLES: &str = r#"
/* --- Hint line above a scroller ---
   Swiping is a narrow-screen gesture: wider screens show the whole bracket
   or the fade, so the swipe hint is hidden there. */
.bracket-hint {
  display: none;
  align-items: center;
  gap: var(--space-2);
  margin: 0 0 var(--space-3);
  min-width: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
}
/* The round robin line also carries the column legend, which every width
   needs: it shows everywhere and only its swipe part is narrow only. */
.bracket-hint.is-legend {
  display: flex;
  flex-wrap: wrap;
}
.bracket-swipe {
  display: none;
  align-items: center;
  gap: var(--space-2);
}
@media (max-width: 820px) {
  .bracket-hint { display: flex; }
  .bracket-swipe { display: inline-flex; }
}
.swipe-arrow {
  flex: 0 0 var(--space-3);
  box-sizing: border-box;
  width: var(--space-3);
  height: var(--space-3);
  border-block-end: var(--focus-ring-width) solid currentColor;
  border-inline-end: var(--focus-ring-width) solid currentColor;
  transform: rotate(-45deg);
}

/* --- Scroller --- */
.bracket-clip {
  position: relative;
  width: 100%;
  max-width: 100%;
  min-width: 0;
}
.bracket-scroller {
  position: relative;
  box-sizing: border-box;
  width: 100%;
  max-width: 100%;
  min-width: 0;
  overflow-x: auto;
  overflow-y: hidden;
  padding-bottom: var(--space-3);
  -webkit-overflow-scrolling: touch;
}
.bracket-scroller:focus-visible,
.bracket-table-scroll:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
/* Over the clipped column at the right edge. The bracket ends with the same
   width of padding, so the last column can scroll clear of it. */
.bracket-fade {
  pointer-events: none;
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: var(--space-8);
  background: linear-gradient(to left, var(--bg) 0%, var(--bg) var(--space-2), transparent 100%);
  z-index: 1;
}

/* --- Bracket grid: one header row, one row of rounds --- */
.bracket {
  box-sizing: content-box;
  display: grid;
  grid-template-columns: repeat(var(--bracket-cols), calc(var(--space-8) * 4));
  grid-template-rows: auto 1fr;
  column-gap: var(--space-5);
  row-gap: var(--space-3);
  align-items: stretch;
  width: max-content;
  padding-inline-end: var(--space-8);
}
.bracket-head {
  grid-row: 1;
  margin: 0;
  min-width: 0;
  min-height: var(--space-6);
  display: flex;
  align-items: center;
  font-family: var(--font-head);
  font-size: var(--text-sm);
  font-weight: var(--font-label-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  color: var(--text-3);
}
/* Every round is as tall as the tallest, and its slots share that height
   evenly, so a later round's slot sits centred between its two feeders. */
.bracket-round {
  grid-row: 2;
  display: flex;
  flex-direction: column;
  justify-content: space-around;
  gap: var(--space-4);
  min-width: 0;
}

/* --- Slot: team, result line, team --- */
.bracket-slot {
  box-sizing: border-box;
  min-width: 0;
  padding: var(--space-3);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-ui);
}
.bracket-slot.is-live { border-color: var(--border-strong); }
.bracket-slot p {
  margin: 0;
  overflow-wrap: anywhere;
}
.bracket-slot p + p { margin-top: var(--space-1); }
/* The winner is the heavier name, the mark points at it and the result line
   says "Won": never colour alone. An open slot ("To be decided") keeps the
   team ink, as the frames draw the champion slot; only the result line under
   it is muted. */
.slot-team.is-winner { font-weight: var(--font-label-weight); }
.bracket-slot .slot-meta {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--text-3);
  font-size: var(--text-sm);
}
.bracket-slot.is-live .slot-meta {
  color: var(--text);
  font-weight: var(--font-label-weight);
}
.slot-mark {
  flex: 0 0 auto;
  width: 0;
  height: 0;
  border-inline: var(--space-1) solid transparent;
}
.slot-mark.is-up { border-block-end: var(--space-2) solid currentColor; }
.slot-mark.is-down { border-block-start: var(--space-2) solid currentColor; }
.bracket-slot .slot-codes {
  margin-top: var(--space-2);
  color: var(--text-3);
  font-size: var(--text-xs);
}
.slot-code {
  font-family: var(--font-mono);
  color: var(--text);
}

/* --- Notes and section text --- */
.bracket-note {
  margin: 0;
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
}

/* --- Standings and results tables --- */
/* Positioned so the visually hidden header names stay inside the scroller:
   without it they hang off the page and widen it on a phone. */
.bracket-table-scroll {
  position: relative;
  box-sizing: border-box;
  width: 100%;
  max-width: 100%;
  min-width: 0;
  overflow-x: auto;
  -webkit-overflow-scrolling: touch;
  background: var(--table-row-bg);
  border: 1px solid var(--table-row-border);
  border-radius: var(--table-radius);
}
.bracket-table {
  width: 100%;
  border-collapse: collapse;
  font-family: var(--table-font);
  font-size: var(--text-sm);
  line-height: var(--leading-ui);
  color: var(--table-row-fg);
  font-variant-numeric: tabular-nums;
}
.bracket-table th,
.bracket-table td {
  box-sizing: border-box;
  height: var(--table-row-height);
  padding: var(--space-2) var(--table-pad-x);
  text-align: left;
  vertical-align: middle;
  white-space: nowrap;
  border-bottom: 1px solid var(--table-row-border);
}
.bracket-table tbody tr:last-child th,
.bracket-table tbody tr:last-child td { border-bottom: 0; }
.bracket-table thead th {
  color: var(--table-header-fg);
  font-weight: var(--table-header-weight);
  font-size: var(--text-xs);
}
.bracket-table tbody th {
  font-weight: var(--font-label-weight);
  color: var(--table-row-fg);
}
.bracket-table .is-num { text-align: right; }
.bracket-table .is-muted { color: var(--table-row-muted); }
.bracket-table .rr-self { background: var(--table-row-alt); }
.rr-word {
  display: block;
  font-weight: var(--font-label-weight);
}
.rr-word.is-win { color: var(--win); }
.rr-word.is-loss { color: var(--loss); }
.rr-detail {
  display: block;
  color: var(--table-row-muted);
  font-size: var(--text-xs);
}
"#;

#[cfg(test)]
mod tests {
    use super::BRACKET_STYLES;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl, media_block,
    };

    #[test]
    fn sizes_and_type_come_from_tokens() {
        assert_sizes_from_tokens(BRACKET_STYLES);
    }

    #[test]
    fn scroller_owns_the_sideways_overflow() {
        let scroller = css_rule(BRACKET_STYLES, ".bracket-scroller {");
        for decl in ["overflow-x: auto", "min-width: 0", "max-width: 100%"] {
            assert!(has_decl(&scroller, decl), "{decl}: {scroller}");
        }
        let clip = css_rule(BRACKET_STYLES, ".bracket-clip {");
        assert!(has_decl(&clip, "min-width: 0"), "{clip}");
        assert!(has_decl(&clip, "position: relative"), "{clip}");
        let table = css_rule(BRACKET_STYLES, ".bracket-table-scroll {");
        assert!(has_decl(&table, "overflow-x: auto"), "{table}");
        assert!(has_decl(&table, "min-width: 0"), "{table}");
        // Hidden header names are absolutely placed: the scroller must be
        // their containing block or they widen the page.
        assert!(has_decl(&table, "position: relative"), "{table}");
    }

    #[test]
    fn fade_is_a_bg_gradient_one_space_8_wide() {
        let fade = css_rule(BRACKET_STYLES, ".bracket-fade {");
        assert!(has_decl(&fade, "width: var(--space-8)"), "{fade}");
        assert!(has_decl(&fade, "pointer-events: none"), "{fade}");
        assert!(
            fade.contains("linear-gradient(to left, var(--bg)"),
            "{fade}"
        );
        // The bracket leaves room so its last column scrolls clear of the fade.
        let bracket = css_rule(BRACKET_STYLES, ".bracket {");
        assert!(
            has_decl(&bracket, "padding-inline-end: var(--space-8)"),
            "{bracket}"
        );
    }

    #[test]
    fn hint_keeps_the_frame_gap_above_the_scroller() {
        // Browser pass: the frame puts `--space-3` under the hint on top of
        // the page gap; the app had none, so the round labels sat 12px high.
        let hint = css_rule(BRACKET_STYLES, ".bracket-hint {");
        assert!(has_decl(&hint, "margin: 0 0 var(--space-3)"), "{hint}");
    }

    #[test]
    fn open_slot_is_not_muted_like_the_result_line() {
        // The frames draw "To be decided" in the champion slot in the team
        // ink; only the line under it is muted.
        assert!(!BRACKET_STYLES.contains(".slot-team.is-open"));
        let meta = css_rule(BRACKET_STYLES, ".bracket-slot .slot-meta {");
        assert!(has_decl(&meta, "color: var(--text-3)"), "{meta}");
    }

    #[test]
    fn heads_share_one_row_and_rounds_the_next() {
        assert!(has_decl(
            &css_rule(BRACKET_STYLES, ".bracket-head {"),
            "grid-row: 1"
        ));
        assert!(has_decl(
            &css_rule(BRACKET_STYLES, ".bracket-round {"),
            "grid-row: 2"
        ));
    }

    #[test]
    fn scrollers_show_the_token_focus_ring() {
        // Both scrollers share the one rule: the bracket and the tables.
        let focus = css_rule(
            BRACKET_STYLES,
            ".bracket-scroller:focus-visible,\n.bracket-table-scroll:focus-visible {",
        );
        assert!(
            has_decl(
                &focus,
                "outline: var(--focus-ring-width) solid var(--focus-ring)"
            ),
            "{focus}"
        );
    }

    #[test]
    fn later_rounds_centre_between_their_feeders() {
        let grid = css_rule(BRACKET_STYLES, ".bracket {");
        assert!(has_decl(&grid, "align-items: stretch"), "{grid}");
        let round = css_rule(BRACKET_STYLES, ".bracket-round {");
        assert!(has_decl(&round, "justify-content: space-around"), "{round}");
    }

    #[test]
    fn the_swipe_hint_shows_on_narrow_screens_only() {
        let hint = css_rule(BRACKET_STYLES, ".bracket-hint {");
        assert!(has_decl(&hint, "display: none"), "{hint}");
        let narrow = media_block(BRACKET_STYLES, "@media (max-width: 820px)");
        assert!(
            narrow.contains(".bracket-hint { display: flex; }"),
            "{narrow}"
        );
    }

    #[test]
    fn the_round_robin_legend_shows_at_every_width() {
        // Vision S4: the legend lived in the narrow-only hint, so above
        // 820px the numbered columns had no key. The legend line shows at
        // every width (outside any media block); only its swipe part waits
        // for the narrow block.
        let legend = css_rule(BRACKET_STYLES, ".bracket-hint.is-legend {");
        assert!(has_decl(&legend, "display: flex"), "{legend}");
        let swipe = css_rule(BRACKET_STYLES, ".bracket-swipe {");
        assert!(has_decl(&swipe, "display: none"), "{swipe}");
        let narrow = media_block(BRACKET_STYLES, "@media (max-width: 820px)");
        assert!(
            narrow.contains(".bracket-swipe { display: inline-flex; }"),
            "{narrow}"
        );
        // Top level, so no media query gates it, and it comes after the
        // plain hint rule so it wins over `display: none`.
        let top = BRACKET_STYLES.find(".bracket-hint.is-legend {").unwrap();
        let base = BRACKET_STYLES.find(".bracket-hint {").unwrap();
        assert!(top > base);
        let before = &BRACKET_STYLES[..top];
        assert_eq!(before.matches('{').count(), before.matches('}').count());
        assert!(!narrow.contains("is-legend"), "{narrow}");
    }

    #[test]
    fn result_words_clear_text_contrast_on_the_table_fill() {
        // `--table-row-bg` is `--surface`, `--table-row-alt` is not used
        // under a result word.
        assert_ink_clears_aa("--win", "--surface", 4.5);
        assert_ink_clears_aa("--loss", "--surface", 4.5);
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
    }
}
