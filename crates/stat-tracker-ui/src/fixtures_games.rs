//! The `--fixture games` snapshot: the Games tab's three open-row states
//! from UX Spec v2 (normal, unsure, undecided), with Wrecking Ball history
//! so the "vs avg" deltas have 20 games to average. Example data only.

use chrono::{TimeZone, Utc};
use stat_tracker::storage::{HeroSegment, PersonalMatch, Snapshot};
use surrealdb_types::Datetime as SurrealDatetime;

use crate::fixtures::{dt, pm};

/// The corrected game: E read 22, set to 24; uploaded; tracked 9:14.
pub const NORMAL: &str = "sess-g-normal";
/// Two unsure stats and a one-snapshot Sigma swap; waiting to upload.
pub const UNSURE: &str = "sess-g-unsure";
/// No outcome, one scoreboard read, upload rejected, no kit.
pub const UNDECIDED: &str = "sess-g-undecided";

/// The server message on the rejected sample upload.
pub const REJECT: &str =
    "reader version cv-v0 is no longer accepted. Update the tracker, then retry.";

fn dts(d: u32, hh: u32, mm: u32, ss: u32) -> SurrealDatetime {
    SurrealDatetime::from(Utc.with_ymd_and_hms(2026, 10, d, hh, mm, ss).unwrap())
}

fn seg(hero: &str, snapshots: u32, first: SurrealDatetime, last: SurrealDatetime) -> HeroSegment {
    HeroSegment {
        hero: hero.into(),
        role: "Tank".into(),
        first_seen: first,
        last_seen: last,
        snapshots,
        confirmed: snapshots >= 2,
        resolution: None,
    }
}

/// A Wrecking Ball game at the history means (E 18, D 7, A 7, 9.2k, 1.3k,
/// 4.6k), uploaded.
fn wb(id: String, map: &str, outcome: &str, at: SurrealDatetime) -> PersonalMatch {
    let mut m = pm(
        &id,
        "Wrecking Ball",
        map,
        "Tank",
        outcome,
        18,
        7,
        7,
        9200,
        1300,
        4600,
        at,
    );
    m.synced = true;
    m.recognizer = "cv-v1".into();
    m
}

fn jq(id: String, map: &str, outcome: &str, at: SurrealDatetime) -> PersonalMatch {
    let mut m = pm(
        &id,
        "Junker Queen",
        map,
        "Tank",
        outcome,
        20,
        8,
        6,
        8100,
        900,
        0,
        at,
    );
    m.synced = true;
    m.recognizer = "cv-v1".into();
    m
}

pub fn snapshot() -> Snapshot {
    // Newest first. Sunday night (Oct 18): the unsure and normal games,
    // then three more. Saturday night (Oct 17) holds the undecided game.
    let mut unsure = pm(
        UNSURE,
        "Wrecking Ball",
        "Suravasa",
        "Tank",
        "victory",
        28,
        7,
        11,
        8900,
        1400,
        4200,
        dts(18, 22, 40, 25),
    );
    unsure.recognizer = "cv-v1".into();
    unsure.suspect_fields = vec!["dmg".into(), "h".into()];
    unsure.heroes_played = vec![
        seg("Wrecking Ball", 6, dts(18, 22, 29, 20), dts(18, 22, 35, 0)),
        seg("Sigma", 1, dts(18, 22, 35, 40), dts(18, 22, 35, 40)),
        seg("Wrecking Ball", 5, dts(18, 22, 36, 10), dts(18, 22, 40, 25)),
    ];

    let mut normal = pm(
        NORMAL,
        "Wrecking Ball",
        "Circuit Royal",
        "Tank",
        "victory",
        22,
        6,
        9,
        10400,
        1100,
        5000,
        dts(18, 22, 25, 19),
    );
    normal.synced = true;
    normal.recognizer = "cv-v1".into();
    normal.corrected_elims = Some(24);
    normal.edited_fields = vec!["elims".into()];
    normal.edited_at = Some(dts(18, 22, 30, 0));
    normal.heroes_played = vec![seg(
        "Wrecking Ball",
        8,
        dts(18, 22, 16, 5),
        dts(18, 22, 25, 19),
    )];

    let mut undecided = pm(
        UNDECIDED,
        "Junker Queen",
        "Suravasa",
        "Tank",
        "",
        21,
        8,
        5,
        7600,
        800,
        3900,
        dt(2026, 10, 17, 21, 11),
    );
    undecided.recognizer = "cv-v0".into();
    undecided.upload_reject = Some(REJECT.into());
    undecided.heroes_played = vec![seg(
        "Junker Queen",
        1,
        dts(17, 21, 11, 0),
        dts(17, 21, 11, 0),
    )];

    let mut matches = vec![
        unsure,
        normal,
        wb(
            "sess-g-h01".into(),
            "Nepal",
            "defeat",
            dt(2026, 10, 18, 22, 5),
        ),
        wb(
            "sess-g-h02".into(),
            "Lijiang Tower",
            "victory",
            dt(2026, 10, 18, 21, 50),
        ),
        wb(
            "sess-g-h03".into(),
            "Busan",
            "victory",
            dt(2026, 10, 18, 21, 31),
        ),
        wb(
            "sess-g-h04".into(),
            "Junkertown",
            "victory",
            dt(2026, 10, 17, 22, 1),
        ),
        wb(
            "sess-g-h05".into(),
            "King's Row",
            "victory",
            dt(2026, 10, 17, 21, 52),
        ),
        wb(
            "sess-g-h06".into(),
            "Ilios",
            "defeat",
            dt(2026, 10, 17, 21, 31),
        ),
        undecided,
        wb(
            "sess-g-h07".into(),
            "Colosseo",
            "defeat",
            dt(2026, 10, 17, 21, 2),
        ),
        wb(
            "sess-g-h08".into(),
            "Busan",
            "victory",
            dt(2026, 10, 17, 20, 41),
        ),
        wb(
            "sess-g-h09".into(),
            "Dorado",
            "defeat",
            dt(2026, 10, 17, 20, 33),
        ),
    ];
    // Older nights: more Wrecking Ball, and Junker Queen's other three games.
    let maps = [
        "Numbani",
        "Midtown",
        "Oasis",
        "Hollywood",
        "Havana",
        "Rialto",
    ];
    for (n, day) in [(10u32, 10u32), (16, 3)] {
        for i in 0..6u32 {
            let outcome = if i % 3 == 2 { "defeat" } else { "victory" };
            let at = dt(2026, 10, day, 21, 50 - i * 9);
            matches.push(wb(
                format!("sess-g-h{:02}", n + i),
                maps[i as usize],
                outcome,
                at,
            ));
        }
    }
    for i in 0..3u32 {
        let at = dt(2026, 10, 10, 20, 40 - i * 10);
        matches.push(jq(format!("sess-g-jq{i}"), "Paraiso", "victory", at));
    }
    matches.sort_by_key(|m| std::cmp::Reverse(m.played_at));
    Snapshot {
        matches,
        sessions: Vec::new(),
    }
}

