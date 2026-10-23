//! The OCR on/off switch: what `ocr = "off"` needs, and what the daemon
//! decides at startup.
//!
//! Off is granted only when nothing the tracker needs would go unread:
//!
//! * [`Read`] is every read a tracked game depends on. Each says whether
//!   this build reads it without Tesseract ([`Read::without_ocr`]). While
//!   one does not, off is refused, whatever packs are installed, so the
//!   switch can never leave a member with a tracker that stores nothing.
//! * [`PackReadiness`] is what the readers that have no OCR fallback need
//!   loaded: a main hero pack, the map pack, and from the word pack the
//!   result words, the match-start titles and the end-of-match titles.
//! * [`decide`] turns the requested setting into the one the daemon runs
//!   with. A refusal carries its reasons ([`Refusal`]) and the daemon keeps
//!   OCR on.
//! * [`Switch::start`] is the daemon's startup step. It sets the gate
//!   ([`crate::ocr::gate`]) and writes [`Status`] to `ocr_switch.json` in the
//!   data dir, which is how the app shows the reason, the pack state and the
//!   blocked-call counts.
//! * [`Switch::check`] re-checks the packs while the daemon runs. A pack that
//!   disappears or goes out of date does not turn OCR back on: the reads it
//!   feeds are held, and the status file says which pack it is.
//!
//! The reads, the packs, the decision and the status file are plain data in
//! [`crate::ocr_status`], re-exported here. This module is the part that
//! loads the packs ([`pack_readiness_on_disk`]) and sets the gate.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{Config, OcrSetting};
use crate::ocr::gate::{self, OcrMode};
pub use crate::ocr_status::{
    Audience, CLICK_FIX, DAEMON_FIX, Decision, HOLD_FIX, LATER_VERSION, PackGroup, PackProblem,
    PackReadiness, READER_ENTRIES, RESTART_TO_APPLY, Read, Refusal, STATUS_FILE, STATUS_TMP_FILE,
    Status, StatusSite, TURN_BACK_ON, capture_notice, daemon_share_open, decide, effective_reader,
    publish_capture_notice, publish_share_notice, publish_share_open, read_status,
    set_share_notice, share_notice, status_path, write_status,
};
use crate::ocr_status::{decide_for_build, status_write_lock};
use crate::shadow::{Reader, ReaderConfig};

/// How often [`watch`] looks at the packs and the blocked-call count.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(30);
/// How long daemon startup waits for the reader packs still downloading
/// when off is asked for and a pack is missing ([`needs_pack_wait`]).
pub const PACK_WAIT: Duration = Duration::from_secs(45);

// ----------------------------------------------------------------- switch

/// The packs under `data_dir`, loaded the way the daemon's reader loads
/// them. Decodes the pack folders, so the app runs it off its UI thread.
///
/// This was `PackReadiness::on_disk`. `PackReadiness` is plain data in
/// `ocr_status` now, and loading the packs needs the reader.
pub fn pack_readiness_on_disk(data_dir: &Path) -> PackReadiness {
    Reader::load(&ReaderConfig::from_data_dir(data_dir)).pack_readiness()
}

/// True when daemon startup should give the reader packs that are still
/// downloading a moment before it decides: off is asked for, this build
/// could grant it, and a pack is missing. Startup syncs heroes-v1 first and
/// the other packs in the background, so without the wait a first start
/// refuses off for the whole run.
pub fn needs_pack_wait(config: &Config) -> bool {
    pack_wait_applies(
        config.ocr_control().requested(),
        &Read::needing_ocr(),
        || pack_readiness_on_disk(&config.data_dir),
    )
}

/// [`needs_pack_wait`] with its inputs as arguments. `packs` is looked at
/// only when off is asked for and this build could grant it.
pub fn pack_wait_applies(
    requested: OcrSetting,
    needing_ocr: &[Read],
    packs: impl FnOnce() -> PackReadiness,
) -> bool {
    requested.is_off() && needing_ocr.is_empty() && !packs().ready()
}

/// The daemon's switch state for one run.
pub struct Switch {
    pub decision: Decision,
    data_dir: PathBuf,
    /// The reader the packs are asked of when off was refused and the
    /// process-wide reader is not this one. It re-loads a pack whose folder
    /// changed, like the process-wide one.
    own_reader: Option<Reader>,
    /// The packs as last seen. `None` when they were never looked at: OCR
    /// asked on, or a build that refuses off whatever the packs are.
    packs: Option<PackReadiness>,
    written: Option<Status>,
    /// The last write of the status file failed (said once at WARN).
    write_failed: bool,
}

impl std::fmt::Debug for Switch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Switch")
            .field("decision", &self.decision)
            .field("packs", &self.packs)
            .finish_non_exhaustive()
    }
}

