//! Daemon tests: own_row_and_hero_seeds. See `mod.rs`.

use super::*;

/// The seed store opens exactly when the new reader is in effect, and
/// both read one function (F262-5): whatever turns the new reader on,
/// the store is there. ocr-v1 never opens or creates the private folder.
#[test]
fn the_own_name_seeds_open_exactly_when_the_new_reader_is_in_effect() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config::Config {
        data_dir: dir.path().to_path_buf(),
        reader: config::ReaderSetting::OcrV1,
        ..config::Config::default()
    };
    use stat_tracker::config::OcrSetting;
    use stat_tracker::ocr_switch::{PackReadiness, decide};
    let ready = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        result_words: true,
        phase_titles: true,
        end_titles: true,
    };
    // Classic OCR on, as asked or because off was refused: `reader` decides.
    let on = decide(OcrSetting::On, &ready, &[]);
    let refused = decide(OcrSetting::Off, &PackReadiness::default(), &[]);
    // Classic OCR off: the new reader, whatever `reader` says.
    let off = decide(OcrSetting::Off, &ready, &[]);
    assert!(off.effective.is_off() && refused.effective.is_on());
    for switch in [&on, &refused] {
        assert!(
            !new_reader_in_effect(&config, switch),
            "an explicit reader = \"ocr-v1\" keeps ocr-v1"
        );
        assert!(open_own_name_seeds(new_reader_in_effect(&config, switch), &config).is_none());
    }
    assert!(
        new_reader_in_effect(&config, &off),
        "off implies the new reader, and its seed store"
    );
    assert!(open_own_name_seeds(new_reader_in_effect(&config, &off), &config).is_some());
    assert!(own_name_seeds(None).is_empty());
    // the bool decides, not the `reader` setting on its own
    assert!(open_own_name_seeds(true, &config).is_some());
    config.reader = config::ReaderSetting::New;
    for switch in [&on, &refused, &off] {
        assert!(new_reader_in_effect(&config, switch));
    }
    let store = open_own_name_seeds(new_reader_in_effect(&config, &on), &config).expect("store");
    assert!(own_name_seeds(Some(&store)).is_empty());
    assert!(!dir.path().join("private").exists(), "nothing until a crop");
    // the daemon takes the capture path, the reader packs and the seed
    // store from that one answer
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let source = norm(daemon_source());
    // Each needle is written in two pieces, so this test's own text is
    // not what it finds.
    for pinned in [
        concat!(
            "let use_new_reader = new_reader_in_effect(&config, &ocr_switch.decision); ",
            "if use_new_reader { shadow::init("
        ),
        concat!(
            "use_new_reader, own_name: ",
            "open_own_name_seeds(use_new_reader, &config),"
        ),
    ] {
        assert_eq!(source.matches(pinned).count(), 1, "{pinned}");
        assert!(norm(fn_source("run")).contains(pinned), "{pinned} in run");
    }
}

#[test]
fn find_own_row_reads_only_the_switched_source() {
    use detect::own_row::{OwnRowRead, Source, Trust, combine, combine_with_hero};
    let found = |row| {
        Some(parse::NameMatch {
            row,
            sole_window: true,
            ..parse::NameMatch::default()
        })
    };
    let seeds = |seed: Option<usize>, lit| OwnRowRead {
        own: combine(seed, lit, true),
        ..OwnRowRead::unread(None, None, true)
    };
    let no_name = || -> Option<parse::NameMatch> { panic!("name read") };
    // ocr-v1: the name match, else the highlight. Neither the seeds, the
    // career hero nor the fallback are touched.
    let v1 = find_own_row(
        false,
        || found(4),
        || panic!("seeds read under ocr-v1"),
        no_name,
        o9_lit(1, 1.5),
        Some(3),
        true,
    );
    assert_eq!(v1.row_idx, Some(4));
    assert_eq!(v1.name_match.map(|m| m.row), Some(4));
    assert!(v1.own_row.is_none() && v1.candidate.is_none());
    let v1 = find_own_row(
        false,
        || None,
        || panic!("seeds read"),
        no_name,
        o9_lit(1, 1.5),
        Some(3),
        true,
    );
    assert_eq!((v1.row_idx, v1.name_match), (Some(1), None));
    let v1 = find_own_row(
        false,
        || None,
        || panic!("seeds"),
        no_name,
        None,
        Some(3),
        true,
    );
    assert_eq!(v1.row_idx, None, "ocr-v1 never takes the career hero's row");
    // new reader: the seeds and the highlight. With a row from them no
    // name is read at all.
    let both = find_own_row(
        true,
        no_name,
        || seeds(Some(2), o9_lit(2, 1.5)),
        no_name,
        o9_lit(2, 1.5),
        None,
        true,
    );
    assert_eq!((both.row_idx, both.name_match), (Some(2), None));
    assert_eq!(both.own_row.map(|o| o.trust), Some(Trust::Sure));
    // seed against a strong highlight: no row, and the name read is not
    // asked to break the tie
    let split = find_own_row(
        true,
        no_name,
        || seeds(Some(2), o9_lit(0, 1.5)),
        no_name,
        o9_lit(0, 1.5),
        None,
        true,
    );
    assert_eq!(split.row_idx, None);
    assert_eq!(split.own_row.map(|o| o.source), Some(Source::Disagree));
    // the seed alone names a row the highlight never saw (team 2)
    let alone = find_own_row(
        true,
        no_name,
        || seeds(Some(7), None),
        no_name,
        None,
        None,
        true,
    );
    assert_eq!(alone.row_idx, Some(7));
    assert!(alone.own_row.is_some_and(|o| o.suspect()));
    // seed and highlight silent, the career hero on one row of the
    // team: that row, marked, and no name is read (O9b)
    let hero = |seeded| OwnRowRead {
        own: combine_with_hero(None, None, Some(3), seeded),
        ..OwnRowRead::unread(None, Some(3), seeded)
    };
    for seeded in [false, true] {
        let by_hero = find_own_row(
            true,
            no_name,
            || hero(seeded),
            no_name,
            None,
            Some(3),
            seeded,
        );
        assert_eq!((by_hero.row_idx, by_hero.name_match), (Some(3), None));
        let own = by_hero.own_row.expect("own row");
        assert_eq!(
            (own.source, own.trust),
            (Source::CareerHero, Trust::Suspect)
        );
        assert!(own.suspect());
    }
    // nothing names a row: main's name read is asked, for this board.
    // With no seed for the board the row is stored as main stores it.
    let silent = |seeded| OwnRowRead {
        own: combine(None, None, seeded),
        ..OwnRowRead::unread(None, None, seeded)
    };
    let named = find_own_row(
        true,
        no_name,
        || silent(false),
        || found(6),
        None,
        None,
        false,
    );
    assert_eq!(named.row_idx, Some(6));
    assert_eq!(named.name_match.map(|m| m.row), Some(6));
    let own = named.own_row.expect("own row");
    assert_eq!(
        (own.source, own.trust),
        (Source::NameFallback, Trust::Unconfirmed)
    );
    assert!(!own.suspect(), "stored as main stores a name-matched row");
    // With a seed for the board that matched no row, the name's row is
    // marked: one voice where two could have spoken (F262-18). The
    // board's own answer decides, not the caller's "any seed exists".
    let named = find_own_row(
        true,
        no_name,
        || silent(true),
        || found(6),
        None,
        None,
        false,
    );
    let own = named.own_row.expect("own row");
    assert_eq!((named.row_idx, own.source), (Some(6), Source::NameFallback));
    assert_eq!(own.trust, Trust::Suspect);
    assert!(own.suspect());
    let named = find_own_row(
        true,
        no_name,
        || silent(false),
        || found(6),
        None,
        None,
        true,
    );
    assert!(!named.own_row.expect("own row").suspect());
    // ...and when that finds no row either (or OCR is off): held
    let none = find_own_row(true, no_name, || silent(true), || None, None, None, true);
    assert_eq!((none.row_idx, none.name_match), (None, None));
    assert_eq!(none.own_row.map(|o| o.source), Some(Source::Neither));
    // a panic in the seed read: the highlight, marked when seeds exist
    let panicked = find_own_row(
        true,
        no_name,
        || panic!("seed reader"),
        no_name,
        o9_lit(3, 1.5),
        Some(1),
        true,
    );
    assert_eq!(panicked.row_idx, Some(3));
    assert_eq!(panicked.own_row.map(|o| o.trust), Some(Trust::Suspect));
    assert!(panicked.candidate.is_none());
    let panicked = find_own_row(
        true,
        no_name,
        || panic!("seed reader"),
        no_name,
        o9_lit(3, 1.5),
        None,
        false,
    );
    assert_eq!(panicked.own_row.map(|o| o.trust), Some(Trust::Unconfirmed));
    // ...and with no highlight either, the career hero's row, marked
    let panicked = find_own_row(
        true,
        no_name,
        || panic!("seed reader"),
        no_name,
        None,
        Some(1),
        false,
    );
    assert_eq!(panicked.row_idx, Some(1));
    assert_eq!(
        panicked.own_row.map(|o| (o.source, o.trust)),
        Some((Source::CareerHero, Trust::Suspect))
    );
}

