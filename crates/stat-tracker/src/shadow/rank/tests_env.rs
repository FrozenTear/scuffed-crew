//! [`RankSetting::env_override`] and
//! [`QueueTitleSetting::env_override`](crate::shadow::queue_title::QueueTitleSetting::env_override)
//! read the real process environment. Setting a variable in this process
//! would race every other test, so each case runs the probe below in its
//! own child process with the variables set there (the pattern of
//! `main_tests::ocr_off_switch`).

use crate::shadow::queue_title::{self, QueueTitleSetting};
use crate::shadow::rank::{ENV, RankSetting};

/// Set only in the child process, so the probe never runs on its own.
const PROBE_ENV: &str = "SCUFFED_RANK_ENV_PROBE";

/// What both overrides read with the rank and queue title variables set to
/// `rank` and `queue` (`None` leaves a variable unset), from a child process.
fn overrides_with(rank: Option<&str>, queue: Option<&str>) -> String {
    let exe = std::env::current_exe().expect("this test binary");
    let mut cmd = std::process::Command::new(exe);
    cmd.args([
        "--ignored",
        "--exact",
        "shadow::rank::tests::env_override::probe_the_env_overrides",
        "--test-threads",
        "1",
        "--nocapture",
    ])
    .env(PROBE_ENV, "1")
    .env_remove(ENV)
    .env_remove(queue_title::ENV);
    if let Some(v) = rank {
        cmd.env(ENV, v);
    }
    if let Some(v) = queue {
        cmd.env(queue_title::ENV, v);
    }
    let run = cmd.output().expect("run the probe in its own process");
    let text = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success() && text.contains("1 passed"),
        "the probe, in its own process:\n{text}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    // libtest prints "test <name> ... " on the probe's own line, so the
    // marker is looked for anywhere in a line
    text.lines()
        .find_map(|l| l.split_once("overrides: ").map(|(_, read)| read))
        .expect("the probe prints its line")
        .to_string()
}

/// Any non-blank value overrides the config: an on word turns the reader
/// on, and anything else, a typo too, turns it off. Unset or blank leaves
/// the config in charge. Each variable decides only its own reader.
#[test]
fn each_variable_overrides_its_reader_when_it_is_not_blank() {
    assert_eq!(overrides_with(None, None), "None None");
    assert_eq!(overrides_with(Some(""), Some("  ")), "None None");
    assert_eq!(
        overrides_with(Some("on"), None),
        "Some(RankV1) None",
        "the queue title variable unset stays unset"
    );
    assert_eq!(overrides_with(None, Some("on")), "None Some(On)");
    assert_eq!(
        overrides_with(Some(" Rank-V1 "), Some("RANK-V1")),
        "Some(RankV1) Some(On)"
    );
    // the queue title reader's id is rank-v1 too: its pack name is no on word
    assert_eq!(
        overrides_with(None, Some("queue-title-v1")),
        "None Some(Off)"
    );
    assert_eq!(
        overrides_with(Some("off"), Some("0")),
        "Some(Off) Some(Off)"
    );
    // a typo is a value: it overrides, and counts as off
    assert_eq!(
        overrides_with(Some("onn"), Some("yse")),
        "Some(Off) Some(Off)"
    );
}

/// Run by [`each_variable_overrides_its_reader_when_it_is_not_blank`] in a
/// child process with the variables it sets: prints what each override
/// reads.
#[test]
#[ignore = "run in its own process by each_variable_overrides_its_reader_when_it_is_not_blank"]
fn probe_the_env_overrides() {
    assert_eq!(
        std::env::var(PROBE_ENV).as_deref(),
        Ok("1"),
        "run through each_variable_overrides_its_reader_when_it_is_not_blank"
    );
    println!(
        "overrides: {:?} {:?}",
        RankSetting::env_override(),
        QueueTitleSetting::env_override()
    );
}
