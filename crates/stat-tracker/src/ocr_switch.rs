//! The OCR on/off switch: what `ocr = "off"` needs, and what the daemon
//! decides at startup.
//!
//! * [`PackReadiness`] is the reader packs OCR-off depends on: a hero pack
//!   (heroes-v2 or heroes-v1), maps-v1 and words-v1.
//! * [`decide`] turns the requested setting into the one the daemon runs
//!   with. Off is refused, with a reason, while a pack is missing or out of
//!   date: the daemon then keeps OCR on.
//! * [`Switch::start`] is the daemon's startup step. It sets the gate
//!   ([`crate::ocr::gate`]) and writes [`Status`] to `ocr_switch.json` in the
//!   data dir, which is how the app shows the reason, the pack state and the
//!   blocked-call count.
//! * [`Switch::watch`] re-checks the packs while the daemon runs. A pack that
//!   disappears or goes out of date does not turn OCR back on: the fields it
//!   feeds are held, and the status file says which pack it is.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{Config, OcrSetting};
use crate::ocr::gate::{self, OcrMode};

/// File name of [`Status`] in the data dir.
pub const STATUS_FILE: &str = "ocr_switch.json";
/// How often [`Switch::watch`] looks at the packs and the blocked-call count.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(30);

/// The reader packs OCR-off needs, as one reader sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PackReadiness {
    /// A hero pack loads: heroes-v2 or heroes-v1.
    pub heroes: bool,
    /// maps-v1 loads (the pack sync installs it).
    pub maps: bool,
    /// words-v1 loads.
    pub words: bool,
    /// The loaded words-v1 has the FINAL SCORE template. A words-v1 from
    /// before that template is out of date.
    pub words_final_score: bool,
}

/// One pack OCR-off cannot run without, and what is wrong with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackProblem {
    HeroesMissing,
    MapsMissing,
    WordsMissing,
    WordsOutOfDate,
}

impl PackProblem {
    /// The pack, as the pack list names it.
    pub const fn pack(self) -> &'static str {
        match self {
            Self::HeroesMissing => "heroes",
            Self::MapsMissing => "maps-v1",
            Self::WordsMissing | Self::WordsOutOfDate => "words-v1",
        }
    }

    /// "the maps-v1 reader pack is not installed".
    pub const fn sentence(self) -> &'static str {
        match self {
            Self::HeroesMissing => "the hero reader pack (heroes-v2 or heroes-v1) is not installed",
            Self::MapsMissing => "the maps-v1 reader pack is not installed",
            Self::WordsMissing => "the words-v1 reader pack is not installed",
            Self::WordsOutOfDate => {
                "the words-v1 reader pack is out of date (it has no FINAL SCORE template)"
            }
        }
    }

    /// What stays unread while this pack is the problem.
    pub const fn held(self) -> &'static str {
        match self {
            Self::HeroesMissing => "heroes",
            Self::MapsMissing => "the map",
            Self::WordsMissing => "the result",
            Self::WordsOutOfDate => "the result on a faint end title",
        }
    }
}

impl PackReadiness {
    /// The packs under `data_dir`, loaded the way the daemon's reader loads
    /// them. Reads the pack folders; the app calls it when the member turns
    /// classic OCR off.
    pub fn on_disk(data_dir: &Path) -> Self {
        crate::shadow::Reader::load(&crate::shadow::ReaderConfig::from_data_dir(data_dir))
            .pack_readiness()
    }

    /// Every pack problem, in pack-list order. Empty when OCR-off can run.
    pub fn problems(&self) -> Vec<PackProblem> {
        let mut out = Vec::new();
        if !self.heroes {
            out.push(PackProblem::HeroesMissing);
        }
        if !self.maps {
            out.push(PackProblem::MapsMissing);
        }
        if !self.words {
            out.push(PackProblem::WordsMissing);
        } else if !self.words_final_score {
            out.push(PackProblem::WordsOutOfDate);
        }
        out
    }

    pub fn ready(&self) -> bool {
        self.problems().is_empty()
    }

    /// `(ready, needed)` over the three packs. An out-of-date pack is not
    /// ready.
    pub fn counts(&self) -> (usize, usize) {
        let ready = [self.heroes, self.maps, self.words && self.words_final_score]
            .into_iter()
            .filter(|ok| *ok)
            .count();
        (ready, 3)
    }

    /// "Reader packs ready: 2 of 3".
    pub fn status_line(&self) -> String {
        let (ready, needed) = self.counts();
        format!("Reader packs ready: {ready} of {needed}")
    }

    /// Why OCR cannot be turned off with these packs, or `None` when it can.
    pub fn refusal(&self) -> Option<String> {
        let problems = self.problems();
        if problems.is_empty() {
            return None;
        }
        Some(format!(
            "Classic OCR stays on: {}. {REFUSAL_FIX}",
            join_sentences(problems.iter().map(|p| p.sentence()))
        ))
    }