#[test]
fn the_name_fallback_needs_a_configured_name() {
    // No name, or a blank one: nothing is read (a blank board would give
    // no match anyway; the point is that no window is OCRed).
    let board = o9_board(0);
    assert_eq!(
        own_row_name_fallback(&[], &board, 5, &names::NameSwitch::off()),
        None
    );
    assert_eq!(
        own_row_name_fallback(&["  ".into()], &board, 5, &names::NameSwitch::off()),
        None
    );
    // With classic OCR off the name read is not asked for at all: no
    // call reaches the gate, so nothing is counted against the own row.
    {
        let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
        assert_eq!(
            own_row_name_fallback(&["PLAYER".into()], &board, 5, &names::NameSwitch::off()),
            None
        );
        assert_eq!(off.blocked(), [], "the fallback returns before any OCR");
    }
}

/// [`stat_tracker::ocr_switch::Read::OwnRow`] with classic OCR off, and
/// both sides of its one condition (a hero pack that reads the board),
/// through the real seed read and the real name fallback.
///
/// * The icons are read: no highlight and no seed, and the career
///   panel's hero on one row of the member's team. That row is the own
///   row, marked, and no name is read.
/// * No icon read to go by (no hero pack for the board, or the own icon
///   dead or covered): no row. With OCR off the Tab is held and nothing
///   goes to the gate: the stated limit. With OCR on the name read is
///   asked, which is why off is refused without a hero pack.
#[test]
fn with_ocr_off_the_career_hero_names_the_own_row_and_without_it_the_tab_is_held() {
    use detect::own_row::{Source, Trust};
    const OWN: usize = 3;
    let hero_read = |row: usize, hero: Option<&str>| shadow::heroes::HeroRead {
        row,
        class: if hero.is_some() {
            shadow::heroes::RowClass::Hero
        } else {
            shadow::heroes::RowClass::Unknown
        },
        hero: hero.map(str::to_string),
        best: hero.unwrap_or("ana").to_string(),
        score: if hero.is_some() { 0.95 } else { 0.5 },
        lead: 0.4,
        second: "kiriko".into(),
        suspect: hero.is_none(),
    };
    // The career panel shows Ana. `own_icon`: what is read on the
    // member's row (`None`: a skull or an ult ring over the icon).
    let icons = |own_icon: Option<&str>| shadow::TabHeroes {
        team_size: 5,
        read: Some(shadow::heroes::HeroFrameRead {
            board: shadow::heroes::HeroBoard {
                rows: (0..10)
                    .map(|row| {
                        if row == OWN {
                            hero_read(row, own_icon)
                        } else {
                            hero_read(row, Some("mercy"))
                        }
                    })
                    .collect(),
                elapsed_us: 1,
            },
            pack: shadow::heroes::HeroPack::V2,
            pad_px: None,
        }),
        panel: Some(shadow::heroes::PanelRead {
            read: hero_read(0, Some("ana")),
            pack: shadow::heroes::HeroPack::V2,
            box_px: (0, 0, 1, 1),
            rel: (0.03, -0.03, 0.16),
        }),
    };
    // What `shadow::read_heroes` returns with no hero pack for the board.
    let no_pack = shadow::TabHeroes {
        team_size: 5,
        read: None,
        panel: None,
    };
    let board = o9_board(OWN);
    let find = |heroes: &shadow::TabHeroes,
                name_fallback: &dyn Fn() -> Option<parse::NameMatch>| {
        find_own_row(
            true,
            || -> Option<parse::NameMatch> { panic!("ocr-v1's name match") },
            // no highlight on the board, and no seed learned yet
            || detect::own_row::read_own_row(None, &board, 5, None, Some(heroes), &[]),
            name_fallback,
            None,
            detect::own_row::career_hero_row(heroes),
            false,
        )
    };

    {
        let off = ocr::gate::scoped(ocr::gate::OcrMode::Off);
        let the_fallback = || {
            own_row_name_fallback(&["PLAYER".into()], &board, 5, &names::NameSwitch::off())
                .map(|hit| hit.name)
        };

        let by_hero = find(&icons(Some("ana")), &the_fallback);
        assert_eq!((by_hero.row_idx, by_hero.name_match), (Some(OWN), None));
        assert_eq!(
            by_hero.own_row.map(|own| (own.source, own.trust)),
            Some((Source::CareerHero, Trust::Suspect)),
            "the row is marked until a seed agrees"
        );

        // The limit: never the highlight, no seed, the own icon unread.
        for heroes in [icons(None), no_pack.clone()] {
            let held = find(&heroes, &the_fallback);
            assert_eq!((held.row_idx, held.name_match), (None, None));
            assert_eq!(held.own_row.map(|own| own.source), Some(Source::Neither));
        }
        assert_eq!(off.blocked(), [], "none of it asked anything of OCR");
    }

    // OCR on (the run of a daemon that was refused off for the hero
    // pack): with no icon read the name read is asked, once, and its
    // row is the own row.
    assert!(!ocr::gate::is_off());
    let asked = std::cell::Cell::new(0);
    let name_read = || {
        asked.set(asked.get() + 1);
        Some(parse::NameMatch {
            row: 6,
            sole_window: true,
            ..parse::NameMatch::default()
        })
    };
    let named = find(&no_pack, &name_read);
    assert_eq!(asked.get(), 1);
    assert_eq!(named.row_idx, Some(6));
    assert_eq!(
        named.own_row.map(|own| own.source),
        Some(Source::NameFallback)
    );
    // and where the career hero names the row, no name is read
    let by_hero = find(&icons(Some("ana")), &name_read);
    assert_eq!((by_hero.row_idx, asked.get()), (Some(OWN), 1));
}

