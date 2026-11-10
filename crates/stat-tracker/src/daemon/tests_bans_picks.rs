//! Tests for `daemon::bans_picks`, kept out of line so the source-text
//! helper `daemon_source()` still sees the production code after this file.

use std::sync::Mutex;

use image::Rgb;

use super::*;

/// Records the rings the ban results are read with.
struct Spy(Mutex<Vec<Vec<[u8; 3]>>>);

impl Engine for Spy {
    fn vote(&self, _: &RgbImage) -> Option<BanVote> {
        Some(BanVote {
            slots: None,
            score: None,
        })
    }
    fn select(&self, _: &RgbImage) -> Option<HeroSelect> {
        Some(HeroSelect {
            picks: Vec::new(),
            ring: [1, 2, 3],
        })
    }
    fn bans(&self, _: &RgbImage, rings: &[[u8; 3]]) -> Option<BanResults> {
        self.0.lock().unwrap().push(rings.to_vec());
        Some(BanResults {
            layout: Layout::Plain,
            bans: Vec::new(),
            lobby_ban: None,
        })
    }
}

/// A frame with a team ring of colour `c` where the game draws it.
fn ringed(w: u32, h: u32, c: [u8; 3]) -> Arc<DynamicImage> {
    let mut img = RgbImage::new(w, h);
    for y in 430..475.min(h) {
        for x in 64..70.min(w) {
            img.put_pixel(x, y, Rgb(c));
        }
    }
    Arc::new(DynamicImage::ImageRgb8(img))
}

#[test]
fn the_ban_vote_ring_is_given_to_the_ban_results_as_a_second_ring() {
    let spy = Spy(Mutex::new(Vec::new()));
    let frames = Frames {
        vote: Some(ringed(FRAME_W, FRAME_H, [128, 0, 128])),
        results: Some(ringed(FRAME_W, FRAME_H, [0, 0, 0])),
        ..Frames::default()
    };
    assert!(read_with(&spy, &frames).is_some());
    // no select frame: the vote ring is the only one
    assert_eq!(spy.0.lock().unwrap()[0], vec![[128, 0, 128]]);
    // with a select frame its ring comes first
    let frames = Frames {
        select: Some(ringed(FRAME_W, FRAME_H, [0, 0, 0])),
        ..frames
    };
    assert!(read_with(&spy, &frames).is_some());
    assert_eq!(spy.0.lock().unwrap()[1], vec![[1, 2, 3], [128, 0, 128]]);
    // a vote frame of another size gives no ring
    let frames = Frames {
        vote: Some(ringed(8, 8, [128, 0, 128])),
        ..frames
    };
    assert!(read_with(&spy, &frames).is_some());
    assert_eq!(spy.0.lock().unwrap()[2], vec![[1, 2, 3]]);
}

/// A config.toml as alpha.2.3 saved it with Read bans and picks off. The
/// switch moved to Settings, Capture and reading; its key did not.
const ALPHA_2_3_READER_OFF: &str = "\
data_dir = \"/home/member/.local/share/scuffed-stat-tracker\"
capture_route = \"auto\"
player_name = \"Member\"
setup_completed = true
bans_picks_reader = false

[auto_detect]
enabled = true
";

/// The tracker keeps an alpha.2.3 file's reader off, and
/// STAT_TRACKER_BANS_PICKS_RECOGNIZER=off still wins over a saved on.
#[test]
fn the_saved_key_and_the_env_override_still_gate_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    let old = stat_tracker::config::Config::parse_file_contents(ALPHA_2_3_READER_OFF).unwrap();
    assert!(!old.bans_picks_reader);
    let feed = |saved: bool, env: &str| {
        BansPicksFeed::for_config(saved, BansPicksSetting::parse(env), dir.path(), false)
    };
    assert!(
        feed(old.bans_picks_reader, "cv-v1").is_none(),
        "alpha.2.3 off stays off"
    );
    assert!(feed(true, "cv-v1").is_some());
    for off in ["off", "OFF", "0", "false", "no"] {
        assert!(feed(true, off).is_none(), "{off} wins over a saved on");
    }
    assert!(feed(false, "1").is_none(), "the variable never turns it on");
    assert_eq!(
        stat_tracker::shadow::bans_picks::ENV,
        "STAT_TRACKER_BANS_PICKS_RECOGNIZER"
    );
}
