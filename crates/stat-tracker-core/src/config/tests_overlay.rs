use super::*;

#[test]
fn ocr_threads_resolved_clamps_and_auto() {
    let mut c = Config::default();
    assert!(
        (2..=4).contains(&c.ocr_threads_resolved()),
        "auto should stay in the historical 2..=4 band"
    );
    c.ocr_threads = Some(1);
    assert_eq!(c.ocr_threads_resolved(), 1);
    c.ocr_threads = Some(3);
    assert_eq!(c.ocr_threads_resolved(), 3);
    c.ocr_threads = Some(99);
    assert_eq!(c.ocr_threads_resolved(), 8);
    c.ocr_threads = Some(0);
    assert_eq!(c.ocr_threads_resolved(), 1);
}

#[test]
fn shadow_env_off_values_keep_it_off() {
    for v in [
        None,
        Some("0"),
        Some("false"),
        Some("FALSE"),
        Some("no"),
        Some(""),
    ] {
        let control = Config::shadow_control(false, v);
        assert!(!control.enabled(), "{v:?} must not turn it on");
        assert!(!control.locked, "{v:?} must not lock Settings");
        assert!(!control.file_on);
    }
    for v in [
        Some("1"),
        Some("true"),
        Some("TRUE"),
        Some("yes"),
        Some("YES"),
    ] {
        let control = Config::shadow_control(false, v);
        assert!(control.locked, "{v:?} locks the toggle on");
        assert!(control.enabled(), "{v:?} turns it on for this process");
        assert!(!control.file_on, "{v:?} must not become the file value");
    }
    let control = Config::shadow_control(true, Some("0"));
    assert!(control.file_on);
    assert!(!control.locked);
    assert!(control.enabled(), "file flag still wins");
}

#[test]
fn shadow_env_override_is_never_saved() {
    // The override is resolved at use, not stored: a config built from the
    // file value (what Settings saves) serializes without the key even
    // when the env would turn the recognizer on.
    let cfg = Config::default();
    let control = Config::shadow_control(cfg.shadow_recognizer, Some("1"));
    assert!(control.enabled());
    assert!(control.locked);
    assert!(!control.file_on);
    assert!(!cfg.shadow_recognizer);
    let raw = toml::to_string_pretty(&cfg).unwrap();
    assert!(!raw.contains("shadow_recognizer"), "{raw}");
    // Config::load must not fold the env var into the struct.
    let src = include_str!("../config.rs");
    let load = &src[src.find("pub fn load").expect("load")..];
    let load = &load[..load.find("pub fn config_path").expect("end of load")];
    assert!(
        !load.contains(concat!("config.shadow_", "recognizer =")),
        "load() must not persist SCUFFED_SHADOW_RECOGNIZER into the config"
    );
}

#[test]
fn report_kit_retention_defaults_to_50_games_and_500_mb() {
    let cfg: Config = toml::from_str("data_dir = \"/tmp/x\"").unwrap();
    assert_eq!(cfg.report_kit_retention(), Retention::new(50, 500));
    let cfg: Config =
        toml::from_str("data_dir = \"/tmp/x\"\nreport_kit_max_games = 20\nreport_kit_max_mb = 100")
            .unwrap();
    assert_eq!(cfg.report_kit_retention(), Retention::new(20, 100));
}

#[test]
fn capture_env_note_names_the_environment_d_file_and_not_session_env() {
    let files = vec![
        ("a.conf".into(), "STAT_TRACKER_CAPTURE=wayshot\n".into()),
        (
            "scuffed-capture.conf".into(),
            "export STAT_TRACKER_CAPTURE=\"portal\"\n".into(),
        ),
    ];
    let note = Config::capture_env_note(Some("portal"), &files).unwrap();
    assert!(note.contains("scuffed-capture.conf"), "{note}");
    assert!(note.contains("STAT_TRACKER_CAPTURE=portal"), "{note}");
    assert!(!note.contains("session.env"), "{note}");
    let unloaded = Config::capture_env_note(None, &files).unwrap();
    assert!(unloaded.contains("has not loaded"), "{unloaded}");
    assert!(unloaded.contains("scuffed-capture.conf"), "{unloaded}");
    assert!(Config::capture_env_note(None, &[]).is_none());
    assert!(Config::capture_env_note(Some("  "), &[]).is_none());
}

