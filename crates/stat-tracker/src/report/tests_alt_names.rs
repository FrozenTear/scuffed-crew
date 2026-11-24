//! The member's other saved names (`player_names`) never reach a report.

use super::scrub::{ScrubContext, scrub_text};

/// `QXAlt` and `QXMain` are made-up names, not members'.
#[test]
fn saved_alt_names_are_scrubbed_like_the_main_name() {
    let config = crate::config::Config {
        player_name: Some("QXMain".into()),
        player_names: vec!["QXAlt".into()],
        ..crate::config::Config::default()
    };
    let mut ctx = ScrubContext::from_config(&config);
    assert_eq!(
        ctx.names,
        ["QXAlt"],
        "the alt is on the list from the start"
    );
    // The names read in the window join it; they do not replace it.
    ctx.names.extend(["QXOther".to_string()]);
    let out = scrub_text(&ctx, "own row QXMain, alt qxalt, then QXOther");
    for name in ["QXMain", "QXAlt", "qxalt", "QXOther"] {
        assert!(!out.contains(name), "{name} in {out}");
    }
}
