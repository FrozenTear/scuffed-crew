//! The Map cell never cuts a chip without an ellipsis: every catalogue map
//! (`stat_tracker::parse::known_map_names`), with its mode chip, with the
//! mode and a queue chip, and with the unsure sign, in a whole game row
//! laid out with the tiny-skia renderer and the bundled fonts at real
//! window widths. Each drawn text is read back from its own widget state
//! (the laid-out paragraph), so a cut shows whatever the row drew: text
//! wider than its box, or past the Map cell's right edge.

use super::*;
use crate::games_detail::{MAP_CELL_GAP, map_cell_parts, map_chips};
use crate::games_map_fit::{CHIP_PAD_X, ELLIPSIS, MIN_KEPT, stub};
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::layout::{Limits, Node};
use iced::advanced::widget::Tree;
use iced::advanced::widget::tree::Tag;

/// The default window, the grid switch points and their edges, 1280 and
/// 1920.
const WINDOWS: [f32; 8] = [
    720.0, 1000.0, 1099.0, 1100.0, 1280.0, 1439.0, 1440.0, 1920.0,
];

/// A queue the chip names, and the widest one (`games_queue::ALL`).
fn queues() -> [&'static str; 2] {
    [
        crate::games_queue::ROLE_6V6,
        crate::games_queue::tests::widest(),
    ]
}

/// What the cell draws for one case.
#[derive(Default)]
struct Drawn {
    /// Every text: content, its paragraph's width, its box's width, and
    /// its left and right edge in the cell.
    texts: Vec<(String, f32, f32, f32, f32)>,
    /// Text whose paragraph is taller than its box.
    too_tall: Vec<String>,
    tooltips: usize,
}

fn text_tag() -> Tag {
    Tag::of::<iced::advanced::widget::text::State<Paragraph>>()
}

fn tooltip_tag() -> Tag {
    let tip: Element<'_, Message> =
        iced::widget::tooltip(text(""), text(""), iced::widget::tooltip::Position::Top).into();
    Tree::new(&tip).tag
}

/// Walk the laid-out tree and nodes together from `x` (the parent's left
/// edge in the cell). A tooltip lays out as its content. A container has
/// no tree of its own (it shares its content's) but has a node, so a node
/// with one child where the tree does not match steps down a node and
/// keeps the tree.
fn walk(tree: &Tree, node: &Node, x: f32, tags: &(Tag, Tag), out: &mut Drawn) {
    if tree.tag == tags.1 {
        out.tooltips += 1;
        return walk(&tree.children[0], node, x, tags, out);
    }
    let b = node.bounds();
    let left = x + b.x;
    let text_leaf = tree.tag == tags.0 && node.children().is_empty();
    if text_leaf {
        let p = tree
            .state
            .downcast_ref::<iced::advanced::widget::text::State<Paragraph>>();
        let min = p.min_bounds();
        if min.height > b.height + 0.5 {
            out.too_tall.push(p.content().to_string());
        }
        out.texts.push((
            p.content().to_string(),
            min.width,
            b.width,
            left,
            left + b.width,
        ));
        return;
    }
    if tree.tag != tags.0 && tree.children.len() == node.children().len() {
        for (t, n) in tree.children.iter().zip(node.children()) {
            walk(t, n, left, tags, out);
        }
        return;
    }
    match node.children() {
        [only] => walk(tree, only, left, tags, out),
        _ => panic!(
            "{b:?}: {} node children, {} tree children",
            node.children().len(),
            tree.children.len()
        ),
    }
}

/// The first node with more than one child: the row. The mouse area lays
/// out as its content, so the tree has one more level and is walked down
/// on its own ([`row_tree`]).
fn row_node(node: &Node) -> &Node {
    match node.children() {
        [only] => row_node(only),
        [] => panic!("no row under {:?}", node.bounds()),
        _ => node,
    }
}

/// The first tree with more than one child: the row's.
fn row_tree(tree: &Tree) -> &Tree {
    match tree.children.as_slice() {
        [only] => row_tree(only),
        [] => panic!("no row tree"),
        _ => tree,
    }
}

