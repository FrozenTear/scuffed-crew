//! The whole Games row still fits with the Rank column at
//! [`games_rows::RANK_W`]: laid out with the tiny-skia renderer at real
//! window widths, every fixed column keeps its full width, the row ends
//! inside the table, and the Rank and Hero cells hold their widest
//! content (the Map cell's own fit is
//! `the_unsure_map_sign_keeps_its_width_when_laid_out`).

use super::*;
use iced::advanced::layout::{Limits, Node};
use iced::advanced::widget::Tree;
use stat_tracker::model::{RankNumber, RankText, RankUpdate};

/// The row under the mouse area and its padding container.
fn row_node(node: &Node) -> &Node {
    match node.children() {
        [only] => row_node(only),
        [] => panic!("no row under {:?}", node.bounds()),
        _ => node,
    }
}

fn lay_out(renderer: &iced::Renderer, mut element: Element<'_, Message>, width: f32) -> Node {
    let mut tree = Tree::new(&element);
    let limits = Limits::new(iced::Size::ZERO, iced::Size::new(width, 400.0));
    element.as_widget_mut().layout(&mut tree, renderer, &limits)
}

/// Width with no limit, so text that does not wrap comes out whole.
fn natural_width(renderer: &iced::Renderer, element: Element<'_, Message>) -> f32 {
    lay_out(renderer, element, 10_000.0).bounds().width
}

/// A game with the widest rank the cell draws: "Grandmaster 5" over
/// "-100% -100%".
fn widest_rank_game() -> Game {
    let mut g = crate::games_rows::tests::game(Outcome::Win, "2026-09-02T21:14:00Z", &[]);
    let word = |value: &str| {
        Some(RankText {
            value: value.into(),
            confidence: 0.9,
        })
    };
    let num = |value| {
        Some(RankNumber {
            value,
            confidence: 0.9,
        })
    };
    g.rank_update = Some(RankUpdate {
        role: word("Damage"),
        role_rank: word("GRANDMASTER"),
        division: num(5),
        progress_pct: num(-100),
        progress_change: num(-100),
        ..RankUpdate::default()
    });
    g.role = Role::Damage;
    g
}

/// The default window, the switch points and the edges of each grid
/// (1000x768 and 1280x860 are the two the PR checks by name).
const WINDOWS: [f32; 10] = [
    720.0, 1000.0, 1099.0, 1100.0, 1280.0, 1366.0, 1439.0, 1440.0, 1600.0, 1920.0,
];

#[test]
fn the_row_fits_with_the_wider_rank_column() {
    let renderer = crate::games_bans_view::tests::headless();
    let mut g = widest_rank_game();
    let rank_need = natural_width(&renderer, crate::games_rank::cell(&g));
    // The widest hero cell: every hero name, with the unsure sign.
    let (hero_need, hero) = scuffed_types::HERO_NAMES
        .iter()
        .map(|h| {
            g.hero = h.display.into();
            (natural_width(&renderer, hero_cell(&g, true)), h.display)
        })
        .fold((0.0_f32, ""), |a, b| if b.0 > a.0 { b } else { a });
    g.hero = hero.into();
    eprintln!("rank cell needs {rank_need}, widest hero cell {hero} needs {hero_need}");
    for window in WINDOWS {
        let grid = Grid::for_window(window);
        let width = window - 2.0 * crate::theme::page_gutter(window);
        let cols = games_rows::columns(grid);
        let row = game_row(&g, &cols, TabState::default(), false, false, false);
        let node = lay_out(&renderer, row, width);
        let row = row_node(&node);
        let cells = row.children();
        let at = format!("{grid:?} window {window} table {width}");
        assert_eq!(cells.len(), cols.len() + 1, "{at}");
        let fixed: f32 = cols
            .iter()
            .filter_map(|(_, w)| match w {
                Width::Px(px) => Some(*px),
                Width::Share(_) => None,
            })
            .sum();
        let mut shares = Vec::new();
        for (cell, (col, w)) in cells.iter().zip(&cols) {
            let got = cell.bounds().width;
            match w {
                Width::Px(px) => assert_eq!(got, *px, "{col:?} {at}"),
                Width::Share(_) => shares.push(format!("{col:?} {got:.0}")),
            }
            match col {
                Col::Rank => assert!(got >= rank_need, "Rank {got} < {rank_need} {at}"),
                Col::Hero => assert!(got >= hero_need, "Hero {got} < {hero_need} {at}"),
                Col::Map => assert!(got > 0.0, "{at}"),
                _ => {}
            }
        }
        let chevron = cells.last().unwrap().bounds();
        let end = chevron.x + chevron.width;
        assert!(end <= row.bounds().width + 0.5, "row ends at {end} {at}");
        eprintln!(
            "{at}: row {:.0}, fixed {fixed} + chevron {CHEVRON} + gaps {}, {}",
            row.bounds().width,
            12.0 * cols.len() as f32,
            shares.join(", ")
        );
    }
}
