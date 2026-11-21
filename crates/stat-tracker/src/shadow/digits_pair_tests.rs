//! Pair check tests for [`super`] (`shadow::digits`): template glyphs
//! only, no captured pixels.
use super::test_glyphs::{glyph_cell, whole_piece};
use super::*;

#[test]
fn pair_check_reads_the_region_where_look_alikes_differ() {
    let tpl = templates_for(REF_H_1440);
    let canvas = |c: usize| -> [f32; CANVAS] { tpl.t[c].as_slice().try_into().unwrap() };
    for (a, b) in [(4, 6), (0, 3)] {
        assert!(pair_check(&canvas(a), tpl, a, b) > 0.9, "{a} vs {b}");
        assert!(pair_check(&canvas(b), tpl, a, b) < -0.9, "{b} read as {a}");
        // antisymmetric: swapping the pair flips the sign
        let (x, y) = (
            pair_check(&canvas(a), tpl, a, b),
            pair_check(&canvas(a), tpl, b, a),
        );
        assert!((x + y).abs() < 1e-9);
    }
    // 3 vs 8 differ only on the left side, so the scores are weaker, but
    // a 3 still favours the 3 and an 8 the 8
    let (three, eight) = (
        pair_check(&canvas(3), tpl, 3, 8),
        pair_check(&canvas(8), tpl, 3, 8),
    );
    assert!(
        three > PAIR_FLOOR * PAIR_TYP_3V8 && eight < 0.0,
        "{three} {eight}"
    );
    assert!((three + pair_check(&canvas(3), tpl, 8, 3)).abs() < 1e-9);
}

#[test]
fn pair_margin_applies_only_to_known_look_alikes() {
    let tpl = templates_for(REF_H_1440);
    let four = glyph_cell(4, false);
    let p = whole_piece(&four);
    let m = pair_margin(&four, tpl, p, 4, 6, false).unwrap();
    assert!(m > 0.9, "{m}");
    // a 4 whose shape runner-up is 1 but which is far too wide to be a 1
    assert!(four.ar[p] > tpl.ar_hi[1]);
    assert!(pair_margin(&four, tpl, p, 4, 1, false).is_some());
    // other pairs get no check
    assert!(pair_margin(&four, tpl, p, 4, 9, false).is_none());
    assert!(pair_margin(&four, tpl, p, 6, 4, false).is_none());
    // a 0 is only helped when the cell is dim
    let zero = glyph_cell(0, false);
    let z = whole_piece(&zero);
    assert!(pair_margin(&zero, tpl, z, 0, 3, true).unwrap() > 0.9);
    assert!(pair_margin(&zero, tpl, z, 0, 3, false).is_none());
    assert!(pair_margin(&zero, tpl, z, 3, 0, true).is_none());
    // a 3 next to an 8 is only helped when the cell is bright
    let three = glyph_cell(3, false);
    let t = whole_piece(&three);
    assert!(pair_margin(&three, tpl, t, 3, 8, false).unwrap() > 0.9);
    assert!(pair_margin(&three, tpl, t, 3, 8, true).is_none());
    assert!(pair_margin(&three, tpl, t, 8, 3, false).is_none());
    // a real 8 scores against the 3, so the check cannot lift it
    let eight = glyph_cell(8, false);
    let e = whole_piece(&eight);
    assert!(pair_margin(&eight, tpl, e, 3, 8, false).unwrap() < 0.0);
}
