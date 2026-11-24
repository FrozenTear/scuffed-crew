//! The portrait directory stays empty of images, and every image under
//! `test-data/` is named in `test-data/image-allowlist.txt`.

#[test]
fn stat_tracker_ships_no_game_art() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let status = std::process::Command::new("bash")
        .arg(root.join("tests/check-no-game-art.sh"))
        .status()
        .expect("run check-no-game-art.sh");
    assert!(status.success(), "game-art check failed");
}