/// The one-line width of a chip with `label`, from the text widget.
fn full_chip_width(renderer: &iced::Renderer, label: &str) -> f32 {
    let mut t: Element<'_, Message> = text(label.to_string())
        .size(SIZE_LABEL)
        .font(FONT_SEMIBOLD)
        .wrapping(Wrapping::None)
        .into();
    let mut tree = Tree::new(&t);
    let limits = Limits::new(iced::Size::ZERO, iced::Size::new(10_000.0, 100.0));
    t.as_widget_mut()
        .layout(&mut tree, renderer, &limits)
        .bounds()
        .width
        + CHIP_PAD_X
}

/// Lay out `g`'s row at `window` and read its Map cell.
fn draw(renderer: &iced::Renderer, g: &Game, window: f32) -> (f32, f32, Drawn) {
    let grid = Grid::for_window(window);
    let width = window - 2.0 * crate::theme::page_gutter(window);
    let cols = games_rows::columns(grid);
    let map_at = cols.iter().position(|(c, _)| *c == Col::Map).unwrap();
    let mut row = game_row(g, &cols, TabState::default(), false, false, false);
    let mut tree = Tree::new(&row);
    let limits = Limits::new(iced::Size::ZERO, iced::Size::new(width, 400.0));
    let node = row.as_widget_mut().layout(&mut tree, renderer, &limits);
    let (cells_tree, cells) = (row_tree(&tree), row_node(&node));
    assert_eq!(cells.children().len(), cols.len() + 1);
    assert_eq!(cells_tree.children.len(), cols.len() + 1);
    let cell = &cells.children()[map_at];
    let mut out = Drawn::default();
    let tags = (text_tag(), tooltip_tag());
    walk(
        &cells_tree.children[map_at],
        cell,
        -cell.bounds().x,
        &tags,
        &mut out,
    );
    (cell.bounds().width, cell.bounds().height, out)
}

/// Per window: cases, chips shortened, chips left out, cells with a
/// tooltip, and the silent cuts found (with the mode chip alone, the
/// case QA counted).
#[derive(Default, Debug)]
struct Counts {
    cases: usize,
    shortened: usize,
    left_out: Vec<String>,
    tooltips: usize,
    cut: Vec<String>,
    mode_only_cut: Vec<String>,
    bare: Vec<String>,
    short_names: Vec<String>,
}

/// A game with map `m`, the map read unsure (`doubt`) or not, and a
/// queue chip with these words or none.
fn case_game(m: &str, doubt: bool, queue: Option<&str>) -> Game {
    let suspect: &[&str] = if doubt { &["map"] } else { &[] };
    let mut g = crate::games_rows::tests::game(Outcome::Win, "2026-09-02T21:14:00Z", suspect);
    g.map_name = m.into();
    if let Some(q) = queue {
        crate::games_queue::tests::set_label(&mut g, q);
    }
    g
}

