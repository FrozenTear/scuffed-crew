use super::tests::settings_shaped_without_shadow;
use super::*;

#[test]
fn name_reader_defaults_on_and_stays_independent_of_the_cv_reader() {
    assert!(Config::default().name_reader);
    let raw = toml::to_string_pretty(&Config::default()).unwrap();
    assert!(!raw.contains("name_reader"), "{raw}");
    let ocr_v1: Config = toml::from_str("data_dir = \"/tmp/x\"\nreader = \"ocr-v1\"\n").unwrap();
    assert!(ocr_v1.name_reader, "ocr-v1 still reads names with the pack");
    assert!(ocr_v1.reader.is_ocr_v1());
    let off: Config =
        toml::from_str("data_dir = \"/tmp/x\"\nreader = \"ocr-v1\"\nname_reader = false\n")
            .unwrap();
    assert!(!off.name_reader);
    assert!(off.reader.is_ocr_v1());
    let saved = toml::to_string_pretty(&off).unwrap();
    assert!(saved.contains("name_reader = false"), "{saved}");
    let on_again: Config = toml::from_str("data_dir = \"/tmp/x\"\nname_reader = true\n").unwrap();
    assert!(on_again.name_reader);
    assert!(
        on_again.reader.is_new(),
        "the cv reader stays on its own default"
    );
}

#[test]
fn shadow_recognizer_defaults_off_and_is_not_written_when_off() {
    assert!(!Config::default().shadow_recognizer);
    let raw = toml::to_string_pretty(&Config::default()).unwrap();
    assert!(!raw.contains("shadow_recognizer"), "{raw}");
    let on: Config = toml::from_str(&format!("shadow_recognizer = true\n{raw}")).unwrap();
    assert!(on.shadow_recognizer);
}

#[test]
fn a_config_with_the_unused_reader_pack_url_still_parses_and_keeps_it() {
    let raw = r#"
data_dir = "/tmp/sst-p1"
setup_completed = true
reader_pack_url = "https://crew.example/packs/reader.zip"
"#;
    let cfg = Config::parse_file_contents(raw).expect("reader_pack_url must still parse");
    assert_eq!(cfg.data_dir, PathBuf::from("/tmp/sst-p1"));
    assert!(cfg.setup_completed);
    assert_eq!(
        cfg.reader_pack_url.as_deref(),
        Some("https://crew.example/packs/reader.zip")
    );
    let saved = toml::to_string_pretty(&cfg).expect("serialize");
    assert!(
        saved.contains("reader_pack_url = \"https://crew.example/packs/reader.zip\""),
        "{saved}"
    );
    let blank = Config::parse_file_contents("data_dir = \"/tmp/sst-p1\"\nreader_pack_url = \"\"\n")
        .expect("blank reader_pack_url must still parse");
    assert_eq!(blank.reader_pack_url.as_deref(), Some(""));
}

fn sample(full: bool) -> Config {
    Config {
        data_dir: PathBuf::from("/var/lib/scuffed-outside"),
        capture_output: full.then(|| "DP-1".to_string()),
        capture_route: if full {
            CaptureRoute::Screencast
        } else {
            CaptureRoute::Auto
        },
        debug_portal_frames: full,
        screencast_max_fps: full.then_some(4),
        screencast_pause_between_grabs: full.then_some(false),
        player_name: full.then(|| "Ada".to_string()),
        sync: full.then(|| SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "super-secret-token".to_string(),
        }),
        auto_detect: AutoDetectConfig {
            enabled: false,
            poll_interval_secs: Some(8),
            poll_interval_ms: full.then_some(750),
            cooldown_secs: 30,
        },
        session_window_secs: 900,
        finished_game_close_secs: 240,
        game_process_names: if full {
            vec!["Overwatch.exe".into(), "wine".into()]
        } else {
            Vec::new()
        },
        debug_ocr: full,
        debug_save_pregame_screens: full,
        report_kit_max_games: full.then_some(80),
        report_kit_max_mb: full.then_some(1000),
        ocr_threads: full.then_some(2),
        shadow_recognizer: full,
        update_channel: String::new(),
        setup_completed: full,
        reader_pack_url: full.then(|| "https://crew.example/packs/reader.zip".to_string()),
        reader: if full {
            ReaderSetting::New
        } else {
            ReaderSetting::OcrV1
        },
        ocr: if full {
            OcrSetting::Off
        } else {
            OcrSetting::On
        },
        // On is the default and is left out of the file. The full sample
        // sets the explicit off so the round trip keeps that line.
        name_reader: !full,
        player_names: if full {
            vec!["FrozenAlt".into()]
        } else {
            Vec::new()
        },
        live_upload: !full,
        rank_reader: full,
        share_stat_pictures: full,
        share_report_pictures: full,
        capture_only_on_screen: !full,
    }
}

