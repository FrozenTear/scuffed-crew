//! Retries for Tab captures that the scoreboard preflight rejected.
//!
//! On 2026-10-15 several Tab frames were taken while the game was still
//! drawing the scoreboard. The preflight found only one or two rows
//! (`dip_count` 1 to 2) and rejected them as "not a scoreboard". A later Tab
//! press was accepted, but one game got no accepted board at all and was
//! lost. A rejected Tab capture now captures again after a short wait, at
//! about 300 ms and about 700 ms after the first rejection, before giving up.
//! Each retry is one screen grab plus the cheap preflight, so CPU stays light.

use std::future::Future;
use std::time::Duration;

/// Waits before each retry, counted from the previous attempt. The retries
/// land about 300 ms and about 700 ms after the first rejection.
pub const TAB_RETRY_WAITS: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(400)];

/// Runs `attempt` once, and again after each wait in `waits` for as long as
/// `retry` says the best result so far was rejected by the scoreboard
/// check. A later result replaces the kept one only when `rank` scores it
/// higher, so a rejected grab never replaces a more useful one. Returns the
/// best result and how many retries ran. An error from `attempt` (a failed
/// screen grab) ends the loop at once.
pub async fn with_retries<T, E, A, AF, S, SF>(
    waits: &[Duration],
    retry: impl Fn(&T) -> bool,
    rank: impl Fn(&T) -> u8,
    mut attempt: A,
    mut sleep: S,
) -> Result<(T, usize), E>
where
    A: FnMut(usize) -> AF,
    AF: Future<Output = Result<T, E>>,
    S: FnMut(Duration) -> SF,
    SF: Future<Output = ()>,
{
    let mut best = attempt(0).await?;
    let mut retries = 0;
    for &wait in waits {
        if !retry(&best) {
            break;
        }
        retries += 1;
        tracing::info!(
            retry = retries,
            of = waits.len(),
            wait_ms = wait.as_millis() as u64,
            "Tab capture rejected by the scoreboard check: capturing again (the board may still be drawing)"
        );
        sleep(wait).await;
        let next = attempt(retries).await?;
        if rank(&next) > rank(&best) {
            best = next;
        }
    }
    if retries > 0 {
        if retry(&best) {
            tracing::warn!(
                retries,
                "Tab capture still rejected after retries: giving up on this Tab press"
            );
        } else {
            tracing::info!(retries, "Tab capture accepted on retry");
        }
    }
    Ok((best, retries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::hero_portrait::{ScoreboardPreflight, preflight_scoreboard};
    use crate::ocr::preprocess::{crop_scoreboard, crop_scoreboard_rect};
    use crate::report::blank::synth::{self, Layout};
    use image::{DynamicImage, Rgb, RgbImage};
    use std::cell::RefCell;

    /// The synthetic 6v6 frame (`report::blank::synth`, no member pixels).
    fn full_6v6() -> RgbImage {
        synth::frame(Layout::L1, &synth::VALUES).to_rgb8()
    }

    /// Minimal synthetic 5v5 frame: the synth board's header, colours and
    /// name strokes with five rows per team at the 5v5 pitch (six 75 px
    /// rows' height over five rows, 90 px). Stands in until Scuffed Vision's
    /// 5v5 board generator lands on main.
    // TODO: switch to `synth::tab_frame(teams, width, drawn)` once PR #285 lands.
    fn full_5v5() -> RgbImage {
        let mut f = RgbImage::from_fn(2560, 1440, |x, y| {
            Rgb([(30 + x * 40 / 2560) as u8, (45 + y * 40 / 1440) as u8, 60])
        });
        let (bx, by, _, _) = crop_scoreboard_rect(2560, 1440);
        let (l, r) = Layout::L1.edges();
        let mut b = RgbImage::from_fn(synth::W, synth::H, |x, y| {
            Rgb([
                (40 + (x * 60 / synth::W) as u8),
                (55 + (y * 50 / synth::H) as u8),
                (70 + ((x + y) * 40 / (synth::W + synth::H)) as u8),
            ])
        });
        let fill = |img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgb<u8>| {
            for y in y0..y1.min(img.height()) {
                for x in x0..x1.min(img.width()) {
                    img.put_pixel(x, y, c);
                }
            }
        };
        fill(&mut b, l, 0, r, synth::T1, Rgb([215, 219, 226]));
        let pitch = 90;
        let row_h = pitch - 2;
        for row in 0..10u32 {
            let team2 = row >= 5;
            let top = if team2 { synth::T2 } else { synth::T1 } + pitch * (row % 5);
            let bot = (top + row_h).min(synth::H);
            let (bar, tint) = if team2 {
                (synth::BAR_T2, synth::TINT_T2)
            } else {
                (synth::BAR_T1, synth::TINT_T1)
            };
            fill(&mut b, l + 104, top, r, bot, tint);
            fill(&mut b, l, top, l + 31, bot, bar);
            for j in 0..7u32 {
                let x = l + 120 + j * 14;
                fill(&mut b, x, top + 28, x + 3, top + 50, Rgb([238, 238, 238]));
                fill(&mut b, x, top + 28, x + 10, top + 31, Rgb([238, 238, 238]));
            }
        }
        image::imageops::replace(&mut f, &b, i64::from(bx), i64::from(by));
        f
    }

    /// `full` as the game shows it mid-draw: only the top `rows_drawn` rows
    /// of team 1 are on screen, the rest of the board area still shows the
    /// world behind it.
    fn half_drawn(full: &RgbImage, rows_drawn: u32, pitch: u32) -> RgbImage {
        let mut f = full.clone();
        let (bx, by, bw, bh) = crop_scoreboard_rect(full.width(), full.height());
        let cut = by + synth::T1 + pitch * rows_drawn;
        for y in cut..by + bh {
            for x in bx..bx + bw {
                f.put_pixel(
                    x,
                    y,
                    Rgb([(30 + x * 40 / 2560) as u8, (45 + y * 40 / 1440) as u8, 60]),
                );
            }
        }
        f
    }

    fn preflight(f: &RgbImage) -> ScoreboardPreflight {
        preflight_scoreboard(&crop_scoreboard(&DynamicImage::ImageRgb8(f.clone()))).0
    }

    /// Feeds `frames` to [`with_retries`] as successive screen grabs and
    /// returns the final verdict, the retry count and the waits slept.
    fn run(frames: Vec<RgbImage>) -> (ScoreboardPreflight, usize, Vec<Duration>, usize) {
        let frames = RefCell::new(frames.into_iter());
        let grabs = RefCell::new(0usize);
        let slept = RefCell::new(Vec::new());
        let (verdict, retries) = block_on(with_retries(
            &TAB_RETRY_WAITS,
            |p: &ScoreboardPreflight| *p == ScoreboardPreflight::NotAScoreboard,
            |p: &ScoreboardPreflight| match p {
                ScoreboardPreflight::NotAScoreboard => 0,
                ScoreboardPreflight::TeamSizeUncertain => 1,
                ScoreboardPreflight::Ready(_) => 2,
            },
            |_| {
                *grabs.borrow_mut() += 1;
                let f = frames.borrow_mut().next().expect("a frame per grab");
                async move { Ok::<_, ()>(preflight(&f)) }
            },
            |d| {
                slept.borrow_mut().push(d);
                async {}
            },
        ))
        .unwrap();
        (verdict, retries, slept.into_inner(), grabs.into_inner())
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn check(full: RgbImage, pitch: u32, size: usize) {
        assert_eq!(
            preflight(&full),
            ScoreboardPreflight::Ready(size),
            "full board"
        );
        let half = half_drawn(&full, 2, pitch);
        assert_eq!(
            preflight(&half),
            ScoreboardPreflight::NotAScoreboard,
            "half-drawn board"
        );

        // Rejected, then the first retry sees the finished board.
        let (v, retries, slept, grabs) = run(vec![half.clone(), full.clone()]);
        assert_eq!(v, ScoreboardPreflight::Ready(size));
        assert_eq!((retries, grabs), (1, 2));
        assert_eq!(slept, vec![Duration::from_millis(300)]);

        // Still drawing at 300 ms, done by 700 ms.
        let (v, retries, slept, _) = run(vec![half.clone(), half.clone(), full.clone()]);
        assert_eq!(v, ScoreboardPreflight::Ready(size));
        assert_eq!(retries, 2);
        assert_eq!(slept.iter().sum::<Duration>(), Duration::from_millis(700));

        // Never finished: gives up after two retries.
        let (v, retries, _, grabs) = run(vec![half.clone(), half.clone(), half]);
        assert_eq!(v, ScoreboardPreflight::NotAScoreboard);
        assert_eq!((retries, grabs), (2, 3));

        // An accepted first grab never retries.
        let (_, retries, slept, grabs) = run(vec![full]);
        assert_eq!((retries, grabs), (0, 1));
        assert!(slept.is_empty());
    }

    #[test]
    fn half_drawn_6v6_board_is_rejected_then_accepted_on_retry() {
        check(full_6v6(), synth::PITCH, 6);
    }

    #[test]
    fn half_drawn_5v5_board_is_rejected_then_accepted_on_retry() {
        check(full_5v5(), 90, 5);
    }

    #[test]
    fn a_later_rejected_grab_never_replaces_a_better_result() {
        // 0 = rejected (retry), 1 = useful but not retried, 2 = accepted.
        let (best, retries) = block_on(with_retries(
            &TAB_RETRY_WAITS,
            |v: &u8| *v == 0,
            |v: &u8| *v,
            |i| async move { Ok::<_, ()>(if i == 0 { 0 } else { 1 }) },
            |_| async {},
        ))
        .unwrap();
        assert_eq!((best, retries), (1, 1), "stops at the useful result");

        // Even if the predicate kept retrying, rank keeps the best result.
        let seq = RefCell::new(vec![1u8, 0, 0].into_iter());
        let (best, retries) = block_on(with_retries(
            &TAB_RETRY_WAITS,
            |_: &u8| true,
            |v: &u8| *v,
            |_| {
                let v = seq.borrow_mut().next().unwrap();
                async move { Ok::<_, ()>(v) }
            },
            |_| async {},
        ))
        .unwrap();
        assert_eq!(
            (best, retries),
            (1, 2),
            "a rejected grab never replaces the kept one"
        );
    }
}