    /// The line the app shows while the daemon runs with OCR off and a pack
    /// is gone or out of date, or `None` when the packs are fine.
    pub fn hold_banner(&self) -> Option<String> {
        let problems = self.problems();
        if problems.is_empty() {
            return None;
        }
        Some(format!(
            "Reading on hold: {}, so {} {} not read. Classic OCR stays off. {HOLD_FIX}",
            join_sentences(problems.iter().map(|p| p.sentence())),
            join_sentences(problems.iter().map(|p| p.held())),
            if problems.len() == 1 && !matches!(problems[0], PackProblem::HeroesMissing) {
                "is"
            } else {
                "are"
            },
        ))
    }
}

/// How a member gets the packs.
pub const REFUSAL_FIX: &str = "Sign in and download the reader packs (Settings, Open setup guide), then turn classic OCR off again.";
/// What a member can do while a pack is gone with OCR off.
pub const HOLD_FIX: &str =
    "Download the reader packs again (Settings, Open setup guide), or turn classic OCR on.";
/// Shown when the packs came back after the daemon refused.
pub const RESTART_TO_APPLY: &str =
    "The reader packs are ready now. Restart the tracker to turn classic OCR off.";

fn join_sentences<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let parts: Vec<&str> = parts.collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// What the daemon runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// What config (or the env override) asked for.
    pub requested: OcrSetting,
    /// What runs. On when off was refused.
    pub effective: OcrSetting,
    /// Why `effective` is on although `requested` is off.
    pub refusal: Option<String>,
}

/// `requested` checked against the packs. On is always granted. Off is
/// granted only when every pack is ready; otherwise OCR stays on and the
/// reason comes back.
pub fn decide(requested: OcrSetting, packs: &PackReadiness) -> Decision {
    let refusal = match requested {
        OcrSetting::On => None,
        OcrSetting::Off => packs.refusal(),
    };
    Decision {
        requested,
        effective: if refusal.is_some() {
            OcrSetting::On
        } else {
            requested
        },
        refusal,
    }
}

/// One blocked call site in [`Status`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSite {
    /// `file:line function`, a place in the tracker's source.
    pub site: String,
    pub count: u64,
}

/// What the daemon last wrote about the switch. Values only: settings, pack
/// flags, counts and source locations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// The daemon that wrote this.
    pub pid: u32,
    pub requested: OcrSetting,
    pub effective: OcrSetting,
    /// Why the daemon kept OCR on at startup.
    pub refusal: Option<String>,
    /// The packs as the daemon's reader last saw them. Not checked while
    /// `requested` is on.
    pub packs: Option<PackReadiness>,
    /// OCR calls the gate blocked in this daemon run.
    pub blocked_total: u64,
    pub blocked: Vec<StatusSite>,
    /// Tesseract instances this daemon run created.
    pub leptess_created: u64,
}

impl Status {
    /// The line for the app's health panel and Settings, or `None` when
    /// there is nothing to say (OCR on as asked, or off with every pack).
    pub fn notice(&self) -> Option<String> {
        if self.requested.is_on() {
            return None;
        }
        match (&self.refusal, &self.packs) {
            (Some(_), Some(packs)) if packs.ready() => Some(RESTART_TO_APPLY.to_string()),
            (Some(reason), _) => Some(reason.clone()),
            (None, Some(packs)) => packs.hold_banner(),
            (None, None) => None,
        }
    }
}

pub fn status_path(data_dir: &Path) -> PathBuf {
    data_dir.join(STATUS_FILE)
}