#[test]
fn toml_save_format_roundtrips_optional_fields_set_and_unset() {
    for full in [true, false] {
        let cfg = sample(full);
        let raw = toml::to_string_pretty(&cfg).expect("toml 1 must serialize Config");
        if full {
            assert!(raw.contains("capture_output"), "{raw}");
            assert!(raw.contains("player_name"), "{raw}");
            assert!(raw.contains("[sync]"), "{raw}");
            assert!(raw.contains("ocr_threads"), "{raw}");
            assert!(raw.contains("name_reader = false"), "{raw}");
        } else {
            assert!(!raw.contains("capture_output"), "{raw}");
            assert!(!raw.contains("player_name"), "{raw}");
            assert!(!raw.contains("[sync]"), "{raw}");
            assert!(!raw.contains("ocr_threads"), "{raw}");
            assert!(!raw.contains("name_reader"), "{raw}");
        }
        let back: Config = toml::from_str(&raw).expect("toml 1 output must parse");
        assert_eq!(back, cfg);
    }
}

#[test]
fn reader_defaults_to_new_and_rejects_unknown_values() {
    assert_eq!(Config::default().reader, ReaderSetting::New);
    assert!(Config::default().uses_new_reader());
    assert!(Config::default().shadow_log_enabled());
    let raw = settings_shaped_without_shadow();
    let loaded = Config::parse_file_contents(&raw).expect("missing reader");
    assert!(
        loaded.uses_new_reader(),
        "no reader line means the new reader"
    );
    let with_new = format!("reader = \"new\"\n{raw}");
    let on = Config::parse_file_contents(&with_new).expect("reader = new");
    assert!(on.uses_new_reader());
    assert!(on.shadow_log_enabled());
    assert!(!on.shadow_recognizer);
    let with_old = format!("reader = \"ocr-v1\"\n{raw}");
    let old = Config::parse_file_contents(&with_old).expect("reader = ocr-v1");
    assert!(old.reader.is_ocr_v1(), "an explicit ocr-v1 stays");
    assert!(!old.uses_new_reader());
    assert!(!old.shadow_log_enabled());
    let unknown = format!("reader = \"cv-v5\"\n{raw}");
    assert!(Config::parse_file_contents(&unknown).is_err());
}

/// A full save (not a one-key patch) leaves the default reader out and
/// writes an explicit ocr-v1, so neither changes on the next load.
#[test]
fn a_full_save_keeps_an_explicit_ocr_v1_and_leaves_the_default_out() {
    let mut cfg = Config::default();
    let text = toml::to_string_pretty(&cfg).unwrap();
    assert!(!text.lines().any(|l| l.starts_with("reader")), "{text}");
    assert!(
        Config::parse_file_contents(&text)
            .unwrap()
            .uses_new_reader()
    );
    cfg.reader = ReaderSetting::OcrV1;
    let text = toml::to_string_pretty(&cfg).unwrap();
    assert!(text.contains("reader = \"ocr-v1\""), "{text}");
    assert!(
        Config::parse_file_contents(&text)
            .unwrap()
            .reader
            .is_ocr_v1()
    );
}

