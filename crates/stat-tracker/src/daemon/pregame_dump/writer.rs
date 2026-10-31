//! The pre-game saves' writer thread ([`PregameWriter`]), shared with the
//! post-game saves.

use super::*;

/// One thread writes every batch in order, so two writes of the same
/// `*_last.png` never overlap. The queue is bounded by [`PREGAME_QUEUE`].
pub(crate) struct PregameWriter {
    tx: Option<std::sync::mpsc::SyncSender<WriterJob>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The thread's end was logged (once).
    dead_logged: bool,
}

impl PregameWriter {
    pub(crate) fn start(root: std::path::PathBuf) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterJob>(PREGAME_QUEUE);
        let thread = std::thread::Builder::new()
            .name("pregame-writer".into())
            .spawn(move || {
                // Game folders in the order they were made, oldest first.
                let mut made = load_pregame_order(&root);
                // Staged frames of a run that ended mid-game.
                sweep_staged(&root, None);
                for job in rx {
                    write_pregame_ops(&root, &job.shots, PREGAME_KEEP_GAMES, &mut made);
                    if let Some(note) = &job.note {
                        write_note(&root, note, PREGAME_KEEP_GAMES, &mut made);
                    }
                }
            })?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            dead_logged: false,
        })
    }

    /// Queue `shots`; false when the queue is full or the writer is gone,
    /// and then none of the batch is written. A full queue is logged at
    /// debug each time; a writer thread that ended (a panic) is logged
    /// once at warn, and no later batch is sent.
    pub(crate) fn send(&mut self, shots: ShotBatch) -> bool {
        self.send_job(WriterJob { shots, note: None })
    }

    /// [`Self::send`] for shots plus a text file written after them.
    pub(crate) fn send_job(&mut self, job: WriterJob) -> bool {
        if job.shots.is_empty() && job.note.is_none() {
            return true;
        }
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        match tx.try_send(job) {
            Ok(()) => true,
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                tracing::debug!(target: "scuffed_stat_tracker", "saved screens dropped (pre- or post-game): writer busy");
                false
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                if !self.dead_logged {
                    self.dead_logged = true;
                    tracing::warn!(target: "scuffed_stat_tracker", "screen writer thread ended: the screens around each game are no longer saved this run");
                }
                false
            }
        }
    }

    /// Whether the writer thread still runs: not when it ended (a panic),
    /// whether or not a send found that out yet.
    pub(crate) fn alive(&self) -> bool {
        !self.dead_logged
            && self.tx.is_some()
            && self.thread.as_ref().is_none_or(|t| !t.is_finished())
    }

    /// [`Self::send_job`], retried every 20 ms while the queue is full, up
    /// to `within`: the post-game window's final note at shutdown.
    pub(crate) async fn send_wait(&mut self, mut job: WriterJob, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            let Some(tx) = self.tx.as_ref() else {
                return false;
            };
            match tx.try_send(job) {
                Ok(()) => return true,
                Err(std::sync::mpsc::TrySendError::Full(back)) if Instant::now() < deadline => {
                    job = back;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(_) => return false,
            }
        }
    }

    /// Whether the writer thread ended this run is logged (tests).
    #[cfg(test)]
    pub(crate) fn dead_logged(&self) -> bool {
        self.dead_logged
    }

    /// A writer whose queue nobody reads, so it fills (tests). Keep the
    /// receiver alive.
    #[cfg(test)]
    pub(crate) fn stalled() -> (Self, std::sync::mpsc::Receiver<WriterJob>) {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterJob>(PREGAME_QUEUE);
        let writer = Self {
            tx: Some(tx),
            thread: None,
            dead_logged: false,
        };
        (writer, rx)
    }

    /// A writer whose thread is already gone (tests).
    #[cfg(test)]
    pub(crate) fn dead() -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<WriterJob>(PREGAME_QUEUE);
        let thread = std::thread::spawn(move || drop(rx));
        while !thread.is_finished() {
            std::thread::yield_now();
        }
        Self {
            tx: Some(tx),
            thread: Some(thread),
            dead_logged: false,
        }
    }

    /// Close the queue and wait up to `within` for the queued writes.
    /// True when they all finished.
    pub(crate) async fn finish(mut self, within: Duration) -> bool {
        drop(self.tx.take());
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let deadline = Instant::now() + within;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                tracing::debug!(target: "scuffed_stat_tracker", "pre-game writes still running at shutdown");
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        thread.join().is_ok()
    }
}