#[test]
fn the_text_fallback_gets_no_name_when_the_new_reader_has_no_own_row() {
    let names = ["Made Up".to_string(), "Alt".to_string()];
    let none: &[String] = &[];
    // ocr-v1: every saved name, with or without a row
    assert_eq!(text_fallback_names(false, None, &names), names);
    assert_eq!(text_fallback_names(false, Some(2), &names), names);
    // new reader: only beside a row the seeds, the highlight or the
    // name fallback found
    assert_eq!(text_fallback_names(true, Some(2), &names), names);
    assert_eq!(text_fallback_names(true, None, &names), none);
    assert_eq!(text_fallback_names(true, None, none), none);
}

/// The raw-text fallback tries every saved name: an alt's line is read
/// when the main name is on no line, and a name with no line stays a
/// "row not found".
#[test]
fn the_text_fallback_reads_an_alt_names_line() {
    let raw = "Ana\nBusan\nÖSTER 10 4 2 3,000 800 400";
    let names = ["FrozenTear".to_string(), "öster".to_string()];
    let read = read_scoreboard_named(&[], None, None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
    let main_only = ["FrozenTear".to_string()];
    assert!(matches!(
        read_scoreboard_named(&[], None, None, raw, "victory", &main_only),
        Err(parse::ScoreboardMiss::PlayerRowNotFound)
    ));
}

/// The main name in a chat line makes that name "found but unreadable"
/// (the text fallback skips chat). With no own row the next saved name is
/// still tried, so an alt's stat line is read. With no alt line the miss
/// stays "unreadable", not "row not found". A highlighted row whose name
/// reads as the alt reads the alt's line past the chat line too.
#[test]
fn a_main_name_in_chat_does_not_hide_the_alts_line() {
    let raw = "[Team] FrozenTear: 1 2 3 4 5 6\nÖSTER 10 4 2 3,000 800 400";
    let names = ["FrozenTear".to_string(), "öster".to_string()];
    let chat_only = "[Team] FrozenTear: 1 2 3 4 5 6";
    let read = read_scoreboard_named(&[], None, None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
    assert!(!read.trusted_cells);
    assert!(matches!(
        read_scoreboard_named(&[], None, None, chat_only, "victory", &names),
        Err(parse::ScoreboardMiss::CellsUnreadable)
    ));
    let alt_row = [two_account_row(Some("ÖSTER"), false)];
    let read = read_scoreboard_named(&alt_row, Some(0), None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
    assert!(!read.trusted_cells);
    assert!(matches!(
        read_scoreboard_named(&alt_row, Some(0), None, chat_only, "victory", &names),
        Err(parse::ScoreboardMiss::CellsUnreadable)
    ));
}

/// One OCR row for the two-account tests: `name` as its name read, and
/// stat cells that parse (`readable`) or are blank.
fn two_account_row(name: Option<&str>, readable: bool) -> ocr::RowOcrResult {
    let stats = ["5", "3", "2", "4,316", "1,200", "899"];
    lobby_row(name, readable.then_some(stats))
}

/// One OCR row of a lobby: `name` as its name read, and `stats` as its six
/// stat cells (`None`: blank cells that do not parse).
fn lobby_row(name: Option<&str>, stats: Option<[&str; 6]>) -> ocr::RowOcrResult {
    let cell = |value: &str| ocr::CellOcrResult {
        value: value.into(),
        confidence: 80,
        suspect: false,
    };
    ocr::RowOcrResult {
        name: name.map(cell),
        name_alternates: Vec::new(),
        stats: stats.unwrap_or([""; 6]).iter().map(|v| cell(v)).collect(),
        mean_confidence: 80,
    }
}

/// Alpha.3 item 15. A highlighted own row whose cells do not parse, on a
/// board with two of the member's saved names: the raw text holds a
/// readable line for the OTHER saved name only. The row reads as the main
/// name, as no saved name, or was not read at all: none of these reads the
/// other account's line. The read is "unreadable".
#[test]
fn an_unreadable_own_row_does_not_read_the_other_saved_accounts_line() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let raw = "Ana\nBusan\nALT 10 4 2 3,000 800 400";
    for name in [Some("MAIN"), Some("Stranger"), None] {
        let rows = [two_account_row(name, false)];
        assert!(
            matches!(
                read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names),
                Err(parse::ScoreboardMiss::CellsUnreadable)
            ),
            "row name {name:?}"
        );
    }
    // No OCR rows at all: the row's name was never read. This used to read
    // Alt's line; it is now unreadable on purpose.
    assert!(matches!(
        read_scoreboard_named(&[], Some(0), None, raw, "victory", &names),
        Err(parse::ScoreboardMiss::CellsUnreadable)
    ));
}

/// The same board, with that row's own name on a readable text line: the
/// row's own line is read (low trust), not the other account's. Cells
/// that parse still win over any line. One saved name reads its line
/// beside the row as before, whatever the row's name read says.
#[test]
fn a_highlighted_row_reads_its_own_line_when_its_cells_do_not_parse() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let raw = "[Team] Alt: 1 2 3 4 5 6\nMAIN 3 1 4 1,500 900 260\nALT 10 4 2 3,000 800 400";
    for (name, want) in [("ALT", (10, 3000)), ("MAIN", (3, 1500))] {
        let rows = [two_account_row(Some(name), false)];
        let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names).unwrap();
        assert_eq!((read.matched.elims, read.matched.damage), want, "{name}");
        assert!(!read.trusted_cells, "{name}");
    }
    let rows = [two_account_row(Some("MAIN"), true)];
    let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (5, 4316));
    assert!(read.trusted_cells);
    let main_only = ["Main".to_string()];
    let rows = [two_account_row(None, false)];
    let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", &main_only).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (3, 1500));
}

/// Site's probe on PR review (ocr-v1): "Spocle" sits inside "Spocle2". The
/// highlighted row reads SPOCLE2 and its cells do not parse; the text holds
/// both accounts' lines. The own line (3, 1500) is read in either saved
/// order. The old first-substring rule named the row "Spocle" with Spocle
/// listed first and read the other account's line (10, 3000).
#[test]
fn an_overlapping_saved_name_reads_the_highlighted_rows_own_line() {
    let raw = "Ana\nBusan\nSPOCLE 10 4 2 3,000 800 400\nSPOCLE2 3 1 4 1,500 900 260";
    let rows = [two_account_row(Some("SPOCLE2"), false)];
    let main_first = ["Spocle".to_string(), "Spocle2".to_string()];
    let alt_first = ["Spocle2".to_string(), "Spocle".to_string()];
    for names in [&main_first, &alt_first] {
        assert_eq!(
            parse::best_name_for_row(&rows[0], &names[..]),
            Some("Spocle2"),
            "{names:?}"
        );
        let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", names).unwrap();
        assert_eq!(
            (read.matched.elims, read.matched.damage),
            (3, 1500),
            "{names:?}"
        );
        assert!(!read.trusted_cells, "{names:?}");
    }
}

