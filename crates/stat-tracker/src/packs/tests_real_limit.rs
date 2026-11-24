//! Against the site's real rate limit: the simulated bucket and clock, and
//! the first installs and races they run.

use super::*;
use scuffed_types::{PACK_IP_BURST, PACK_IP_REPLENISH_SECS};

// ---- P1c fix round: against the site's real rate limit (F268-1) ----
// The fake below keeps the bucket the way the site's limiter does, on a
// clock that only the trackers' own waits move.

/// The site's pack rate limit as governor 0.10 keeps it (GCRA):
/// [`PACK_IP_BURST`] in a burst, one more every
/// [`PACK_IP_REPLENISH_SECS`] s, per address. A refused request costs
/// nothing and is told the whole seconds to the next allowed time, plus
/// one (`governor_error_response` in site-server).
struct Bucket {
    /// The limiter's "theoretical arrival time". `None` until the
    /// first request.
    tat: Option<Duration>,
}

impl Bucket {
    const EVERY: Duration = Duration::from_secs(PACK_IP_REPLENISH_SECS);
    const BURST: u32 = PACK_IP_BURST;

    /// A bucket with `left` requests in it at time zero.
    fn with_left(left: u32) -> Self {
        Self {
            tat: Some(Self::EVERY * (Self::BURST - left)),
        }
    }

    /// `Ok` when the request is let through, `Err(retry_after)` in
    /// whole seconds when it is not.
    fn ask(&mut self, now: Duration) -> Result<(), u64> {
        let tau = Self::EVERY * (Self::BURST - 1);
        let tat = self.tat.unwrap_or(now);
        let earliest = tat.saturating_sub(tau);
        if now < earliest {
            return Err((earliest - now).as_secs() + 1);
        }
        self.tat = Some(tat.max(now) + Self::EVERY);
        Ok(())
    }
}

#[test]
fn the_bucket_model_is_the_burst_then_one_every_ten_seconds() {
    let mut bucket = Bucket { tat: None };
    let at = Duration::from_secs_f64;
    for request in 0..Bucket::BURST {
        assert_eq!(bucket.ask(at(0.0)), Ok(()), "request {request}");
    }
    // Empty: the next one is allowed at 10 s. Asking does not cost.
    assert_eq!(bucket.ask(at(0.0)), Err(11));
    assert_eq!(bucket.ask(at(3.0)), Err(8));
    assert_eq!(bucket.ask(at(9.5)), Err(1));
    assert_eq!(bucket.ask(at(10.0)), Ok(()));
    assert_eq!(bucket.ask(at(10.0)), Err(11));
    assert_eq!(bucket.ask(at(12.0)), Err(9));
    assert_eq!(bucket.ask(at(20.0)), Ok(()));
    // The burst plus the two refills above is 18 requests, so the bucket
    // is whole again 10 s after the last one was let through.
    let whole = (f64::from(Bucket::BURST) + 3.0) * 10.0;
    for request in 0..Bucket::BURST {
        assert_eq!(bucket.ask(at(whole)), Ok(()), "request {request}");
    }
    assert_eq!(bucket.ask(at(whole)), Err(11));
    // The starting states the tests use.
    assert_eq!(Bucket::with_left(Bucket::BURST).ask(at(0.0)), Ok(()));
    let mut four = Bucket::with_left(4);
    for request in 0..4 {
        assert_eq!(four.ask(at(0.0)), Ok(()), "request {request}");
    }
    assert_eq!(four.ask(at(0.0)), Err(11));
    assert_eq!(Bucket::with_left(0).ask(at(0.0)), Err(11));
    assert_eq!(Bucket::with_left(0).ask(at(10.0)), Ok(()));
}

/// Time for trackers that share one bucket. Each tracker is an actor.
/// The clock jumps to the next wake-up once every actor is asleep or
/// done, so no wait is slept for real and the order of events is the
/// order of the waits.
struct SimClock {
    state: Mutex<SimState>,
    moved: tokio::sync::Notify,
}