impl Switch {
    /// Daemon startup: read the request, check this build and the packs
    /// when it is off, set the gate, log the outcome and write the status
    /// file.
    ///
    /// The packs are loaded at most once, and only when they can change the
    /// answer: off is asked for and this build could grant it. A build that
    /// refuses anyway loads nothing and watches nothing.
    ///
    /// When off is granted, or refused for a pack while `reader = "new"`,
    /// the reader that was loaded becomes the process-wide one
    /// ([`crate::shadow::install`]), so the daemon does not load the packs a
    /// second time. Refused for a pack under `reader = "ocr-v1"`, the reader
    /// stays with the switch, only to see the packs arrive.
    pub fn start(config: &Config) -> Self {
        Self::start_with(config, &Read::needing_ocr(), |data_dir| {
            Reader::load(&ReaderConfig::from_data_dir(data_dir))
        })
    }

    /// [`Self::start`] with the reads this build still needs OCR for and
    /// the reader as arguments (tests). `reader` runs only when off is
    /// requested and `needing_ocr` is empty.
    pub fn start_with(
        config: &Config,
        needing_ocr: &[Read],
        reader: impl FnOnce(&Path) -> Reader,
    ) -> Self {
        let requested = config.ocr_control().requested();
        let reader =
            (requested.is_off() && needing_ocr.is_empty()).then(|| reader(&config.data_dir));
        let packs = reader.as_ref().map(Reader::pack_readiness);
        let decision = match &packs {
            Some(packs) => decide(requested, packs, needing_ocr),
            None => decide_for_build(requested, needing_ocr),
        };
        let own_reader = match reader {
            Some(reader) if decision.reader(config).is_new() => {
                // The new reader reads this run (off was granted, or it was
                // refused for a pack and `reader = "new"`): this is the
                // reader. When one was installed before (a second start in
                // one process), that one stays, and the watch asks it.
                crate::shadow::install(reader);
                None
            }
            other => other,
        };
        let mut switch = Self {
            decision,
            data_dir: config.data_dir.clone(),
            own_reader,
            packs,
            written: None,
            write_failed: false,
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
            (OcrSetting::On, Some(refusal)) => {
                gate::set_mode(OcrMode::On);
                tracing::warn!(
                    reason = %refusal.text(Audience::Daemon),
                    packs = ?refusal.packs.iter().map(|p| p.pack()).collect::<Vec<_>>(),
                    reads = ?refusal.reads,
                    "ocr = \"off\" was refused: classic OCR stays on for this run"
                );
            }
            (OcrSetting::On, None) => gate::set_mode(OcrMode::On),
        }
    }

    /// True when the daemon runs without Tesseract. It then reads with the
    /// new reader whatever `reader` says ([`Decision::reader`]).
    pub fn is_off(&self) -> bool {
        self.decision.effective.is_off()
    }

    fn status(&self) -> Status {
        // Two places can share a key (calls from outside the source
        // folder are all `other.rs:1`): one line per key, counts added.
        let mut by_key: std::collections::BTreeMap<String, u64> = Default::default();
        for b in gate::blocked_calls() {
            let count = by_key.entry(b.site.key()).or_default();
            *count = count.saturating_add(b.count);
        }
        Status {
            pid: std::process::id(),
            requested: self.decision.requested,
            effective: self.decision.effective,
            refusal: self.decision.refusal.clone(),
            packs: self.packs,
            blocked_total: gate::blocked_total(),
            blocked: by_key
                .into_iter()
                .map(|(site, count)| StatusSite { site, count })
                .collect(),
            leptess_created: gate::leptess_created(),
            share_open: daemon_share_open(),
            share_notice: share_notice(),
            capture_notice: capture_notice(),
        }
    }

    /// Write the status file when it changed since the last write.
    fn publish(&mut self) {
        let _guard = status_write_lock();
        let status = self.status();
        if self.written.as_ref() == Some(&status) {
            return;
        }
        match write_status(&self.data_dir, &status) {
            Ok(()) => {
                self.written = Some(status);
                self.write_failed = false;
            }
            // Once per failure, not on every tick: without the file the app
            // cannot say what the tracker decided.
            Err(e) if !self.write_failed => {
                self.write_failed = true;
                tracing::warn!(
                    error = %e,
                    file = STATUS_FILE,
                    "could not write the OCR switch status; the app will not show what the tracker decided"
                );
            }
            Err(e) => tracing::debug!(error = %e, "could not write the OCR switch status"),
        }
    }

    /// The packs right now, as the reader in use (or, after a refusal, the
    /// switch's own reader) has them loaded.
    fn packs_now(&self) -> PackReadiness {
        match &self.own_reader {
            Some(reader) => reader.pack_readiness(),
            None => crate::shadow::pack_readiness(),
        }
    }

