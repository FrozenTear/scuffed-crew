//! One check of the repo's copy rule over the 0.6 modules: no em dash and
//! no en dash anywhere in their source, so every string they draw (consts,
//! inline literals, the pieces `format!` joins) and every comment is
//! covered, with the README and the help text.

/// The modules the 0.6 top tabs and Settings page added or rewrote.
const SOURCES: &[(&str, &str)] = &[
    ("account_card.rs", include_str!("account_card.rs")),
    ("capture_reading.rs", include_str!("capture_reading.rs")),
    ("capture_shared.rs", include_str!("capture_shared.rs")),
    ("chrome.rs", include_str!("chrome.rs")),
    ("companion_gate.rs", include_str!("companion_gate.rs")),
    ("cli.rs", include_str!("cli.rs")),
    ("form_widgets.rs", include_str!("form_widgets.rs")),
    ("games_state.rs", include_str!("games_state.rs")),
    ("icons.rs", include_str!("icons.rs")),
    ("nav.rs", include_str!("nav.rs")),
    ("nav_fit.rs", include_str!("nav_fit.rs")),
    ("overview_chips.rs", include_str!("overview_chips.rs")),
    ("overview_heroes.rs", include_str!("overview_heroes.rs")),
    ("overview_main.rs", include_str!("overview_main.rs")),
    ("overview_page.rs", include_str!("overview_page.rs")),
    ("overview_side.rs", include_str!("overview_side.rs")),
    ("maps_tab.rs", include_str!("maps_tab.rs")),
    ("seasons.rs", include_str!("seasons.rs")),
    ("seasons_tab.rs", include_str!("seasons_tab.rs")),
    ("small_sample.rs", include_str!("small_sample.rs")),
    ("tab_bits.rs", include_str!("tab_bits.rs")),
    ("session_bits.rs", include_str!("session_bits.rs")),
    ("stat_tiles.rs", include_str!("stat_tiles.rs")),
    ("settings_cards.rs", include_str!("settings_cards.rs")),
    ("settings_env.rs", include_str!("settings_env.rs")),
    ("settings_fixed.rs", include_str!("settings_fixed.rs")),
    ("settings_model.rs", include_str!("settings_model.rs")),
    ("settings_open.rs", include_str!("settings_open.rs")),
    ("settings_page.rs", include_str!("settings_page.rs")),
    ("settings_rows.rs", include_str!("settings_rows.rs")),
    ("settings_save.rs", include_str!("settings_save.rs")),
    ("settings_update.rs", include_str!("settings_update.rs")),
    ("tracker_status.rs", include_str!("tracker_status.rs")),
    ("update_check.rs", include_str!("update_check.rs")),
    ("winrate_bar.rs", include_str!("winrate_bar.rs")),
    ("what_leaves.rs", include_str!("what_leaves.rs")),
    ("README.md", include_str!("../README.md")),
    (
        "assets/draw_icons.py",
        include_str!("../assets/draw_icons.py"),
    ),
];

#[test]
fn the_0_6_modules_have_no_em_or_en_dashes() {
    for (name, source) in SOURCES {
        for (i, line) in source.lines().enumerate() {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{name}:{}: {line}",
                i + 1
            );
        }
    }
}