#[test]
fn ocr_env_overrides_the_file_and_ignores_other_values() {
    let file_on = |env| Config::ocr_control_from(OcrSetting::On, env);
    assert_eq!(file_on(None).requested(), OcrSetting::On);
    assert_eq!(file_on(Some("off")).requested(), OcrSetting::Off);
    assert_eq!(file_on(Some(" OFF\n")).requested(), OcrSetting::Off);
    for ignored in ["", "0", "false", "no", "offf"] {
        let control = file_on(Some(ignored));
        assert_eq!(control.env, None, "{ignored:?} is not an override");
        assert_eq!(control.requested(), OcrSetting::On);
    }
    let file_off = Config::ocr_control_from(OcrSetting::Off, Some("on"));
    assert_eq!(
        file_off.file,
        OcrSetting::Off,
        "Settings still sees the file"
    );
    assert_eq!(file_off.requested(), OcrSetting::On);
    assert_eq!(
        Config::ocr_control_from(OcrSetting::Off, None).requested(),
        OcrSetting::Off
    );
}

#[test]
fn poll_interval_is_faster_by_default_only_with_the_new_reader() {
    use std::time::Duration;
    let auto = AutoDetectConfig::default();
    assert_eq!(auto.poll_interval(true), Duration::from_millis(1500));
    assert_eq!(auto.poll_interval(false), Duration::from_secs(4));
    // 4 is what older Settings wrote: still automatic.
    let legacy = AutoDetectConfig {
        poll_interval_secs: Some(4),
        ..auto
    };
    assert_eq!(legacy.poll_interval(true), Duration::from_millis(1500));
    assert_eq!(legacy.poll_interval(false), Duration::from_secs(4));
    // Any other value overrides both readers.
    let set = AutoDetectConfig {
        poll_interval_secs: Some(2),
        ..auto
    };
    assert_eq!(set.poll_interval(true), Duration::from_secs(2));
    assert_eq!(set.poll_interval(false), Duration::from_secs(2));
    let zero = AutoDetectConfig {
        poll_interval_secs: Some(0),
        ..auto
    };
    assert_eq!(zero.poll_interval(false), Duration::from_secs(1));
    // A config without the key parses as automatic.
    let parsed: AutoDetectConfig = toml::from_str("enabled = true\ncooldown_secs = 120\n").unwrap();
    assert_eq!(parsed.poll_interval_secs, None);
    assert_eq!(parsed.poll_interval(true), Duration::from_millis(1500));
}

#[test]
fn poll_interval_ms_overrides_seconds_and_automatic_and_clamps() {
    use std::time::Duration;
    let auto = AutoDetectConfig::default();
    let ms = AutoDetectConfig {
        poll_interval_ms: Some(750),
        ..auto
    };
    assert_eq!(ms.poll_interval(true), Duration::from_millis(750));
    assert_eq!(ms.poll_interval(false), Duration::from_millis(750));
    assert_eq!(ms.poll_interval_source(), "poll_interval_ms");
    let with_secs = AutoDetectConfig {
        poll_interval_secs: Some(2),
        poll_interval_ms: Some(800),
        ..auto
    };
    assert_eq!(with_secs.poll_interval(true), Duration::from_millis(800));
    assert_eq!(with_secs.poll_interval(false), Duration::from_millis(800));
    let low = AutoDetectConfig {
        poll_interval_ms: Some(1),
        ..auto
    };
    assert_eq!(low.poll_interval(true), Duration::from_millis(500));
    let high = AutoDetectConfig {
        poll_interval_ms: Some(99_000),
        ..auto
    };
    assert_eq!(high.poll_interval(false), Duration::from_millis(10_000));
    let raw = toml::to_string(&auto).unwrap();
    assert!(raw.contains("poll_interval_secs = 4"), "{raw}");
    assert!(!raw.contains("poll_interval_ms"), "{raw}");
    let set_raw = toml::to_string(&ms).unwrap();
    assert!(set_raw.contains("poll_interval_ms = 750"), "{set_raw}");
    assert!(set_raw.contains("poll_interval_secs = 4"), "{set_raw}");
    let back: AutoDetectConfig = toml::from_str(&set_raw).unwrap();
    assert_eq!(back.poll_interval_ms, Some(750));
    assert_eq!(back.poll_interval(true), Duration::from_millis(750));
    assert_eq!(auto.poll_interval_source(), "automatic");
    let secs = AutoDetectConfig {
        poll_interval_secs: Some(2),
        ..auto
    };
    assert_eq!(secs.poll_interval_source(), "poll_interval_secs");
}