/// Lay one case out at `window` and check it, counting into `n` and
/// adding what is wrong to `problems` (so a run reports every case).
fn check_case(
    renderer: &iced::Renderer,
    (m, doubt, queue): (&str, bool, Option<&str>),
    window: f32,
    n: &mut Counts,
    problems: &mut Vec<String>,
) {
    let g = case_game(m, doubt, queue);
    let parts = map_cell_parts(&g, doubt);
    assert_eq!(parts.contains(&MapPart::Warn), doubt, "{m}");
    let full = map_chips(&g);
    let mode = games_rows::mode_chip(m);
    let at = format!("{m} doubt {doubt} queue {queue:?} window {window}");
    let (cell_w, cell_h, drawn) = draw(renderer, &g, window);
    n.cases += 1;
    // Nothing is cut: no text is wider than its box or ends past the cell.
    for (s, min_w, box_w, _, right) in &drawn.texts {
        if *min_w > box_w + 0.01 || *right > cell_w + 0.01 {
            n.cut.push(format!(
                "{at}: {s:?} needs {min_w}, box {box_w}, ends {right}"
            ));
            let tracked = !matches!(mode, Some("Practice" | "Deathmatch"));
            if !doubt && queue.is_none() && Some(s.as_str()) == mode && tracked {
                n.mode_only_cut.push(m.to_string());
            }
        }
    }
    if !drawn.too_tall.is_empty() {
        problems.push(format!("{at}: taller than its box {:?}", drawn.too_tall));
    }
    // The name (in full, or shortened with "…" when even the mode chip's
    // stub would not fit beside it wrapped), then one text a chip.
    let name = &drawn.texts[0].0;
    let name_short = name != m;
    if name_short {
        let kept = name.strip_suffix(ELLIPSIS).unwrap_or_default();
        assert!(m.starts_with(kept), "{at}: name {name:?}");
        // Never a bare "…": the name keeps its first letters too.
        if kept.trim().chars().count() < MIN_KEPT {
            n.bare.push(format!("{at}: name {name:?}"));
        }
        n.short_names.push(format!("{at}: {name:?}"));
    }
    let chips = &drawn.texts[1..];
    let mut changed = Vec::new();
    for (i, (s, _, _, left, _)) in chips.iter().enumerate() {
        let want = &full[i];
        if s == want {
            continue;
        }
        changed.push(want.clone());
        n.shortened += 1;
        let Some(kept) = s.strip_suffix(ELLIPSIS) else {
            problems.push(format!("{at}: {s:?} is neither {want:?} nor shortened"));
            continue;
        };
        // Never a bare "…": a shortened chip keeps its first letters.
        if kept.trim().chars().count() < MIN_KEPT.min(want.chars().count() - 1) {
            n.bare.push(format!("{at}: {s:?} for {want:?}"));
        }
        if !want.starts_with(kept) {
            problems.push(format!("{at}: {s:?} is not a start of {want:?}"));
        }
        // The full chip would not have fit with a stub chip for each chip
        // after it (or it is the last one shown before one left out).
        let after: f32 = full[i + 1..chips.len()]
            .iter()
            .map(|c| MAP_CELL_GAP + full_chip_width(renderer, &stub(c)))
            .sum();
        let full_right = left - CHIP_PAD_X / 2.0 + full_chip_width(renderer, want) + after;
        let before_left_out = chips.len() < full.len() && i + 1 == chips.len();
        if full_right <= cell_w - 0.01 && !before_left_out {
            problems.push(format!(
                "{at}: {want:?} fits ({full_right} in {cell_w}) but shows {s:?}"
            ));
        }
    }
    // A chip with no room even for its stub is left out whole, only after
    // a chip that ends in "…", so the row still shows that more is there.
    let left_out = &full[chips.len()..];
    if !left_out.is_empty() {
        n.left_out.push(format!("{at}: {left_out:?}"));
        if !chips.last().is_some_and(|c| c.0.ends_with(ELLIPSIS)) {
            problems.push(format!(
                "{at}: {left_out:?} left out with no chip ending in …"
            ));
        }
    }
    changed.extend(left_out.iter().cloned());
    // The tooltip: map and mode in full, and every chip not shown in full;
    // none when the name and every chip show in full.
    let want_tip = (!changed.is_empty() || name_short).then(|| {
        let mut words = vec![m.to_string()];
        words.extend(mode.map(str::to_string));
        words.extend(changed.iter().filter(|c| Some(c.as_str()) != mode).cloned());
        words.join(", ")
    });
    let fit = crate::games_map_fit::fit(cell_w, cell_h, m, FONT_BOLD, &parts, mode);
    if fit.tip != want_tip {
        problems.push(format!("{at}: tooltip {:?}, want {want_tip:?}", fit.tip));
    }
    // The Map cell has the bans and picks tooltip pattern only when
    // something is not shown in full.
    if drawn.tooltips != usize::from(want_tip.is_some()) {
        problems.push(format!(
            "{at}: {} tooltips, want {want_tip:?}",
            drawn.tooltips
        ));
    }
    n.tooltips += drawn.tooltips;
}

