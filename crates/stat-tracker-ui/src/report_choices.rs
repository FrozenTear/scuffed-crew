//! The game picker of the report dialog: which games it lists. Kits are
//! kept for up to 300 games, and the picker lists the newest 50 with a
//! saved picture and the newest 8 without one, plus the selected game.

use crate::app::TrackerApp;
use crate::model::local_hm;

impl TrackerApp {
    pub(crate) fn game_choices(&self, selected: Option<&str>) -> Vec<crate::report::GameChoice> {
        // `games` is newest first. Kits are kept for up to 300 games, so the
        // games with a saved picture are capped too, at the newest 50, and
        // the games without one at the newest 8: a long history is not a
        // column of hundreds of chips. A game already selected stays listed.
        const RECENT_WITH_KIT: usize = 50;
        const RECENT_WITHOUT_KIT: usize = 8;
        let mut with_kit = 0;
        let mut without_kit = 0;
        let mut choices = Vec::new();
        for game in &self.games {
            // Every stored game can be reported: without a saved picture
            // the report carries the stored values and the session id.
            let has_picture = self.sessions_with_kit.contains(&game.session_id);
            let has_kit = true;
            let (listed, cap) = if has_picture {
                (&mut with_kit, RECENT_WITH_KIT)
            } else {
                (&mut without_kit, RECENT_WITHOUT_KIT)
            };
            if *listed == cap {
                if selected != Some(game.session_id.as_str()) {
                    continue;
                }
            } else {
                *listed += 1;
            }
            choices.push(crate::report::GameChoice {
                session_id: game.session_id.clone(),
                label: format!(
                    "{} · {} · {} {}{}",
                    game.display_hero(),
                    game.display_map(),
                    game.played_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d"),
                    local_hm(game.played_at),
                    if has_picture {
                        ""
                    } else {
                        " · no picture saved"
                    }
                ),
                has_kit,
            });
        }
        choices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kits are kept for 300 games: the picker lists the newest 50 games
    /// with a picture and the newest 8 without, plus a selected older game.
    #[test]
    fn game_picker_lists_the_newest_50_kit_games() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TrackerApp::new(crate::fixtures::test_cli(tmp.path())).0;
        let template = crate::snapshot::games_from_snapshot(&crate::fixtures_games::snapshot())
            .into_iter()
            .next()
            .expect("a fixture game");
        let newest = template.played_at;
        let game = |i: usize, sid: String| {
            let mut g = template.clone();
            g.session_id = sid;
            g.played_at = newest - chrono::Duration::minutes(i as i64);
            g
        };
        // Newest first, like the snapshot: 300 kit games, then 20 without.
        app.games = (0..300)
            .map(|i| game(i, format!("kit{i:03}")))
            .chain((0..20).map(|i| game(300 + i, format!("bare{i:02}"))))
            .collect();
        app.sessions_with_kit = (0..300).map(|i| format!("kit{i:03}")).collect();

        let sids = |choices: Vec<crate::report::GameChoice>| -> Vec<String> {
            choices.into_iter().map(|c| c.session_id).collect()
        };
        let listed = sids(app.game_choices(None));
        let want: Vec<String> = (0..50)
            .map(|i| format!("kit{i:03}"))
            .chain((0..8).map(|i| format!("bare{i:02}")))
            .collect();
        assert_eq!(listed, want, "newest 50 kit games, then newest 8 without");

        let listed = sids(app.game_choices(Some("kit250")));
        assert_eq!(listed.len(), 59);
        assert_eq!(listed[50], "kit250", "the selected game stays listed");
        assert_eq!(&listed[..50], &want[..50]);
    }

    /// A selected game without a picture that is older than the newest 8
    /// games without one stays listed, after them in newest-first order.
    #[test]
    fn game_picker_keeps_a_selected_older_game_without_a_picture() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = TrackerApp::new(crate::fixtures::test_cli(tmp.path())).0;
        let template = crate::snapshot::games_from_snapshot(&crate::fixtures_games::snapshot())
            .into_iter()
            .next()
            .expect("a fixture game");
        let newest = template.played_at;
        // Newest first: 20 games, none with a saved picture.
        app.games = (0..20)
            .map(|i| {
                let mut g = template.clone();
                g.session_id = format!("bare{i:02}");
                g.played_at = newest - chrono::Duration::minutes(i as i64);
                g
            })
            .collect();
        app.sessions_with_kit.clear();

        let listed: Vec<String> = app
            .game_choices(Some("bare15"))
            .into_iter()
            .map(|c| c.session_id)
            .collect();
        let want: Vec<String> = (0..8)
            .map(|i| format!("bare{i:02}"))
            .chain(["bare15".to_string()])
            .collect();
        assert_eq!(listed, want, "the newest 8, then the selected bare15");
    }
}
