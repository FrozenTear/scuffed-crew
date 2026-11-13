//! The Unsure confirm rule's verdicts for the latest game (alpha.3 item 5).
//!
//! The tracker keeps the reads of each own-row stat of the latest game in
//! `stat_reads.json` ([`stat_tracker::games::stat_reads`]). On each
//! snapshot load the rows of that game are flagged from the rule
//! ([`apply`]): a stat is unsure only on `Verdict::Unsure`, judged live
//! while `active_game.json` still names the game unfinished, and with
//! `game_over` after. The verdicts are kept here for the Games panel's
//! confidence line ([`line`]). Older games keep the flags stored on their
//! rows: the tracker does not store a verdict with the game.

use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;
use stat_tracker::games::stat_reads::{self, STATS};
use stat_tracker::storage::Snapshot;
use stat_tracker::unsure_rule::{StatConfirm, Verdict};

type Verdicts = (String, [Option<StatConfirm>; 6]);

/// The verdicts of the last [`apply`]: the game and one per stat.
static LATEST: Mutex<Option<Verdicts>> = Mutex::new(None);

/// Flag the latest game's stats in `snap` from the rule. A missing or
/// unreadable log leaves `snap` as it is.
pub fn apply(data_dir: &Path, snap: &mut Snapshot) {
    let verdicts = verdicts(data_dir, snap);
    if let Ok(mut latest) = LATEST.lock() {
        *latest = Some(verdicts);
    }
}

fn verdicts(data_dir: &Path, snap: &mut Snapshot) -> Verdicts {
    let log = stat_reads::load(data_dir);
    let live = crate::overlay::live_active_game_key(
        data_dir,
        crate::overlay::DEFAULT_SESSION_WINDOW_SECS,
        Utc::now(),
    );
    let game_over = live.as_deref() != Some(log.session_id.as_str());
    let verdicts = log.confirm(&log.session_id, game_over);
    stat_reads::apply_verdicts(&mut snap.matches, &log.session_id, &verdicts);
    (log.session_id, verdicts)
}

/// The confidence line for stat `flat` (`e`, `dmg`, ...) of `session_id`
/// when the rule says Unsure, with `stored` the number the game shows.
pub fn line(session_id: &str, flat: &str, stored: u32) -> Option<String> {
    line_in(LATEST.lock().ok()?.as_ref()?, session_id, flat, stored)
}

fn line_in(latest: &Verdicts, session_id: &str, flat: &str, stored: u32) -> Option<String> {
    let (sid, verdicts) = latest;
    let col = STATS.iter().position(|s| *s == flat)?;
    let c = verdicts[col].filter(|c| c.verdict == Verdict::Unsure)?;
    (sid == session_id).then(|| stat_reads::confidence_line(&c, stored))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stat_tracker::unsure_rule::StatRead;

    /// A log on disk flags its game's stat only on Unsure, and the line
    /// says how sure the reader was.
    #[test]
    fn the_latest_game_is_flagged_from_the_rule() {
        let dir = tempfile::tempdir().unwrap();
        stat_reads::update(dir.path(), |l| {
            for _ in 0..3 {
                let mut tab = [Some(StatRead::sure(5, 0.9)); 6];
                tab[3] = Some(StatRead::unsure(Some(1200), 0.4));
                l.push("s-ui", false, &tab);
            }
        });
        let mut snap = crate::fixtures::snapshot(crate::cli::FixtureKind::Sample);
        snap.matches.truncate(1);
        snap.matches[0].session_id = "s-ui".into();
        snap.matches[0].suspect_fields = vec!["e".into(), "hero".into()];
        let v = verdicts(dir.path(), &mut snap);
        assert_eq!(snap.matches[0].suspect_fields, vec!["hero", "dmg"]);
        let line = |flat, stored| line_in(&v, "s-ui", flat, stored);
        assert_eq!(line("dmg", 900).as_deref(), Some("Read 1200 at 40%"));
        assert_eq!(line("e", 5), None, "a confirmed stat has no line");
        assert_eq!(line_in(&v, "other", "dmg", 900), None);
    }
}