/// A saved scoreboard picture for the normal and unsure games, so their
/// panels offer "Scoreboard picture". The undecided game has none.
pub fn write_kits(data_dir: &std::path::Path) -> anyhow::Result<()> {
    use stat_tracker::report::blank::BLANK_TABLE_VERSION;
    use stat_tracker::report::bundle::sha256_hex;
    use stat_tracker::report::kit::{KitReads, READS_JSON, SCOREBOARD_PNG, session_dir};
    let board = ::image::RgbImage::from_pixel(64, 36, ::image::Rgb([28, 28, 37]));
    let mut png = Vec::new();
    ::image::DynamicImage::ImageRgb8(board).write_to(
        &mut std::io::Cursor::new(&mut png),
        ::image::ImageFormat::Png,
    )?;
    for sid in [NORMAL, UNSURE] {
        let dir = session_dir(data_dir, sid).ok_or_else(|| anyhow::anyhow!("kit dir"))?;
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(SCOREBOARD_PNG), &png)?;
        let reads = KitReads {
            captured_at: Utc.with_ymd_and_hms(2026, 10, 18, 22, 40, 0).unwrap(),
            width: 64,
            height: 36,
            team_size: Some(5),
            map: None,
            mode: None,
            result: None,
            recognizer: "cv-v1".into(),
            blank_table: BLANK_TABLE_VERSION,
            scoreboard_sha256: sha256_hex(&png),
            reads: Default::default(),
        };
        std::fs::write(dir.join(READS_JSON), serde_json::to_vec(&reads)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::FixtureKind;
    use crate::games_detail::{self, Stat, Tracked, Upload};
    use crate::model::Outcome;
    use crate::snapshot::{games_from_snapshot, install_fixture};

    fn games() -> Vec<crate::model::Game> {
        games_from_snapshot(&snapshot())
    }

    fn game<'a>(all: &'a [crate::model::Game], sid: &str) -> &'a crate::model::Game {
        all.iter().find(|g| g.session_id == sid).expect(sid)
    }

    #[test]
    fn the_three_states_read_as_the_spec_draws_them() {
        let all = games();
        let n = game(&all, NORMAL);
        assert_eq!(games_detail::read_vs_set(n, Stat::E), Some((22, 24)));
        assert_eq!(games_detail::tracked(n), Tracked::Span(554));
        assert_eq!(games_detail::upload(n, false, true), Upload::Uploaded);
        let avg = games_detail::averages(n, &all);
        assert_eq!(avg.window, 20);
        let deltas: Vec<_> = Stat::ALL
            .iter()
            .map(|&s| games_detail::delta_line(n, s, &avg).text().unwrap())
            .collect();
        assert_eq!(
            deltas,
            [
                "+6 vs avg",
                "\u{2212}1 vs avg",
                "+2 vs avg",
                "+1.2k vs avg",
                "\u{2212}0.2k vs avg",
                "+0.4k vs avg"
            ]
        );

        let u = game(&all, UNSURE);
        assert_eq!(games_detail::tracked(u), Tracked::Span(665));
        assert_eq!(
            games_detail::hero_split(u).line(),
            "Wrecking Ball 92% · Sigma 8% unconfirmed"
        );
        assert_eq!(games_detail::upload(u, false, true), Upload::Waiting);

        let d = game(&all, UNDECIDED);
        assert_eq!(d.outcome, Outcome::Unknown);
        assert_eq!(games_detail::tracked(d), Tracked::OneRead);
        assert!(games_detail::averages(d, &all).means.is_none());
        assert!(games_detail::upload(d, false, true).rejected());
    }

    #[test]
    fn install_saves_a_picture_for_two_of_the_three() {
        let dir = tempfile::tempdir().unwrap();
        install_fixture(dir.path(), FixtureKind::Games).unwrap();
        assert!(crate::report::has_kit(dir.path(), NORMAL));
        assert!(crate::report::has_kit(dir.path(), UNSURE));
        assert!(!crate::report::has_kit(dir.path(), UNDECIDED));
    }
}