struct SimState {
    now: Duration,
    /// Actors that are neither asleep nor done.
    awake: usize,
    /// When each sleeping actor wakes.
    wakes: Vec<Duration>,
    /// The run is given up when the clock would pass this.
    limit: Duration,
    /// How many waits were asked for. A job that asks for more than
    /// [`SimClock::MAX_SLEEPS`] is spinning, and the run is given up.
    sleeps: usize,
    ran_out: bool,
}

impl SimState {
    /// Jump to the next wake-up if nobody is awake.
    fn settle(&mut self) {
        if self.awake > 0 || self.ran_out {
            return;
        }
        let Some(next) = self.wakes.iter().min().copied() else {
            return;
        };
        if next > self.limit {
            self.ran_out = true;
            return;
        }
        self.now = next;
        let asleep = self.wakes.len();
        self.wakes.retain(|wake| *wake > next);
        self.awake += asleep - self.wakes.len();
    }
}

impl SimClock {
    const MAX_SLEEPS: usize = 1000;

    fn new(actors: usize, limit: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(SimState {
                now: Duration::ZERO,
                awake: actors,
                wakes: Vec::new(),
                limit,
                sleeps: 0,
                ran_out: false,
            }),
            moved: tokio::sync::Notify::new(),
        })
    }

    fn now(&self) -> Duration {
        self.state.lock().expect("clock").now
    }

    /// The calling actor sleeps for `wait` of this clock's time.
    fn sleep(self: &Arc<Self>, wait: Duration) -> Nap {
        let wake = {
            let mut state = self.state.lock().expect("clock");
            let wake = state.now + wait;
            state.wakes.push(wake);
            state.awake -= 1;
            state.sleeps += 1;
            if state.sleeps > Self::MAX_SLEEPS {
                state.ran_out = true;
            }
            state.settle();
            wake
        };
        self.moved.notify_waiters();
        let clock = Arc::clone(self);
        Box::pin(async move {
            loop {
                let moved = clock.moved.notified();
                tokio::pin!(moved);
                let _ = moved.as_mut().enable();
                {
                    let state = clock.state.lock().expect("clock");
                    if !state.ran_out && state.now >= wake && !state.wakes.contains(&wake) {
                        return;
                    }
                }
                moved.await;
            }
        })
    }

    /// The calling actor is done for good.
    fn actor_done(&self) {
        {
            let mut state = self.state.lock().expect("clock");
            state.awake -= 1;
            state.settle();
        }
        self.moved.notify_waiters();
    }

    /// Resolves when the clock gave up at its limit.
    async fn ran_out(&self) {
        loop {
            let moved = self.moved.notified();
            tokio::pin!(moved);
            let _ = moved.as_mut().enable();
            if self.state.lock().expect("clock").ran_out {
                return;
            }
            moved.await;
        }
    }
}

/// A site with all ten packs behind the real rate limit, and every
/// request it saw: when, which path, which status.
struct LimitedSite {
    base: String,
    seen: Arc<Mutex<Vec<(Duration, String, u16)>>>,
}

impl LimitedSite {
    fn statuses(&self) -> Vec<u16> {
        let seen = self.seen.lock().expect("seen");
        seen.iter().map(|(_, _, status)| *status).collect()
    }

    fn paths(&self) -> Vec<String> {
        let seen = self.seen.lock().expect("seen");
        seen.iter().map(|(_, path, _)| path.clone()).collect()
    }

    fn times(&self) -> Vec<Duration> {
        let seen = self.seen.lock().expect("seen");
        seen.iter().map(|(at, _, _)| *at).collect()
    }

    fn refused(&self) -> usize {
        self.statuses()
            .iter()
            .filter(|status| **status == 429)
            .count()
    }
}