/// The cases for each map: its mode chip, mode and a queue chip (short
/// and long), each with and without the unsure sign.
fn cases(m: &str) -> Vec<(&str, bool, Option<&str>)> {
    let mut out = Vec::new();
    for doubt in [false, true] {
        let [one, wide] = queues();
        for queue in [None, Some(one), Some(wide)] {
            out.push((m, doubt, queue));
        }
    }
    out
}

#[test]
fn map_chips_never_clip_silently() {
    let renderer = crate::games_bans_view::tests::headless();
    let maps = stat_tracker::parse::known_map_names();
    // Every catalogue map, a wider set than the Games list draws: only the
    // maps `stat_tracker::parse::stats_row_is_tracked` keeps (33 today)
    // reach the list. Wuxing University - Water College (Stadium) and
    // Workshop Green Screen (Workshop), the longest names, are not among
    // them. The name is shortened before any chip is left out, so at the
    // windows below no chip is left out, and names are shortened only at
    // 1100 (pinned below).
    assert!(maps.len() >= 54, "{}", maps.len());
    let mut problems: Vec<String> = Vec::new();
    let mut report = Vec::new();
    // Per window: chips left out, names shortened (of 372 cases each).
    let mut pinned = Vec::new();
    for window in WINDOWS {
        let mut n = Counts::default();
        for &m in &maps {
            for case in cases(m) {
                check_case(&renderer, case, window, &mut n, &mut problems);
            }
        }
        report.push(format!(
            "window {window}: {} cases, {} chips shortened with an ellipsis, {} left out, \
             {} cells with a tooltip, {} silent cuts ({} tracked maps cut with the mode chip \
             alone: {:?})\n  left out: {:?}\n  names shortened: {:?}",
            n.cases,
            n.shortened,
            n.left_out.len(),
            n.tooltips,
            n.cut.len(),
            n.mode_only_cut.len(),
            n.mode_only_cut,
            n.left_out,
            n.short_names,
        ));
        pinned.push((window, n.left_out.len(), n.short_names.len()));
        problems.extend(n.cut);
        problems.extend(n.bare);
    }
    // Chief's rule (24 Nov): no chip is left out where shortening the name
    // to two letters and "…" makes room. At 1100 that shortens 73 names
    // (with the sign or a queue chip) and leaves nothing out. It was 74
    // while the chip showed the raw title ("COMPETITIVE: 6V6 OPEN QUEUE");
    // the widest queue chip is now "Role queue, size unknown" (#654).
    let want: Vec<(f32, usize, usize)> = WINDOWS
        .iter()
        .map(|&w| (w, 0, if w == 1100.0 { 73 } else { 0 }))
        .collect();
    assert_eq!(pinned, want, "(window, chips left out, names shortened)");
    let listed: Vec<&str> = maps
        .iter()
        .copied()
        .filter(|m| {
            stat_tracker::parse::stats_row_is_tracked(
                m,
                stat_tracker::parse::map_mode(m).unwrap_or(""),
            )
        })
        .collect();
    for m in ["Wuxing University - Water College", "Workshop Green Screen"] {
        assert!(!listed.contains(&m), "{m} reaches the Games list");
    }
    eprintln!(
        "{} catalogue maps, {} reach the Games list\n{}",
        maps.len(),
        listed.len(),
        report.join("\n")
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every window from 720 to 1920 px in 1 px steps for the maps whose name
/// and mode chip are widest on one line, so no window between the named
/// ones cuts a chip either.
#[test]
fn map_chips_never_clip_silently_at_any_window() {
    let renderer = crate::games_bans_view::tests::headless();
    let mut maps: Vec<(f32, &str)> = stat_tracker::parse::known_map_names()
        .into_iter()
        .map(|m| {
            let (_, _, drawn) = draw(&renderer, &case_game(m, false, None), 10_000.0);
            (drawn.texts.iter().map(|t| t.1).sum::<f32>(), m)
        })
        .collect();
    maps.sort_by(|a, b| b.0.total_cmp(&a.0));
    let widest: Vec<&str> = maps.iter().take(6).map(|m| m.1).collect();
    let mut problems = Vec::new();
    let mut n = Counts::default();
    for window in 720..=1920 {
        for &m in &widest {
            for (m, doubt, queue) in cases(m) {
                if queue != Some(queues()[1]) {
                    check_case(
                        &renderer,
                        (m, doubt, queue),
                        window as f32,
                        &mut n,
                        &mut problems,
                    );
                }
            }
        }
    }
    eprintln!(
        "maps {widest:?}: {} cases, {} chips shortened, {} left out, {} tooltips, {} silent cuts",
        n.cases,
        n.shortened,
        n.left_out.len(),
        n.tooltips,
        n.cut.len()
    );
    problems.extend(n.cut);
    problems.extend(n.bare);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// A name shortened with no chip cut still gets the tooltip: a one-word
/// name too wide for the cell beside a one-letter chip, whose stub ("X…")
/// is wider than the chip, so the chip shows in full.
#[test]
fn a_shortened_name_alone_gets_the_tooltip() {
    let _renderer = crate::games_bans_view::tests::headless();
    let name = "W".repeat(40);
    let parts = [MapPart::Name, MapPart::Chip("X".into())];
    let fit = crate::games_map_fit::fit(120.0, 40.0, &name, FONT_BOLD, &parts, Some("X"));
    let short = fit.name.clone().expect("the name is shortened");
    let kept = short.strip_suffix(ELLIPSIS).expect("ends in …");
    assert!(
        kept.chars().count() >= MIN_KEPT && name.starts_with(kept),
        "{short:?}"
    );
    assert_eq!(fit.parts, parts, "no chip cut or left out");
    assert_eq!(fit.tip, Some(format!("{name}, X")));
}

/// A chip that fits shows its full text and the cell has no tooltip:
/// every catalogue map with its mode chip at 1920.
#[test]
fn a_chip_that_fits_is_unchanged() {
    let renderer = crate::games_bans_view::tests::headless();
    for m in stat_tracker::parse::known_map_names() {
        let mut g = crate::games_rows::tests::game(Outcome::Win, "2026-09-02T21:14:00Z", &[]);
        g.map_name = m.into();
        let (_, _, drawn) = draw(&renderer, &g, 1920.0);
        let shown: Vec<&str> = drawn.texts.iter().map(|t| t.0.as_str()).collect();
        let mut want = vec![m];
        want.extend(games_rows::mode_chip(m));
        assert_eq!(shown, want, "{m}");
        assert_eq!(drawn.tooltips, 0, "{m}");
    }
}

/// The unsure-sign row's contract (`the_unsure_map_sign_keeps_its_width_when_laid_out`):
/// the name, the sign and the mode chip always show, the mode chip whole
/// or ending in "…". The queue chip shows, or is left out and the tooltip
/// names the queue title.
pub(super) fn shows_mode_and_names_queue(
    parts: &[MapPart],
    fit: &crate::games_map_fit::MapFit,
    at: &str,
) {
    let shown = &fit.parts;
    let [MapPart::Chip(full_mode), MapPart::Chip(queue)] = &parts[2..] else {
        panic!("{at}: mode and queue chips after the name and sign: {parts:?}");
    };
    assert!(
        matches!(&shown[..2], [MapPart::Name, MapPart::Warn]),
        "{at}: {shown:?}"
    );
    let Some(MapPart::Chip(shown_mode)) = shown.get(2) else {
        panic!("{at}: no mode chip in {shown:?}");
    };
    let kept = shown_mode.strip_suffix(ELLIPSIS);
    assert!(
        shown_mode == full_mode || kept.is_some_and(|k| full_mode.starts_with(k.trim_end())),
        "{at}: mode chip {shown_mode:?} for {full_mode:?}"
    );
    let tip_names_queue = fit
        .tip
        .as_deref()
        .is_some_and(|t| t.contains(queue.as_str()));
    let queue_ok = match shown.len() {
        4 => matches!(&shown[3], MapPart::Chip(_)),
        n => n == 3 && tip_names_queue,
    };
    assert!(queue_ok, "{at}: {shown:?}, tip {:?}", fit.tip);
}
