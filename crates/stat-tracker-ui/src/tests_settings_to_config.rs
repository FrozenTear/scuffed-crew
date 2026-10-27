//! Guard for [`SettingsForm::to_config`]: every [`Config`] key is either one
//! the form owns or one it keeps from the file it saves over.
//!
//! `to_config` ends in `..base.clone()`, so a key added to `Config` is kept
//! from the file by default. That is right for a key the 0.6 page sets
//! through `ConfigEdit`, and wrong for one the form shows: the form would
//! drop the member's edit on save. The destructure below names every field
//! with no `..`, so a new field stops this test from compiling until it is
//! put in one group. A key the form owns also needs its line in `to_config`.

use std::path::PathBuf;

use stat_tracker::config::{
    AutoDetectConfig, CaptureRoute, Config, OcrSetting, ReaderSetting, ShadowRecognizerControl,
    SyncConfig,
};

use crate::settings::SettingsForm;

/// A file with every key off its default, so a key taken from the wrong
/// side shows up as a default value.
fn every_key_set() -> Config {
    Config {
        data_dir: PathBuf::from("/guard/data"),
        capture_output: Some("DP-1".to_string()),
        capture_route: CaptureRoute::Kwin,
        debug_portal_frames: true,
        screencast_max_fps: Some(2),
        screencast_pause_between_grabs: Some(false),
        player_name: Some("FrozenTear".to_string()),
        sync: Some(SyncConfig {
            server_url: "https://crew.example".to_string(),
            token: "not-a-real-token".to_string(),
        }),
        auto_detect: AutoDetectConfig {
            enabled: false,
            poll_interval_secs: Some(8),
            poll_interval_ms: Some(750),
            cooldown_secs: 30,
        },
        session_window_secs: 900,
        finished_game_close_secs: 240,
        game_process_names: vec!["Overwatch.exe".to_string(), "wine64".to_string()],
        debug_ocr: true,
        report_kit_max_games: Some(80),
        report_kit_max_mb: Some(1000),
        ocr_threads: Some(2),
        shadow_recognizer: true,
        update_channel: "prerelease".to_string(),
        setup_completed: true,
        reader_pack_url: Some("https://crew.example/packs/reader.zip".to_string()),
        reader: ReaderSetting::OcrV1,
        ocr: OcrSetting::Off,
        name_reader: false,
        player_names: vec!["FrozenAlt".to_string()],
        live_upload: false,
        rank_reader: true,
        share_stat_pictures: true,
        share_report_pictures: true,
        capture_only_on_screen: false,
    }
}

/// The form is loaded from the defaults and saved over [`every_key_set`]:
/// each key the form owns comes out at its default, each key it keeps comes
/// out as the file had it.
#[test]
fn every_config_key_is_owned_by_the_form_or_kept_from_the_file() {
    let defaults = Config::default();
    let file = every_key_set();
    let control = ShadowRecognizerControl {
        file_on: false,
        locked: false,
    };
    let form = SettingsForm::from_config_and_shadow(&defaults, control);
    let Config {
        // Owned by the form: the form's value wins.
        capture_output,
        capture_route,
        player_name,
        sync,
        auto_detect,
        session_window_secs,
        game_process_names,
        debug_ocr,
        shadow_recognizer,
        reader,
        ocr,
        // Not on the form: kept from the file. `update_channel` is kept
        // while its checkbox still shows what it showed at load.
        data_dir,
        debug_portal_frames,
        screencast_max_fps,
        screencast_pause_between_grabs,
        finished_game_close_secs,
        report_kit_max_games,
        report_kit_max_mb,
        ocr_threads,
        update_channel,
        setup_completed,
        reader_pack_url,
        name_reader,
        player_names,
        live_upload,
        rank_reader,
        share_stat_pictures,
        share_report_pictures,
        capture_only_on_screen,
    } = form.to_config(&file);

    assert_eq!(capture_output, defaults.capture_output);
    assert_eq!(capture_route, defaults.capture_route);
    assert_eq!(player_name, defaults.player_name);
    assert_eq!(sync, defaults.sync);
    assert_eq!(auto_detect.enabled, defaults.auto_detect.enabled);
    assert_eq!(
        auto_detect.cooldown_secs,
        defaults.auto_detect.cooldown_secs
    );
    assert_ne!(
        auto_detect.poll_interval_secs,
        Some(8),
        "the form's poll wins"
    );
    assert_eq!(
        auto_detect.poll_interval_ms, file.auto_detect.poll_interval_ms,
        "the millisecond poll is not on the form"
    );
    assert_eq!(session_window_secs, defaults.session_window_secs);
    assert_eq!(game_process_names, defaults.game_process_names);
    assert_eq!(debug_ocr, defaults.debug_ocr);
    assert_eq!(shadow_recognizer, defaults.shadow_recognizer);
    assert_eq!(reader, defaults.reader);
    assert_eq!(ocr, defaults.ocr);

    assert_eq!(data_dir, file.data_dir);
    assert_eq!(debug_portal_frames, file.debug_portal_frames);
    assert_eq!(screencast_max_fps, file.screencast_max_fps);
    assert_eq!(
        screencast_pause_between_grabs,
        file.screencast_pause_between_grabs
    );
    assert_eq!(finished_game_close_secs, file.finished_game_close_secs);
    assert_eq!(report_kit_max_games, file.report_kit_max_games);
    assert_eq!(report_kit_max_mb, file.report_kit_max_mb);
    assert_eq!(ocr_threads, file.ocr_threads);
    assert_eq!(update_channel, file.update_channel);
    assert_eq!(setup_completed, file.setup_completed);
    assert_eq!(reader_pack_url, file.reader_pack_url);
    assert_eq!(name_reader, file.name_reader);
    assert_eq!(player_names, file.player_names);
    assert_eq!(live_upload, file.live_upload);
    assert_eq!(rank_reader, file.rank_reader);
    assert_eq!(share_stat_pictures, file.share_stat_pictures);
    assert_eq!(share_report_pictures, file.share_report_pictures);
    assert_eq!(capture_only_on_screen, file.capture_only_on_screen);
}