/// A lobby of ten rows for the ocr-v1 own-row tests: Spocle readable on row
/// 7, `own` on the highlighted row 2 with `own_stats`, strangers elsewhere.
fn two_account_lobby(own: &str, own_stats: Option<[&str; 6]>) -> Vec<ocr::RowOcrResult> {
    let spocle = ["10", "4", "2", "3,000", "800", "400"];
    let stranger = ["1", "1", "1", "100", "100", "100"];
    (0..10)
        .map(|i| match i {
            2 => lobby_row(Some(own), own_stats),
            7 => lobby_row(Some("SPOCLE"), Some(spocle)),
            _ => lobby_row(Some(&format!("Stranger{i}")), Some(stranger)),
        })
        .collect()
}

/// The ocr-v1 own row as the daemon chooses it (`analyze_frame`'s name
/// match, the names switch off): the highlighted row's own name first,
/// then the whole board, then the highlight.
fn ocr_v1_own_row(
    rows: &[ocr::RowOcrResult],
    highlight: usize,
    names: &[String],
) -> (Option<usize>, Option<parse::NameMatch>) {
    let choice = find_own_row(
        false,
        || {
            highlighted_own_name(rows, Some(highlight), names)
                .or_else(|| names::match_named_rows(rows, names, 5, names::BoardMatch::Cells))
        },
        || panic!("seeds read under ocr-v1"),
        || panic!("name fallback under ocr-v1"),
        o9_lit(highlight, 1.5),
        None,
        false,
    );
    (choice.row_idx, choice.name_match)
}

/// Site's blocking finding 1, cells readable: FrozenTear plays on Sinmic
/// (highlighted, row 2) while Spocle, another saved name, sits readable on
/// row 7. The highlighted row wins and its own cells are stored, whatever
/// the saved order, and also when the row's name read is fuzzy ("SINMlC")
/// while Spocle is an exact hit (the old board match took row 7).
#[test]
fn ocr_v1_takes_the_highlighted_row_over_another_saved_name_on_the_board() {
    let main_first = ["Spocle".to_string(), "Sinmic".to_string()];
    let alt_first = ["Sinmic".to_string(), "Spocle".to_string()];
    let own_stats = ["7", "2", "1", "2,100", "300", "50"];
    for names in [&main_first, &alt_first] {
        let sinmic = names.iter().position(|n| n == "Sinmic").unwrap();
        for read_as in ["SINMIC", "SINMlC"] {
            let rows = two_account_lobby(read_as, Some(own_stats));
            let (row, found) = ocr_v1_own_row(&rows, 2, names);
            assert_eq!(row, Some(2), "{names:?} {read_as}");
            assert_eq!(found.map(|m| m.name), Some(sinmic), "{names:?} {read_as}");
            let read = read_scoreboard_named(&rows, row, found, "", "victory", names).unwrap();
            assert_eq!((read.matched.elims, read.matched.damage), (7, 2100));
            assert!(read.trusted_cells);
        }
    }
    // A highlighted row that names no saved name leaves the board match
    // as before: Spocle's row. One saved name never asks the highlight.
    let rows = two_account_lobby("Stranger", Some(own_stats));
    assert_eq!(ocr_v1_own_row(&rows, 2, &main_first).0, Some(7));
    let spocle_only = ["Spocle".to_string()];
    assert_eq!(highlighted_own_name(&rows, Some(2), &spocle_only), None);
}

/// Site's blocking finding 1, cells unreadable: the same lobby with the
/// highlighted Sinmic row's cells blank. The text fallback reads Sinmic's
/// own line, or, with no Sinmic line, nothing; never Spocle's line.
#[test]
fn ocr_v1_reads_the_highlighted_rows_line_not_another_saved_names_row() {
    let main_first = ["Spocle".to_string(), "Sinmic".to_string()];
    let alt_first = ["Sinmic".to_string(), "Spocle".to_string()];
    let both = "Ana\nBusan\nSPOCLE 10 4 2 3,000 800 400\nSINMIC 3 1 4 1,500 900 260";
    let spocle_only = "Ana\nBusan\nSPOCLE 10 4 2 3,000 800 400";
    for names in [&main_first, &alt_first] {
        let rows = two_account_lobby("SINMIC", None);
        let (row, found) = ocr_v1_own_row(&rows, 2, names);
        assert_eq!(row, Some(2), "{names:?}");
        let read = read_scoreboard_named(&rows, row, found, both, "victory", names).unwrap();
        assert_eq!((read.matched.elims, read.matched.damage), (3, 1500));
        assert!(!read.trusted_cells);
        assert!(
            matches!(
                read_scoreboard_named(&rows, row, found, spocle_only, "victory", names),
                Err(parse::ScoreboardMiss::CellsUnreadable)
            ),
            "{names:?}"
        );
    }
}

/// The highlighted row's name can sit in another name window (5v5 perk
/// and centred layouts): the match reports that window.
#[test]
fn the_highlighted_rows_name_is_read_in_any_name_window() {
    let names = ["Spocle".to_string(), "Sinmic".to_string()];
    let mut rows = two_account_lobby("0 3", None);
    rows[2].name_alternates = vec![
        Some(ocr::CellOcrResult {
            value: "SINMIC".into(),
            confidence: 80,
            suspect: false,
        }),
        None,
    ];
    let found = highlighted_own_name(&rows, Some(2), &names).expect("named");
    assert_eq!((found.row, found.window, found.name), (2, 1, 1));
}

/// Nit 4 of the review: when the names pack (or the highlight's own read)
/// matched the row, the text fallback searches for that saved name, not
/// for what Tesseract's per-cell name read of the row says. A match for
/// another row, or a name index past the list, falls back to the row's
/// own read.
#[test]
fn a_pack_matched_row_reads_the_matched_names_line() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let raw = "Ana\nBusan\nMAIN 3 1 4 1,500 900 260\nALT 10 4 2 3,000 800 400";
    let rows = [two_account_row(Some("MAIN"), false)];
    let pack = |row, name| {
        Some(parse::NameMatch {
            row,
            name,
            ..parse::NameMatch::default()
        })
    };
    for (found, want) in [
        (pack(0, 1), (10, 3000)),
        (pack(0, 0), (3, 1500)),
        (pack(3, 1), (3, 1500)),
        (pack(0, 9), (3, 1500)),
        (None, (3, 1500)),
    ] {
        let read = read_scoreboard_named(&rows, Some(0), found, raw, "victory", &names).unwrap();
        assert_eq!((read.matched.elims, read.matched.damage), want, "{found:?}");
    }
    // A pack match on a row whose Tesseract read names no saved name.
    let rows = [two_account_row(Some("Stranger"), false)];
    let read = read_scoreboard_named(&rows, Some(0), pack(0, 1), raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
}

