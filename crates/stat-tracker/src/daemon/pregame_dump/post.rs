//! The post-game side of [`PregameDump`]: the same switch, folders and
//! writer as the pre-game saves ([`crate::daemon::postgame_dump`]). A child
//! module so it shares the saver's fields.

use super::*;

impl PregameDump {
    /// The poll loop's post-game want for the coming tick: `None` with the
    /// switch off (no saver), so the tick keeps no frame for these saves.
    pub(crate) fn post_want_of(
        dump: Option<&mut Self>,
        now: Instant,
    ) -> Option<crate::daemon::postgame_dump::PostWant> {
        dump.map(|dump| dump.post_want(now))
    }

    /// The poll loop's post-game step, after the tick's decision so a
    /// confirmed outcome opens the window: feed the saver `tick` and
    /// `frame`, then return how soon the next tick should come, only in the
    /// window and only when `every_tick_polls`. With the switch off (no
    /// saver) nothing is checked: `now`, `tick` and `every_tick_polls` are
    /// never called, `frame` is dropped and the regular interval stays.
    pub(crate) fn post_after_decision(
        dump: Option<&mut Self>,
        now: impl Fn() -> Instant,
        tick: impl FnOnce() -> crate::daemon::postgame_dump::PostTick,
        frame: Option<Arc<image::DynamicImage>>,
        poll: Duration,
        every_tick_polls: impl FnOnce() -> bool,
    ) -> Option<Duration> {
        let dump = dump?;
        dump.observe_post(now(), tick(), frame);
        dump.post_sooner_tick(now(), poll)
            .filter(|_| every_tick_polls())
    }

    /// What the next tick should hand over for the post-game saves, at
    /// its poll time `now`; nothing once the writer is gone. Notes when
    /// this tick was polled ([`Self::observe_post`], [`Self::post_sooner_tick`])
    /// and sends a final note a full queue refused.
    pub(crate) fn post_want(&mut self, now: Instant) -> crate::daemon::postgame_dump::PostWant {
        self.post_polled_at = Some(now);
        crate::daemon::postgame_dump::retry_pending(&mut self.post_log, &mut self.writer);
        if !self.writer.alive() {
            return crate::daemon::postgame_dump::PostWant::default();
        }
        self.post.want(now)
    }

    /// While a post-game window is open and this tick handed over a frame:
    /// bring the next tick forward to this (at least
    /// [`crate::daemon::postgame_dump::POSTGAME_SLACK`]), when the next sample is
    /// due before the regular tick (`poll` after this tick's poll).
    pub(crate) fn post_sooner_tick(&self, now: Instant, poll: Duration) -> Option<Duration> {
        let due_in = self.post.next_due_in(now)?;
        let since = self
            .post_ticked_at
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        crate::daemon::postgame_dump::sooner_tick(due_in, since, poll)
    }

    pub(super) fn post_sink(
        &mut self,
    ) -> (
        &mut crate::daemon::postgame_dump::PostgameTracker,
        crate::daemon::postgame_dump::DumpSink<'_>,
    ) {
        let sink = crate::daemon::postgame_dump::DumpSink {
            root: &self.root,
            latest: self.tracker.latest_game(),
            owner: &self.post_owner,
            log: &mut self.post_log,
            writer: &mut self.writer,
        };
        (&mut self.post, sink)
    }

    /// Feed one poll tick to the post-game saves, after the pre-game ones
    /// (whose loading read counts as a new screen). The tick counts at its
    /// poll time ([`Self::post_want`]), else `now`. With the writer gone
    /// the saves stop: no ring, no thumbnails.
    pub(crate) fn observe_post(
        &mut self,
        now: Instant,
        mut tick: crate::daemon::postgame_dump::PostTick,
        frame: Option<Arc<image::DynamicImage>>,
    ) {
        tick.new_screen |= std::mem::take(&mut self.saw_loading);
        if !self.writer.alive() {
            self.post.stop();
            return;
        }
        let at = self.post_polled_at.take().unwrap_or(now);
        self.post_ticked_at = Some(at);
        let latest = self.tracker.latest_game().map(|(folder, _)| folder);
        self.post_owner.note(at, latest, tick.game_opened);
        let (post, mut sink) = self.post_sink();
        post.observe(at, tick, frame, &mut sink);
    }

    /// The pre-game want for the coming tick: `want`, plus the loading
    /// check while a post-game window is open, so a loading screen after
    /// the match ends the window ([`Self::observe_post`]). It is the same
    /// check as the pre-game saves' ([`classify`]); no frame is kept for
    /// it (`other_frame` is the tracker's own).
    pub(super) fn with_post_loading(&self, want: PregameWant) -> PregameWant {
        PregameWant {
            loading: want.loading || self.post.window_open(),
            ..want
        }
    }

    /// `seen` for the pre-game tracker at `now`: a loading read only the
    /// post-game window asked for is no loading screen to it, so it sees
    /// what it would have with no window open. Its ban-results wait and a
    /// window never overlap: a hero ban read ends the window.
    pub(super) fn pre_seen(&self, now: Instant, seen: Seen) -> Seen {
        if seen == Seen::Loading && !self.tracker.want(now).loading {
            Seen::Other
        } else {
            seen
        }
    }

    /// Replace the writer with one whose thread is gone (tests).
    #[cfg(test)]
    pub(crate) fn kill_writer(&mut self) {
        self.writer = PregameWriter::dead();
    }

    /// Whether a post-game window is open (tests).
    #[cfg(test)]
    pub(crate) fn post_window_open(&self) -> bool {
        self.post.window_open()
    }

    /// The game closed: [`Self::release`], and the post-game saves drop
    /// their ring and end their window.
    pub(crate) fn game_closed(&mut self, now: Instant) {
        self.release(now);
        let (post, mut sink) = self.post_sink();
        post.release(
            now,
            crate::daemon::postgame_dump::PostEnd::GameClosed,
            &mut sink,
        );
    }
}