#[test]
fn ocr_defaults_to_on_and_takes_only_on_or_off() {
    assert_eq!(Config::default().ocr, OcrSetting::On);
    let has_ocr_key = |text: &str| text.lines().any(|line| line.starts_with("ocr "));
    let raw = settings_shaped_without_shadow();
    assert!(!has_ocr_key(&raw), "seed has no ocr key");
    let loaded = Config::parse_file_contents(&raw).expect("missing ocr");
    assert_eq!(loaded.ocr, OcrSetting::On);
    // a config with OCR on does not grow the key; off writes it
    assert!(!has_ocr_key(&toml::to_string_pretty(&loaded).unwrap()));
    let mut written_off = loaded.clone();
    written_off.ocr = OcrSetting::Off;
    assert!(has_ocr_key(&toml::to_string_pretty(&written_off).unwrap()));
    let off = Config::parse_file_contents(&format!("ocr = \"off\"\n{raw}")).expect("off");
    assert_eq!(off.ocr, OcrSetting::Off);
    assert!(off.ocr.is_off());
    let on = Config::parse_file_contents(&format!("ocr = \"on\"\n{raw}")).expect("on");
    assert_eq!(on.ocr, OcrSetting::On);
    // the file value reads like the env value: any case, trimmed
    for spelled in ["\"Off\"", "\"OFF\"", "\" off \""] {
        let loaded = Config::parse_file_contents(&format!("ocr = {spelled}\n{raw}"))
            .unwrap_or_else(|e| panic!("ocr = {spelled}: {e}"));
        assert_eq!(loaded.ocr, OcrSetting::Off, "{spelled}");
    }
    for bad in ["\"no\"", "\"\"", "\"offf\"", "false", "0"] {
        assert!(
            Config::parse_file_contents(&format!("ocr = {bad}\n{raw}")).is_err(),
            "ocr = {bad} must not parse"
        );
    }
    // `reader` is its own key: turning OCR off does not rewrite it.
    assert_eq!(off.reader, ReaderSetting::New, "missing reader is new");
    let off_old =
        Config::parse_file_contents(&format!("ocr = \"off\"\nreader = \"ocr-v1\"\n{raw}"))
            .expect("off with ocr-v1");
    assert!(off_old.reader.is_ocr_v1());
}

#[test]
fn automatic_poll_interval_is_written_as_4_for_older_versions() {
    let auto = AutoDetectConfig::default();
    assert_eq!(auto.poll_interval_secs, None);
    let raw = toml::to_string(&auto).unwrap();
    assert!(raw.contains("poll_interval_secs = 4"), "{raw}");
    let back: AutoDetectConfig = toml::from_str(&raw).unwrap();
    assert_eq!(back, auto);
    let fixed = AutoDetectConfig {
        poll_interval_secs: Some(5),
        ..auto
    };
    assert!(
        toml::to_string(&fixed)
            .unwrap()
            .contains("poll_interval_secs = 5")
    );
}

#[test]
fn capture_route_accepts_case_and_the_env_names() {
    assert_eq!(
        CaptureRoute::parse("Screencast"),
        Some(CaptureRoute::Screencast)
    );
    assert_eq!(
        CaptureRoute::parse("portal"),
        Some(CaptureRoute::Screencast)
    );
    assert_eq!(
        CaptureRoute::parse("portal-screenshot"),
        Some(CaptureRoute::Screenshot)
    );
    assert_eq!(CaptureRoute::parse("KWin"), Some(CaptureRoute::Kwin));
    assert_eq!(CaptureRoute::parse("kde-direct"), Some(CaptureRoute::Kwin));
    assert_eq!(CaptureRoute::Kwin.as_str(), "kwin");
    assert_eq!(CaptureRoute::parse("nope"), None);
    #[derive(Deserialize)]
    struct Wrap {
        capture_route: CaptureRoute,
    }
    let parsed: Wrap = toml::from_str("capture_route = \"Screencast\"\n").unwrap();
    assert_eq!(parsed.capture_route, CaptureRoute::Screencast);
    let unknown: Wrap = toml::from_str("capture_route = \"nope\"\n").unwrap();
    assert_eq!(unknown.capture_route, CaptureRoute::Auto);
    let kwin: Wrap = toml::from_str("capture_route = \"kwin\"\n").unwrap();
    assert_eq!(kwin.capture_route, CaptureRoute::Kwin);
}