/// The daemon asks the highlighted row's own name before the board match,
/// and passes the match on to the text fallback.
#[test]
fn the_ocr_v1_name_match_asks_the_highlighted_row_first() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let body = norm(fn_source("analyze_frame"));
    let first = body.find("highlighted_own_name(").expect("highlight first");
    assert_eq!(body.matches("highlighted_own_name(").count(), 1);
    assert!(first < body.find("names::match_player_row(").unwrap());
    let hc_body = norm(fn_source("handle_capture"));
    assert!(hc_body.contains("read_scoreboard_named( &rows, player_row_idx, name_match,"));
}

/// Fail safe for alpha.3 item 15, through `handle_capture` (ocr-v1, two
/// saved names). A Tab whose highlighted row (named Main) has no readable
/// cells, while the text holds Alt's stat line, stores nothing and logs a
/// WARN. A row that names no saved name does the same and also logs why at
/// INFO; a readable Tab logs no such line. The game stays open, and the
/// next Tab with readable cells is recorded on it. Alt's numbers are not
/// stored.
#[tokio::test]
async fn an_unreadable_highlighted_row_warns_and_the_game_keeps_recording() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let log = test_log::TestLog::default();
    let sub = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _sub = tracing::subscriber::set_default(sub);
    tracing::callsite::rebuild_interest_cache();
    let own_row = |name: &str, readable: bool| {
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.rows[0] = two_account_row(Some(name), readable);
        analysis.ocr = Ok(ocr::OcrResult {
            raw_text: "Ana\nBusan\nALT 25 5 1 9,000 100 200".into(),
            confidence: 80,
        });
        analysis
    };
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), false).await;
    ctx.own_names = vec!["Main".into(), "Alt".into()];
    let mut now = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
    active.session_id = "two".into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    let mut st = session(Some(active), now);

    let tabs = [("MAIN", false), ("STRANGER", false), ("MAIN", true)];
    for (tab, (name, readable)) in tabs.into_iter().enumerate() {
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
            analysis: own_row(name, readable),
            board: None,
        });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let report = handle_capture(&ctx, req).await.unwrap();
        assert_eq!(report.recorded, readable, "tab {tab}");
        if !readable {
            assert!(report.ignore_row, "tab {tab}");
        }
        apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "two", Ok(report), now).await;
        let open = st.active_game.as_ref().expect("the game stays open");
        assert_eq!(open.session_id, "two", "tab {tab}");
        now += Duration::from_secs(20);
    }

    let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let warned = text
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("capture rejected: stat cells unreadable"))
        .count();
    assert_eq!(warned, 2, "{text}");
    let why = text
        .lines()
        .filter(|l| l.contains("INFO") && l.contains("names none of the saved names"))
        .count();
    assert_eq!(why, 1, "{text}");
    let rows = ctx.store.get_session_snapshots("two").await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0].elims, rows[0].damage), (5, 4316));
}

/// T4 (O9): the vision pass reads names in the per-cell pass only under
/// ocr-v1 and finds the own row only through `find_own_row` with the
/// daemon's reader switch. Whitespace is ignored. Hard-wiring the switch,
/// reading names in the cell pass under the new reader, a second name
/// fallback, or matching a name in `apply_new_reader` breaks this.
#[test]
fn the_own_row_goes_through_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let src = daemon_source();
    // In src/daemon/tab/vision.rs since 0.6 S3: found by name, wherever it lives.
    let body = norm(fn_source("analyze_frame"));
    // the per-cell pass (names and stat cells) is ocr-v1's only: the new
    // reader gets rows with nothing read on them (O10 took the stat
    // cells out of it, so `ocr::Names::Skip` has no caller here)
    assert!(body.contains(
        "let read_cells = || ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size); let mut rows = read_tab_rows(use_new_reader, read_cells, || ocr::unread_rows(team_size));"
    ));
    assert_eq!(body.matches("recognize_scoreboard_cells").count(), 1);
    assert_eq!(body.matches("ocr::Names").count(), 0);
    assert!(body.contains("} = find_own_row( use_new_reader, || {"));
    // ocr-v1's name match goes through names::match_player_row. The new
    // reader's only name read is the one named fallback.
    assert_eq!(body.matches("match_player_row_by_name").count(), 0);
    assert_eq!(body.matches("names::match_player_row(").count(), 1);
    assert_eq!(body.matches("own_row_name_fallback(").count(), 1);
    assert_eq!(body.matches("recognize_scoreboard_names").count(), 0);
    // one call in the daemon (the tests no longer live in main.rs, so this
    // assertion's own text is not a match)
    assert_eq!(
        src.matches("recognize_scoreboard_names_pre_cropped(")
            .count(),
        1
    );
    assert!(fn_source("own_row_name_fallback").contains("recognize_scoreboard_names_pre_cropped("));
    assert_eq!(body.matches("detect::own_row::read_own_row(").count(), 1);
    assert_eq!(body.matches("detect::own_row::seed_gate(").count(), 1);
    // the career hero's row comes from the one icon read, in the seed
    // read and in its panic fallback (O9b)
    assert!(body.contains("highlight, tab_heroes.as_ref(), &own_seeds, ) },"));
    assert!(body.contains(
        "highlight, tab_heroes .as_ref() .and_then(detect::own_row::career_hero_row), !own_seeds.is_empty(), );"
    ));
    assert!(body.contains("player_row_idx: row_idx, name_match, own_row,"));
    // In src/daemon/tab/cells.rs since 0.6 S4: found by name, wherever it lives.
    let an_body = norm(fn_source("apply_new_reader"));
    assert!(!an_body.contains("find_player_row_by_name"));
    assert!(!an_body.contains("player_name"));
    assert!(an_body.contains(
        "if own_row_suspect { reader_apply::mark_own_row_suspect(&mut parsed.suspect_fields); }"
    ));
    // In src/daemon/tab/grab.rs since 0.6 S10: found by name, wherever it lives.
    let hc_body = norm(fn_source("handle_capture"));
    assert!(hc_body.contains("text_fallback_names(board_reader, player_row_idx, own_names),"));
    assert!(hc_body.contains(
        "note_own_seed_verdict(ctx.own_name.as_ref(), own_seed_verdict, seed_session).await;"
    ));
    // the offer sits on the new reader's own read path (O10)
    assert!(hc_body.contains(
        "if let Some(board) = new_board { let guesses = reader_apply::own_row_guesses(&board, board_row); offer_own_name_seed("
    ));
    // no name is matched for the board's row
    assert!(
        hc_body.contains(
            "let board_row = reader_apply::own_row(None, None, player_row_idx, team_size);"
        )
    );
    assert_eq!(hc_body.matches("find_player_row_by_name").count(), 0);
    // a suspect own row is one low-trust bit for the plan and both gates
    assert!(hc_body.contains("let own_row_suspect = own_row.is_some_and(|own| own.suspect());"));
    assert!(hc_body.contains("suspect_row: own_row_suspect,"));
    assert!(hc_body.contains("trusted_cells && !own_row_suspect,"));
    assert!(hc_body.contains(
        "let gate_suspect = if own_row_suspect { [true; capture_gate::GATE_COLS] } else { suspect };"
    ));
    assert!(hc_body.contains("let own_seeds = own_name_seeds(ctx.own_name.as_ref());"));
}