    /// One look at the packs and the counters. Logs when the pack state
    /// changes and rewrites the status file when anything did. The daemon
    /// also calls it once when it shuts down, so the file holds the run's
    /// last counts, with OCR on as well.
    pub fn check(&mut self) {
        if self.watches() {
            let now = self.packs_now();
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
                    packs = ?now.problems().iter().map(|p| p.pack()).collect::<Vec<_>>(),
                    "{banner} (the tracker does not fall back to classic OCR while it is off)"
                ),
                None => tracing::info!("the reader packs are back: nothing is on hold"),
            }
        } else if now.ready()
            && self
                .decision
                .refusal
                .as_ref()
                .is_some_and(|r| r.reads.is_empty())
        {
            tracing::info!("{RESTART_TO_APPLY}");
        }
    }

    /// Whether the watch has anything to do: only when the packs were
    /// looked at, which is when off was asked for and this build could
    /// grant it. Then a pack that goes away is held and named, and packs
    /// that arrive after a refusal are announced ("restart the tracker").
    pub fn watches(&self) -> bool {
        self.packs.is_some()
    }
}

/// The switch as the daemon shares it between the watch and shutdown.
pub type SharedSwitch = std::sync::Arc<std::sync::Mutex<Switch>>;

/// Re-check `switch` every [`WATCH_INTERVAL`] for as long as the daemon
/// runs. Meant for `tokio::spawn`. Ends at once when OCR was asked to stay
/// on: there is nothing to watch then.
pub async fn watch(switch: SharedSwitch) {
    if !switch.lock().unwrap_or_else(|e| e.into_inner()).watches() {
        return;
    }
    let mut timer = tokio::time::interval(WATCH_INTERVAL);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        // The check stats pack folders and may write the status file: off
        // the async workers.
        let switch = std::sync::Arc::clone(&switch);
        let _ = tokio::task::spawn_blocking(move || {
            switch.lock().unwrap_or_else(|e| e.into_inner()).check()
        })
        .await;
    }
}

/// The reader id for this process, from the OCR gate.
///
/// Moved here from `shadow::digits` (0.6 lane D V3) so the cv reader does
/// not read the gate; `shadow::digits` re-exports it at its old path.
pub fn live_recognizer_id() -> &'static str {
    crate::shadow::digits::recognizer_id(!gate::is_off())
}

#[cfg(test)]
mod tests {
    use super::*;

    const READY: PackReadiness = PackReadiness {
        heroes: true,
        maps: true,
        words: true,
        result_words: true,
        phase_titles: true,
        end_titles: true,
    };

    #[test]
    fn startup_waits_for_packs_only_when_that_could_turn_ocr_off() {
        let missing = || PackReadiness::default();
        let never = || -> PackReadiness { panic!("the packs were looked at") };
        assert!(pack_wait_applies(OcrSetting::Off, &[], missing));
        assert!(!pack_wait_applies(OcrSetting::Off, &[], || READY));
        // OCR on, or a build that refuses off anyway: no wait, no pack load
        assert!(!pack_wait_applies(OcrSetting::On, &[], never));
        assert!(!pack_wait_applies(
            OcrSetting::Off,
            &[Read::StatCells],
            never
        ));
    }

    /// Every `shadow::read_*` entry point has a line in [`READER_ENTRIES`],
    /// so what it needs is part of [`PackReadiness`]. A new read without a
    /// line fails here.
    #[test]
    fn every_reader_entry_point_has_its_pack_groups() {
        let source = include_str!("shadow/reader.rs");
        let production = source.split("\n#[cfg(test)]").next().unwrap();
        let mut found: Vec<&str> = production
            .lines()
            .filter_map(|line| line.strip_prefix("pub fn read_"))
            .filter_map(|rest| rest.split(['(', '<']).next())
            .collect();
        found.sort_unstable();
        let mut listed: Vec<&str> = READER_ENTRIES
            .iter()
            .map(|(name, _)| name.trim_start_matches("read_"))
            .collect();
        listed.sort_unstable();
        assert_eq!(
            found, listed,
            "shadow/reader.rs and ocr_switch::READER_ENTRIES name different reads"
        );
        assert!(
            found.len() >= 12,
            "the scan found the entry points: {found:?}"
        );
        // A reader with nothing loaded has none of the groups, and one with
        // everything has all of them: every group is a readiness field.
        for (name, groups) in READER_ENTRIES {
            for group in *groups {
                assert!(READY.has(*group), "{name}");
                assert!(!PackReadiness::default().has(*group), "{name}");
            }
        }
    }
}
