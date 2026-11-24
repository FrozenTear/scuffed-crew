/// Shape Settings writes: top-level keys, then `[auto_detect]` and `[sync]`.
pub(super) fn settings_shaped_without_shadow() -> String {
    "\
data_dir = \"/tmp/sst-settings-shape\"
capture_output = \"DP-1\"
player_name = \"FrozenTear\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = \"https://crew.example\"
token = \"not-a-real-token\"
"
    .to_string()
}