fn serve_limited(clock: &Arc<SimClock>, bucket: Bucket) -> (LimitedSite, PackContents) {
    let (list, files, contents) = all_packs_for_script();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (clock, bucket, log) = (Arc::clone(clock), Mutex::new(bucket), seen.clone());
    let served = serve_script(move |path, _| {
        let now = clock.now();
        let answer = match bucket.lock().expect("bucket").ask(now) {
            Ok(()) => healthy(&list, &files, path),
            Err(wait) => {
                let body = format!(r#"{{"error":"rate_limited","retry_after":{wait}}}"#);
                Some((429, body.into_bytes(), Some(wait)))
            }
        };
        let status = answer.as_ref().map_or(0, |(status, _, _)| *status);
        log.lock()
            .expect("seen")
            .push((now, path.to_string(), status));
        answer
    });
    let site = LimitedSite {
        base: served.base,
        seen,
    };
    (site, contents)
}

/// One tracker on the simulated clock: it starts after `start_after`,
/// runs its startup sync and then its job to the end. The second value
/// collects the waits the job asked for.
fn tracker(
    clock: &Arc<SimClock>,
    site: &LimitedSite,
    data_dir: &Path,
    start_after: Duration,
    jitter: Duration,
) -> (Nap, Arc<Mutex<Vec<Duration>>>) {
    let (clock, base, data_dir) = (Arc::clone(clock), site.base.clone(), data_dir.to_path_buf());
    let waits = Arc::new(Mutex::new(Vec::new()));
    let asked = waits.clone();
    let run = async move {
        if !start_after.is_zero() {
            clock.sleep(start_after).await;
        }
        let startup = sync_on_startup_capped(&base, TOKEN, &data_dir, PACK_MAX_BYTES).await;
        let server_url = base.clone();
        let mut keeper = PackKeeper::new(&data_dir, Some(startup), move || {
            Some(PackCredentials {
                server_url: server_url.clone(),
                token: TOKEN.to_string(),
            })
        });
        keeper.jitter = jitter;
        let napper = Arc::clone(&clock);
        keeper.sleep = Box::new(move |wait| {
            asked.lock().expect("waits").push(wait);
            napper.sleep(wait)
        });
        keeper.run().await;
        clock.actor_done();
    };
    (Box::pin(run), waits)
}

/// Run the trackers to the end. `false` when the clock reached its
/// limit, or a tracker asked for too many waits, with one still not
/// done.
async fn run_trackers(clock: &Arc<SimClock>, trackers: Vec<Nap>) -> bool {
    let handles: Vec<_> = trackers.into_iter().map(tokio::spawn).collect();
    let all = async {
        for handle in handles {
            handle.await.expect("tracker");
        }
    };
    tokio::select! {
        () = all => true,
        () = clock.ran_out() => false,
    }
}

const SIM_LIMIT: Duration = Duration::from_secs(3600);

#[tokio::test]
async fn against_the_real_limit_one_first_install_is_eleven_requests_and_none_is_refused() {
    let clock = SimClock::new(1, SIM_LIMIT);
    let (site, contents) = serve_limited(&clock, Bucket::with_left(Bucket::BURST));
    let dir = tempfile::tempdir().expect("tempdir");
    let (tracker, waits) = tracker(&clock, &site, dir.path(), Duration::ZERO, TEST_JITTER);
    assert!(run_trackers(&clock, vec![tracker]).await);
    // The list and ten packs are eleven requests and the bucket holds
    // the burst of 16: none is refused, the job never waits and the
    // clock never moves.
    const { assert!(Bucket::BURST > 11, "the burst must hold a first install") };
    assert!(waits.lock().unwrap().is_empty());
    assert_all_installed(dir.path(), &contents);
    assert_eq!(site.paths(), list_then(&pack_names()));
    assert_eq!(site.statuses(), vec![200; 11]);
    assert_eq!(site.refused(), 0);
    assert_eq!(clock.now(), Duration::ZERO);
}

/// F268-1 as reviewed: the bucket runs out in the middle of a first
/// install. The refused download is the next request, not the list, and
/// the rest follow one every 10 s.
#[tokio::test]
async fn against_the_real_limit_a_429_in_the_middle_is_followed_by_the_downloads() {
    let clock = SimClock::new(1, SIM_LIMIT);
    let (site, contents) = serve_limited(&clock, Bucket::with_left(4));
    let dir = tempfile::tempdir().expect("tempdir");
    let (tracker, waits) = tracker(&clock, &site, dir.path(), Duration::ZERO, TEST_JITTER);
    assert!(
        run_trackers(&clock, vec![tracker]).await,
        "the job must end; it asked for {:?}",
        site.paths()
    );
    assert_all_installed(dir.path(), &contents);
    // The list and three packs fit. bans-v1 is refused with 11 s.
    let mut expected = list_then(&pack_names()[..3]);
    expected.extend(tar_paths(&["bans-v1.tar"]));
    expected.extend(tar_paths(&pack_names()[3..]));
    assert_eq!(site.paths(), expected);
    let mut statuses = vec![200; 4];
    statuses.push(429);
    statuses.extend([200; 7]);
    assert_eq!(site.statuses(), statuses);
    let mut expected_waits = vec![secs(11) + TEST_JITTER];
    expected_waits.extend([secs(10); 6]);
    assert_eq!(*waits.lock().unwrap(), expected_waits);
    let expected_times: Vec<Duration> = (0..7).map(|k| ms(11_750 + k * 10_000)).collect();
    assert_eq!(site.times()[5..], expected_times);
    assert_eq!(clock.now(), ms(71_750));
}

#[tokio::test]
async fn against_the_real_limit_a_bucket_that_starts_empty_refills_one_request_every_ten_seconds() {
    let clock = SimClock::new(1, SIM_LIMIT);
    let (site, contents) = serve_limited(&clock, Bucket::with_left(0));
    let dir = tempfile::tempdir().expect("tempdir");
    let (tracker, waits) = tracker(&clock, &site, dir.path(), Duration::ZERO, TEST_JITTER);
    assert!(
        run_trackers(&clock, vec![tracker]).await,
        "the job must end; it asked for {:?}",
        site.paths()
    );
    assert_all_installed(dir.path(), &contents);
    // The refused list, the list, then the ten packs.
    let mut expected = vec![LIST_PATH.to_string()];
    expected.extend(list_then(&pack_names()));
    assert_eq!(site.paths(), expected);
    assert_eq!(site.refused(), 1);
    let mut expected_waits = vec![secs(11) + TEST_JITTER];
    expected_waits.extend([secs(10); 10]);
    assert_eq!(*waits.lock().unwrap(), expected_waits);
    // One request every 10 s, each 1.75 s after the bucket gave it.
    let times = site.times();
    assert_eq!(times[1], ms(11_750));
    assert!(
        times[1..]
            .windows(2)
            .all(|pair| pair[1] - pair[0] == secs(10))
    );
    assert_eq!(clock.now(), ms(111_750));
}

#[tokio::test]
async fn against_the_real_limit_two_trackers_behind_one_address_both_finish() {
    let clock = SimClock::new(2, SIM_LIMIT);
    // Other traffic from the same address has already used up half of the
    // burst, so 8 requests are left: this is the contention case, not a
    // first install on a full bucket.
    let (site, contents) = serve_limited(&clock, Bucket::with_left(Bucket::BURST / 2));
    let (dir_a, dir_b) = (
        tempfile::tempdir().expect("tempdir"),
        tempfile::tempdir().expect("tempdir"),
    );
    // B starts 3 s after A, as a second machine on release night would.
    let (a, waits_a) = tracker(&clock, &site, dir_a.path(), Duration::ZERO, TEST_JITTER);
    let (b, waits_b) = tracker(&clock, &site, dir_b.path(), secs(3), TEST_JITTER);
    assert!(
        run_trackers(&clock, vec![a, b]).await,
        "both jobs must end; they asked for {:?}",
        site.paths()
    );
    assert_all_installed(dir_a.path(), &contents);
    assert_all_installed(dir_b.path(), &contents);
    // A takes the 8 requests the bucket has left, and its next request
    // (its eighth pack) is refused. B's list is refused too, 3 s in. Both
    // then come back when the bucket refills, B behind A, so A backs off
    // 10.75, 20.75, 40.75 and 80.75 s when B takes the request it wanted,
    // and then fetches its last two packs 10 s apart.
    let mut expected_a = vec![ms(11_750), ms(10_750), ms(20_750), ms(40_750), ms(80_750)];
    expected_a.extend([secs(10); 2]);
    assert_eq!(*waits_a.lock().unwrap(), expected_a);
    let mut expected_b = vec![ms(8_750)];
    expected_b.extend([secs(10); 10]);
    assert_eq!(*waits_b.lock().unwrap(), expected_b);
    // 22 requests are needed (two lists, twenty packs): 6 are refused.
    let (requests, refused) = (site.statuses().len(), site.refused());
    assert_eq!((requests - refused, refused), (22, 6));
    let mut statuses = vec![200; 8];
    statuses.extend([429, 429, 200, 429, 200, 429, 200, 200, 429]);
    statuses.extend([200, 200, 200, 200, 429, 200, 200, 200, 200, 200, 200]);
    assert_eq!(site.statuses(), statuses);
    assert_eq!(site.paths().len(), 28);
    assert_eq!(clock.now(), ms(184_750));
}

/// Both trackers want the same refilled request every 10 s. The one
/// that comes second backs off further each time, so the first gets
/// through, and then the second.
#[tokio::test]
async fn against_the_real_limit_two_trackers_on_an_empty_bucket_both_finish() {
    let clock = SimClock::new(2, SIM_LIMIT);
    let (site, contents) = serve_limited(&clock, Bucket::with_left(0));
    let (dir_a, dir_b) = (
        tempfile::tempdir().expect("tempdir"),
        tempfile::tempdir().expect("tempdir"),
    );
    let (a, waits_a) = tracker(&clock, &site, dir_a.path(), Duration::ZERO, ms(750));
    let (b, waits_b) = tracker(&clock, &site, dir_b.path(), Duration::ZERO, ms(2_250));
    assert!(
        run_trackers(&clock, vec![a, b]).await,
        "both jobs must end; they asked for {:?}",
        site.paths()
    );
    assert_all_installed(dir_a.path(), &contents);
    assert_all_installed(dir_b.path(), &contents);
    let (requests, refused, took) = (site.statuses().len(), site.refused(), clock.now());
    println!(
        "two trackers, empty bucket: {requests} requests, {refused} refused, done after {took:?}; \
         waits A {:?}, waits B {:?}",
        waits_a.lock().unwrap(),
        waits_b.lock().unwrap()
    );
    // 22 requests are needed (two lists, twenty packs). With ten packs a
    // tracker's first install is a refused list, a list and ten
    // downloads, so the two keep meeting at the refill: A is refused once
    // more after its ten requests, then 10.75, 20.75, 40.75 and 80.75 s,
    // and then fetches its last two packs 10 s apart. B waits 12.25,
    // 22.25 and 42.25 s and then has the bucket to itself: 10 refused
    // requests in all, both done after 274.5 s.
    assert_eq!(requests - refused, 22);
    assert_eq!(refused, 10);
    assert_eq!(took, ms(274_500));
    let mut expected_a = vec![ms(11_750)];
    expected_a.extend([secs(10); 8]);
    expected_a.extend([ms(9_750), ms(10_750), ms(20_750), ms(40_750), ms(80_750)]);
    expected_a.extend([secs(10); 2]);
    assert_eq!(*waits_a.lock().unwrap(), expected_a);
    let mut expected_b = vec![ms(13_250), ms(12_250), ms(22_250), ms(42_250)];
    expected_b.extend([secs(10); 10]);
    assert_eq!(*waits_b.lock().unwrap(), expected_b);
}