#[tokio::test]
async fn a_suspect_own_row_marks_every_field_and_is_not_a_trusted_read() {
    use detect::own_row::combine_with_hero;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    for (seed, lit, hero, seeded, marked) in [
        // seed and highlight agree: nothing is marked
        (Some(0), o9_lit(0, 1.5), None, true, false),
        // only the seed, or only the highlight where a seed exists
        (Some(0), None, None, true, true),
        (None, o9_lit(0, 1.5), None, true, true),
        // a seed over a weak highlight that names another row
        (Some(0), o9_lit(3, 1.1), None, true, true),
        // a strong highlight before any seed exists: as it always was
        (None, o9_lit(0, 1.5), None, false, false),
        // a weak highlight before any seed exists
        (None, o9_lit(0, 1.1), None, false, true),
        // the career hero's row alone (O9b): marked, seeds or not
        (None, None, Some(0), false, true),
        (None, None, Some(0), true, true),
        // a seed the career hero confirms: nothing is marked
        (Some(0), None, Some(0), true, false),
        // a seed the career hero contradicts: the seed's row, marked
        (Some(0), None, Some(2), true, true),
        // no seed named a row, and a strong highlight the career hero
        // is read on (two of the member's accounts in one lobby, the
        // 2026-10-19 boards): nothing is marked
        (None, o9_lit(0, 1.5), Some(0), true, false),
        // a weak highlight, or the career hero on another row: marked
        (None, o9_lit(0, 1.1), Some(0), true, true),
        (None, o9_lit(0, 1.5), Some(2), true, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let ctx = o8_ctx(dir.path(), true).await;
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.own_row = Some(combine_with_hero(seed, lit, hero, seeded));
        assert_eq!(analysis.own_row.unwrap().row, Some(0));
        let board = Some(o8_board(Some("zenyatta")));
        let report = o9_tab(&ctx, "o9", "Busan", analysis, board).await;
        let rows = ctx.store.get_session_snapshots("o9").await.unwrap();
        let gate = report.gate_state.expect("gate");
        if marked {
            // A suspect own row stores none of its numbers and does not
            // seed the totals. The gate stays low-trust until a clean row.
            assert!(
                !report.recorded,
                "seed {seed:?} highlight {lit:?} hero {hero:?} seeded {seeded}"
            );
            assert!(
                rows.is_empty(),
                "seed {seed:?} highlight {lit:?} hero {hero:?} seeded {seeded}"
            );
            assert!(gate.low_trust);
            assert_ne!(gate.accepted.damage, 3000);
        } else {
            assert!(report.recorded);
            assert!(rows[0].suspect_fields.is_empty());
            assert_eq!((rows[0].elims, rows[0].damage), (10, 3000));
            assert!(!gate.low_trust);
        }
    }
}

#[tokio::test]
async fn the_new_reader_without_an_own_row_is_not_rescued_by_the_name_in_the_text() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    // The full-board text holds the configured name with six stats: the
    // text fallback (T7) would take that line as the member's row.
    let tab = || {
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.player_row_idx = None;
        analysis.ocr = Ok(ocr::OcrResult {
            raw_text: "Ana\nBusan\nMADEUP 10 4 2 3,000 800 400".into(),
            confidence: 80,
        });
        analysis
    };
    // ocr-v1 takes it, as before
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), false).await;
    ctx.own_names = vec!["MadeUp".into()];
    assert!(o9_tab(&ctx, "o9", "Busan", tab(), None).await.recorded);
    // the new reader holds the capture: a seed names another row than a
    // strong highlight
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), true).await;
    ctx.own_names = vec!["MadeUp".into()];
    let mut analysis = tab();
    analysis.own_row = Some(combine(Some(2), o9_lit(0, 1.5), true));
    assert_eq!(analysis.own_row.unwrap().row, None);
    let board = Some(o8_board(Some("zenyatta")));
    let report = o9_tab(&ctx, "o9", "Busan", analysis, board).await;
    assert!(!report.recorded);
    assert!(
        ctx.store
            .get_session_snapshots("o9")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn gated_crops_reach_the_seed_store_and_are_trusted_after_two_real_games() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), true).await;
    ctx.own_name = Some(o9_store(dir.path()));
    let file = detect::own_row::seed_dir(dir.path()).join("seeds.json");
    // Four boards of one game are one game, and so is a second session
    // on the same map a moment later (a stat split of that game). A
    // session on another map is a second game.
    for (i, (session, map, trusted)) in [
        ("g1", "Busan", 0),
        ("g1", "Busan", 0),
        ("g1", "Busan", 0),
        ("g1-split", "Busan", 0),
        ("g2", "Ilios", 3),
    ]
    .into_iter()
    .enumerate()
    {
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.own_name_candidate = Some(o9_candidate(i));
        let board = Some(o8_board(Some("zenyatta")));
        let report = o9_tab(&ctx, session, map, analysis, board).await;
        assert!(report.recorded, "tab {i}");
        assert_eq!(
            own_name_seeds(ctx.own_name.as_ref()).len(),
            trusted,
            "tab {i}"
        );
        // a waiting crop is never on disk; the trusted seeds are
        assert_eq!(file.is_file(), trusted > 0, "tab {i}");
    }
    // the seeds the next vision pass gets find the name on a new board
    let seeds = own_name_seeds(ctx.own_name.as_ref());
    let board = detect::own_row::NameBoard::measure(None, &o9_board(7), 5).unwrap();
    assert_eq!(detect::own_row::seed_row(&board.scores(&seeds)), Some(7));
    // a restart reads them back
    assert_eq!(own_name_seeds(Some(&o9_store(dir.path()))).len(), 3);
    // a Tab without a gated crop offers nothing
    let before = std::fs::read(&file).unwrap();
    let analysis = o8_analysis(None, Some("zenyatta"));
    let board = Some(o8_board(Some("zenyatta")));
    o9_tab(&ctx, "g3", "Busan", analysis, board).await;
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