#[test]
fn screencast_max_fps_defaults_to_four_and_clamps() {
    // rc.6: 2 fps put a Tab's first usable frame up to 500 ms after the
    // overlay delay, and a short tap closed the board before it. The
    // default is now 4. A member can still set 2.
    let cfg = Config::default();
    assert_eq!(cfg.screencast_max_fps(), 4);
    let mut two = cfg.clone();
    two.screencast_max_fps = Some(2);
    assert_eq!(two.screencast_max_fps(), 2);
    assert!(
        !toml::to_string(&cfg)
            .unwrap()
            .contains("screencast_max_fps")
    );
    let mut four = cfg.clone();
    four.screencast_max_fps = Some(4);
    assert_eq!(four.screencast_max_fps(), 4);
    assert!(
        toml::to_string(&four)
            .unwrap()
            .contains("screencast_max_fps = 4")
    );
    four.screencast_max_fps = Some(0);
    assert_eq!(four.screencast_max_fps(), 1);
    four.screencast_max_fps = Some(9);
    assert_eq!(four.screencast_max_fps(), 5);
    let raw = toml::to_string(&Config::default()).unwrap();
    // The key has to sit before any table. A line appended after
    // `[auto_detect]` would belong to that table and be ignored.
    let raw = format!("screencast_max_fps = 4\n{raw}");
    let parsed: Config = toml::from_str(&raw).unwrap();
    assert_eq!(parsed.screencast_max_fps(), 4);
}

#[test]
fn screencast_pause_is_auto_unless_config_or_env_says_otherwise() {
    use PauseOverride::*;
    let cfg = Config::default();
    assert_eq!(cfg.screencast_pause_between_grabs, None);
    assert_eq!(screencast_pause_override(None, None), Auto);
    assert_eq!(screencast_pause_override(Some(true), None), Auto);
    assert_eq!(screencast_pause_override(Some(false), None), ConfigOff);
    assert!(
        !toml::to_string(&cfg)
            .unwrap()
            .contains("screencast_pause_between_grabs"),
        "not written unless set"
    );
    for off in ["0", "false", "OFF", " no "] {
        assert_eq!(screencast_pause_override(None, Some(off)), EnvOff, "{off}");
        assert_eq!(screencast_pause_override(Some(true), Some(off)), EnvOff);
    }
    for on in ["1", "true", "On", "YES"] {
        assert_eq!(
            screencast_pause_override(Some(false), Some(on)),
            EnvOn,
            "{on}"
        );
        assert_eq!(screencast_pause_override(None, Some(on)), EnvOn);
    }
    // Empty or unknown leaves the file in charge.
    assert_eq!(screencast_pause_override(Some(false), Some("")), ConfigOff);
    assert_eq!(
        screencast_pause_override(Some(false), Some("maybe")),
        ConfigOff
    );
    assert_eq!(screencast_pause_override(None, Some("")), Auto);
    let raw = toml::to_string(&Config::default()).unwrap();
    let raw = format!("screencast_pause_between_grabs = false\n{raw}");
    let parsed: Config = toml::from_str(&raw).unwrap();
    assert_eq!(parsed.screencast_pause_between_grabs, Some(false));
    assert!(
        toml::to_string(&parsed)
            .unwrap()
            .contains("screencast_pause_between_grabs = false")
    );
}
