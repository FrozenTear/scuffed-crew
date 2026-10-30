//! The post-game side of [`PregameDump`]: the same switch, folders and
//! writer as the pre-game saves ([`crate::daemon::postgame_dump`]). A child
//! module so it shares the saver's fields.

use super::*;

impl PregameDump {
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