/// O9b: a crop from the career hero's row (no highlight) reaches the
/// store like a highlighted row's, and is trusted only after four
/// boards from three real games on two career heroes.
#[tokio::test]
async fn career_hero_crops_reach_the_seed_store_and_need_three_games_and_a_hero_swap() {
    use detect::own_row::{Proof, combine_with_hero};
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), true).await;
    ctx.own_name = Some(o9_store(dir.path()));
    let file = detect::own_row::seed_dir(dir.path()).join("seeds.json");
    // What trusts three highlighted crops (two games) leaves these
    // waiting; a split of the second game is not a third game; one
    // hero through three games is not enough either.
    for (i, (session, map, hero, trusted)) in [
        ("g1", "Busan", "zenyatta", 0),
        ("g1", "Busan", "zenyatta", 0),
        ("g2", "Ilios", "zenyatta", 0),
        ("g2-split", "Ilios", "zenyatta", 0),
        ("g3", "Oasis", "zenyatta", 0),
        ("g3", "Oasis", "ana", 3),
    ]
    .into_iter()
    .enumerate()
    {
        let mut analysis = o8_analysis(None, Some(hero));
        analysis.own_row = Some(combine_with_hero(None, None, Some(0), false));
        analysis.own_name_candidate = Some(o9_candidate(i).proved(Proof::CareerHero, Some(hero)));
        let board = Some(o8_board(Some(hero)));
        let report = o9_tab(&ctx, session, map, analysis, board).await;
        // A career-hero row with no highlight is a suspect own row: its
        // numbers are not stored. The crop is still offered to the seeds.
        assert!(!report.recorded, "tab {i}");
        if i == 0 {
            let rows = ctx.store.get_session_snapshots(session).await.unwrap();
            assert!(rows.is_empty(), "a suspect own row stores no stats");
        }
        assert!(report.gate_state.expect("gate").low_trust, "tab {i}");
        assert_eq!(
            own_name_seeds(ctx.own_name.as_ref()).len(),
            trusted,
            "tab {i}"
        );
        // a waiting crop is never on disk; the trusted seeds are
        assert_eq!(file.is_file(), trusted > 0, "tab {i}");
    }
    let seeds = own_name_seeds(ctx.own_name.as_ref());
    let board = detect::own_row::NameBoard::measure(None, &o9_board(7), 5).unwrap();
    assert_eq!(detect::own_row::seed_row(&board.scores(&seeds)), Some(7));
    assert_eq!(own_name_seeds(Some(&o9_store(dir.path()))).len(), 3);
}

#[tokio::test]
async fn seeds_that_keep_contradicting_a_strong_highlight_are_dropped() {
    use detect::own_row::{DISAGREE_DROPS, Verdict, combine};
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), true).await;
    ctx.own_name = Some(o9_store(dir.path()));
    let file = detect::own_row::seed_dir(dir.path()).join("seeds.json");
    for (i, (session, map)) in [("g1", "Busan"), ("g1", "Busan"), ("g2", "Ilios")]
        .into_iter()
        .enumerate()
    {
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.own_name_candidate = Some(o9_candidate(i));
        o9_tab(
            &ctx,
            session,
            map,
            analysis,
            Some(o8_board(Some("zenyatta"))),
        )
        .await;
    }
    assert_eq!(own_name_seeds(ctx.own_name.as_ref()).len(), 3);
    // Held boards (a seed against a strong highlight) are not stored,
    // but they are counted: the seeds go on the third in a row.
    let held = || {
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.player_row_idx = None;
        analysis.own_row = Some(combine(Some(2), o9_lit(0, 1.5), true));
        analysis.own_seed_verdict = Verdict::Disagree(88);
        analysis
    };
    for i in 1..=DISAGREE_DROPS {
        assert_eq!(own_name_seeds(ctx.own_name.as_ref()).len(), 3, "board {i}");
        let report = o9_tab(&ctx, "g3", "Busan", held(), Some(o8_board(None))).await;
        assert!(!report.recorded);
    }
    assert!(own_name_seeds(ctx.own_name.as_ref()).is_empty());
    assert!(!file.exists(), "the dropped seeds are off the disk too");
}

#[tokio::test]
async fn ocr_v1_never_offers_a_crop_or_creates_the_private_folder() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), false).await;
    // Even with a store, a crop and a verdict handed in, the ocr-v1
    // path uses none of them.
    ctx.own_name = Some(o9_store(dir.path()));
    for (i, (session, map)) in [("g1", "Busan"), ("g1", "Busan"), ("g2", "Ilios")]
        .into_iter()
        .enumerate()
    {
        let mut analysis = o8_analysis(Some("Ana"), None);
        analysis.own_name_candidate = Some(o9_candidate(i));
        analysis.own_seed_verdict = detect::own_row::Verdict::Disagree(88);
        assert!(o9_tab(&ctx, session, map, analysis, None).await.recorded);
    }
    assert!(own_name_seeds(ctx.own_name.as_ref()).is_empty());
    assert!(!dir.path().join("private").exists());
    let rows = ctx.store.get_session_snapshots("g1").await.unwrap();
    assert!(rows.iter().all(|r| r.suspect_fields.is_empty()));
    // Nor does it judge seeds that are there (a member who switched back
    // from the new reader): verdicts are not counted under ocr-v1.
    {
        let mut store = ctx.own_name.as_ref().unwrap().lock().unwrap();
        for (i, (session, map)) in [("s1", "Busan"), ("s1", "Busan"), ("s2", "Ilios")]
            .into_iter()
            .enumerate()
        {
            let game = detect::own_row::GameTag {
                session: session.into(),
                map: map.into(),
                at: 0,
            };
            store.offer(o9_candidate(i), &game);
        }
        assert_eq!(store.trusted().len(), 3);
    }
    for _ in 0..2 * detect::own_row::DISAGREE_DROPS {
        let mut analysis = o8_analysis(Some("Ana"), None);
        analysis.own_seed_verdict = detect::own_row::Verdict::Disagree(88);
        assert!(o9_tab(&ctx, "g3", "Busan", analysis, None).await.recorded);
    }
    assert_eq!(own_name_seeds(ctx.own_name.as_ref()).len(), 3);
}

#[tokio::test]
async fn new_reader_hero_comes_from_the_pack_read_never_the_career_panel() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    // The career panel says Reinhardt and the OCR text Ana; the heroes-v2
    // own-row read says Zenyatta and is the hero authority.
    let (rows, report) = o8_tabs(
        &ctx,
        vec![(
            o8_analysis(Some("Reinhardt"), Some("zenyatta")),
            Some(o8_board(Some("zenyatta"))),
        )],
        no_edit,
    )
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hero, "Zenyatta");
    assert!(report.career_panel, "a pack read is the authority");
    assert!(report.hero_auth.career_ever_ok);
    assert_eq!(report.hero_auth.accepted_hero.as_deref(), Some("Zenyatta"));
}

#[tokio::test]
async fn new_reader_abstain_stores_unknown_not_the_career_or_ocr_hero() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    // The own-row read abstained (suspect), so the board names no hero
    // either. The career panel and the OCR text must not stand in.
    let (rows, report) = o8_tabs(
        &ctx,
        vec![(o8_analysis(Some("Reinhardt"), None), Some(o8_board(None)))],
        no_edit,
    )
    .await;
    assert_eq!(rows[0].hero, "Unknown");
    assert!(rows[0].suspect_fields.iter().any(|f| f == "hero"));
    assert!(!report.career_panel);
    assert!(!report.hero_auth.career_ever_ok);
    assert!(report.hero_auth.accepted_hero.is_none());
}