/// The status the daemon last wrote, or `None` when there is none or it
/// does not parse.
pub fn read_status(data_dir: &Path) -> Option<Status> {
    let bytes = std::fs::read(status_path(data_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write `status` next to the other daemon files (temp file, then rename).
pub fn write_status(data_dir: &Path, status: &Status) -> std::io::Result<()> {
    let path = status_path(data_dir);
    let tmp = data_dir.join(format!("{STATUS_FILE}.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(status).map_err(std::io::Error::other)?;
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// The daemon's switch state for one run.
#[derive(Debug)]
pub struct Switch {
    pub decision: Decision,
    data_dir: PathBuf,
    packs: Option<PackReadiness>,
    written: Option<Status>,
}

impl Switch {
    /// Daemon startup: read the request, check the packs when it is off,
    /// set the gate, log the outcome and write the status file.
    ///
    /// When off is requested this loads the process-wide reader
    /// ([`crate::shadow::init`]) from `config.data_dir`, since off implies
    /// the new reader.
    pub fn start(config: &Config) -> Self {
        Self::start_with(config, |data_dir| {
            crate::shadow::init(&crate::shadow::ReaderConfig::from_data_dir(data_dir));
            crate::shadow::pack_readiness()
        })
    }

    /// [`Self::start`] with the pack check as an argument (tests). `packs`
    /// runs only when off is requested.
    pub fn start_with(config: &Config, packs: impl FnOnce(&Path) -> PackReadiness) -> Self {
        let requested = config.ocr_control().requested();
        let packs = requested.is_off().then(|| packs(&config.data_dir));
        let decision = decide(requested, &packs.unwrap_or_default());
        let mut switch = Self {
            decision,
            data_dir: config.data_dir.clone(),
            packs,
            written: None,
        };
        switch.apply();
        switch.publish();
        switch
    }

    /// Set the gate and say what was decided.
    fn apply(&self) {
        match (&self.decision.effective, &self.decision.refusal) {
            (OcrSetting::Off, _) => {
                gate::set_mode(OcrMode::Off);
                tracing::info!(
                    "classic OCR is off (ocr = \"off\"): the new reader reads everything and Tesseract is never started"
                );
            }
            (OcrSetting::On, Some(reason)) => {
                gate::set_mode(OcrMode::On);
                tracing::warn!(
                    reason = %reason,
                    "ocr = \"off\" was refused: classic OCR stays on for this run"
                );
            }
            (OcrSetting::On, None) => gate::set_mode(OcrMode::On),
        }
    }

    /// True when the daemon runs without Tesseract. It then reads with the
    /// new reader whatever `reader` says.
    pub fn is_off(&self) -> bool {
        self.decision.effective.is_off()
    }

    fn status(&self) -> Status {
        Status {
            pid: std::process::id(),
            requested: self.decision.requested,
            effective: self.decision.effective,
            refusal: self.decision.refusal.clone(),
            packs: self.packs,
            blocked_total: gate::blocked_total(),
            blocked: gate::blocked_calls()
                .into_iter()
                .map(|b| StatusSite {
                    site: b.site.to_string(),
                    count: b.count,
                })
                .collect(),
            leptess_created: gate::leptess_created(),
        }
    }

    /// Write the status file when it changed since the last write.
    fn publish(&mut self) {
        let status = self.status();
        if self.written.as_ref() == Some(&status) {
            return;
        }
        match write_status(&self.data_dir, &status) {
            Ok(()) => self.written = Some(status),
            Err(e) => tracing::debug!(error = %e, "could not write the OCR switch status"),
        }
    }

    /// One look at the packs and the blocked-call count. Logs when the pack
    /// state changes and rewrites the status file when anything did.
    pub fn check(&mut self) {
        if self.decision.requested.is_off() {
            let now = crate::shadow::pack_readiness();
            if self.packs != Some(now) {
                self.log_pack_change(&now);
                self.packs = Some(now);
            }
        }
        self.publish();
    }

    fn log_pack_change(&self, now: &PackReadiness) {
        if self.is_off() {
            match now.hold_banner() {
                Some(banner) => tracing::warn!(
                    "{banner} (the tracker does not fall back to classic OCR while it is off)"
                ),
                None => tracing::info!("the reader packs are back: nothing is on hold"),
            }
        } else if now.ready() {
            tracing::info!("{RESTART_TO_APPLY}");
        }
    }

    /// Re-check every [`WATCH_INTERVAL`] for as long as the daemon runs.
    /// Meant for `tokio::spawn`. Ends at once when OCR was asked to stay on:
    /// there is nothing to watch then.
    pub async fn watch(mut self) {
        if self.decision.requested.is_on() {
            return;
        }
        let mut timer = tokio::time::interval(WATCH_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            self.check();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: PackReadiness = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        words_final_score: true,
    };

    #[test]
    fn off_is_granted_only_with_every_pack() {
        let granted = decide(OcrSetting::Off, &READY);
        assert_eq!(granted.effective, OcrSetting::Off);
        assert_eq!(granted.refusal, None);
        assert_eq!(READY.counts(), (3, 3));
        assert_eq!(READY.status_line(), "Reader packs ready: 3 of 3");

        let cases = [
            (
                PackReadiness {
                    heroes: false,
                    ..READY
                },
                "hero reader pack",
            ),
            (
                PackReadiness {
                    maps: false,
                    ..READY
                },
                "maps-v1",
            ),
            (
                PackReadiness {
                    words: false,
                    words_final_score: false,
                    ..READY
                },
                "words-v1 reader pack is not installed",
            ),
            (
                PackReadiness {
                    words_final_score: false,
                    ..READY
                },
                "words-v1 reader pack is out of date",
            ),
        ];
        for (packs, named) in cases {
            let refused = decide(OcrSetting::Off, &packs);
            assert_eq!(refused.requested, OcrSetting::Off);
            assert_eq!(refused.effective, OcrSetting::On, "{packs:?} keeps OCR on");
            let reason = refused.refusal.expect("a refusal carries its reason");
            assert!(reason.starts_with("Classic OCR stays on: "), "{reason}");
            assert!(reason.contains(named), "{reason}");
            assert!(reason.ends_with(REFUSAL_FIX), "{reason}");
            assert_eq!(packs.counts(), (2, 3), "{packs:?}");
            assert_eq!(packs.problems().len(), 1, "{packs:?}");
        }
    }

    #[test]
    fn on_is_always_granted_and_never_looks_at_the_packs() {
        let none = PackReadiness::default();
        let on = decide(OcrSetting::On, &none);
        assert_eq!(on.effective, OcrSetting::On);
        assert_eq!(on.refusal, None);
        assert_eq!(none.counts(), (0, 3));
    }

    #[test]
    fn the_refusal_names_every_missing_pack_once() {
        let none = PackReadiness::default();
        let reason = none.refusal().unwrap();
        assert_eq!(
            reason,
            format!(
                "Classic OCR stays on: the hero reader pack (heroes-v2 or heroes-v1) is not installed, \
                 the maps-v1 reader pack is not installed and the words-v1 reader pack is not installed. {REFUSAL_FIX}"
            )
        );
        // a missing words-v1 is not also reported as out of date
        assert_eq!(
            none.problems(),
            [
                PackProblem::HeroesMissing,
                PackProblem::MapsMissing,
                PackProblem::WordsMissing
            ]
        );
    }

    #[test]
    fn the_hold_banner_says_what_is_not_read_and_never_offers_ocr_by_itself() {
        assert_eq!(READY.hold_banner(), None);
        let no_maps = PackReadiness {
            maps: false,
            ..READY
        };
        assert_eq!(
            no_maps.hold_banner().unwrap(),
            format!(
                "Reading on hold: the maps-v1 reader pack is not installed, so the map is not read. \
                 Classic OCR stays off. {HOLD_FIX}"
            )
        );
        let two = PackReadiness {
            heroes: false,
            words_final_score: false,
            ..READY
        };
        let banner = two.hold_banner().unwrap();
        assert!(
            banner.contains("so heroes and the result on a faint end title are not read"),
            "{banner}"
        );
    }

    fn status(
        requested: OcrSetting,
        refusal: Option<&str>,
        packs: Option<PackReadiness>,
    ) -> Status {
        Status {
            pid: 7,
            requested,
            effective: if refusal.is_some() {
                OcrSetting::On
            } else {
                requested
            },
            refusal: refusal.map(str::to_string),
            packs,
            blocked_total: 0,
            blocked: Vec::new(),
            leptess_created: 0,
        }
    }

    #[test]
    fn the_notice_follows_the_daemon_state() {
        // OCR on as asked: nothing to say, whatever the packs are.
        assert_eq!(status(OcrSetting::On, None, None).notice(), None);
        // Off and every pack loaded: nothing to say.
        assert_eq!(status(OcrSetting::Off, None, Some(READY)).notice(), None);
        // Refused at startup: the reason.
        let missing = PackReadiness {
            maps: false,
            ..READY
        };
        let reason = missing.refusal().unwrap();
        assert_eq!(
            status(OcrSetting::Off, Some(&reason), Some(missing)).notice(),
            Some(reason.clone())
        );
        // Refused, and the packs arrived since: a restart applies it.
        assert_eq!(
            status(OcrSetting::Off, Some(&reason), Some(READY)).notice(),
            Some(RESTART_TO_APPLY.to_string())
        );
        // Running with OCR off and a pack went away: the hold banner.
        assert_eq!(
            status(OcrSetting::Off, None, Some(missing)).notice(),
            missing.hold_banner()
        );
    }

    #[test]
    fn the_status_file_round_trips_and_a_broken_one_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_status(dir.path()), None, "no file yet");
        let mut written = status(OcrSetting::Off, None, Some(READY));
        written.blocked_total = 3;
        written.blocked = vec![StatusSite {
            site: "crates/stat-tracker/src/main.rs:1 recognize".into(),
            count: 3,
        }];
        write_status(dir.path(), &written).unwrap();
        assert_eq!(read_status(dir.path()), Some(written));
        let text = std::fs::read_to_string(status_path(dir.path())).unwrap();
        assert!(text.contains("\"requested\": \"off\""), "{text}");
        assert!(text.contains("\"effective\": \"off\""), "{text}");
        // no temp file is left behind
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, [std::ffi::OsString::from(STATUS_FILE)]);
        std::fs::write(status_path(dir.path()), b"{ not json").unwrap();
        assert_eq!(read_status(dir.path()), None);
    }
}