#[tokio::test]
async fn new_reader_abstain_holds_the_hero_already_accepted() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (rows, report) = o8_tabs(
        &ctx,
        vec![
            (
                o8_analysis(None, Some("zenyatta")),
                Some(o8_board(Some("zenyatta"))),
            ),
            (o8_analysis(Some("Reinhardt"), None), Some(o8_board(None))),
        ],
        no_edit,
    )
    .await;
    let heroes: Vec<&str> = rows.iter().map(|r| r.hero.as_str()).collect();
    assert_eq!(heroes, ["Zenyatta", "Zenyatta"], "abstain holds Zenyatta");
    assert_eq!(report.hero_auth.accepted_hero.as_deref(), Some("Zenyatta"));
}

#[tokio::test]
async fn ocr_v1_hero_still_comes_from_the_career_panel() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    // ocr-v1 ignores a pack read and never reads the board.
    let (rows, report) = o8_tabs(
        &ctx,
        vec![
            (o8_analysis(Some("Reinhardt"), Some("zenyatta")), None),
            // no career read: the held career hero, as before
            (o8_analysis(None, Some("zenyatta")), None),
        ],
        no_edit,
    )
    .await;
    let heroes: Vec<&str> = rows.iter().map(|r| r.hero.as_str()).collect();
    assert_eq!(heroes, ["Reinhardt", "Reinhardt"]);
    assert!(report.hero_auth.career_ever_ok);
    // and with no career read and nothing held, the OCR text, as before
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    let (rows, _) = o8_tabs(
        &ctx,
        vec![(o8_analysis(None, Some("zenyatta")), None)],
        no_edit,
    )
    .await;
    assert_eq!(rows[0].hero, "Ana", "ocr-v1 keeps its OCR-text fallback");
}

#[tokio::test]
async fn new_reader_never_overrides_a_member_hero_pick() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let tab = || {
        (
            o8_analysis(None, Some("zenyatta")),
            Some(o8_board(Some("zenyatta"))),
        )
    };
    let (rows, _) = o8_tabs(&ctx, vec![tab(), tab()], pick_reinhardt).await;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.hero, "Zenyatta", "the read stays as read");
        assert_eq!(row.display_hero(), "Reinhardt", "the member's pick wins");
        assert_eq!(row.display_role(), "Tank");
        assert!(row.field_edited("hero"));
        assert!(!row.suspect_fields.iter().any(|f| f == "hero"));
    }
}

#[tokio::test]
async fn ocr_v1_rows_after_a_hero_pick_are_written_as_before() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    let tab = || (o8_analysis(Some("Zenyatta"), None), None);
    let (rows, _) = o8_tabs(&ctx, vec![tab(), tab()], pick_reinhardt).await;
    assert_eq!(rows[0].display_hero(), "Reinhardt");
    // ocr-v1 is unchanged: a later row carries no overlay of its own
    assert_eq!(rows[1].corrected_hero, None);
}

#[test]
fn read_own_hero_reads_only_the_switched_source() {
    let tab = |hero: Option<&str>| shadow::TabHeroes {
        team_size: 5,
        read: Some(shadow::heroes::HeroFrameRead {
            board: shadow::heroes::HeroBoard {
                rows: (0..10)
                    .map(|row| shadow::heroes::HeroRead {
                        row,
                        class: if hero.is_some() {
                            shadow::heroes::RowClass::Hero
                        } else {
                            shadow::heroes::RowClass::Unknown
                        },
                        hero: hero.map(str::to_string),
                        best: hero.unwrap_or("ana").to_string(),
                        score: if hero.is_some() { 0.9 } else { 0.5 },
                        lead: 0.2,
                        second: "mercy".into(),
                        suspect: hero.is_none(),
                    })
                    .collect(),
                elapsed_us: 1,
            },
            pack: shadow::heroes::HeroPack::V2,
            pad_px: None,
        }),
        panel: None,
    };
    // ocr-v1: the career panel, and the icons are never read. (A panic
    // in the closure would be caught and look like "not read", so the
    // closure counts its calls instead.)
    let reads = std::cell::Cell::new(0);
    let icons = read_tab_heroes(false, || {
        reads.set(reads.get() + 1);
        tab(Some("zenyatta"))
    });
    assert_eq!(icons, None);
    assert_eq!(reads.get(), 0, "ocr-v1 must not read the hero icons");
    let v1 = read_own_hero(false, || Some("Reinhardt".into()), icons, Some(0));
    assert_eq!(v1.career_hero.as_deref(), Some("Reinhardt"));
    assert_eq!((v1.pack_hero, v1.tab_heroes), (None, None));
    // ocr-v1 ignores an icon read even if one were handed to it
    let v1 = read_own_hero(false, || None, Some(tab(Some("zenyatta"))), Some(0));
    assert_eq!((v1.pack_hero, v1.tab_heroes), (None, None));
    // new: the icons, and the career panel is never read
    let career = || -> Option<String> { panic!("the new reader must not read the career panel") };
    let icons = read_tab_heroes(true, || {
        reads.set(reads.get() + 1);
        tab(Some("zenyatta"))
    });
    assert!(icons.is_some());
    assert_eq!(reads.get(), 1, "one icon read per Tab");
    let new = read_own_hero(true, career, icons, Some(3));
    assert_eq!(new.career_hero, None);
    assert_eq!(new.pack_hero.as_deref(), Some("zenyatta"));
    assert!(new.tab_heroes.is_some(), "kept for the board read");
    // an abstaining own row, an unknown own row, a row off the board
    let career = || -> Option<String> { panic!("no career read") };
    assert_eq!(
        read_own_hero(true, career, Some(tab(None)), Some(3)).pack_hero,
        None
    );
    let career = || -> Option<String> { panic!("no career read") };
    assert_eq!(
        read_own_hero(true, career, Some(tab(Some("zenyatta"))), None).pack_hero,
        None
    );
    let career = || -> Option<String> { panic!("no career read") };
    assert_eq!(
        read_own_hero(true, career, Some(tab(Some("zenyatta"))), Some(10)).pack_hero,
        None
    );
    // the career panel's portrait authenticates when the row abstains
    let panel = || {
        let mut t = tab(None);
        t.panel = Some(shadow::heroes::PanelRead {
            read: shadow::heroes::HeroRead {
                row: 0,
                class: shadow::heroes::RowClass::Hero,
                hero: Some("ana".into()),
                best: "ana".into(),
                score: 0.97,
                lead: 0.4,
                second: "mercy".into(),
                suspect: false,
            },
            pack: shadow::heroes::HeroPack::V2,
            box_px: (0, 0, 1, 1),
            rel: (0.03, -0.03, 0.16),
        });
        t
    };
    let career = || -> Option<String> { panic!("no career read") };
    assert_eq!(
        read_own_hero(true, career, Some(panel()), Some(3))
            .pack_hero
            .as_deref(),
        Some("ana")
    );
    // a panic in the icon reader is an abstain, not a failed Tab: no
    // icon read, so no hero authority and no career hero's row
    let career = || -> Option<String> { panic!("no career read") };
    let icons = read_tab_heroes(true, || panic!("icon reader"));
    assert_eq!(icons, None);
    let panicked = read_own_hero(true, career, icons, Some(0));
    assert_eq!((panicked.pack_hero, panicked.tab_heroes), (None, None));
}
