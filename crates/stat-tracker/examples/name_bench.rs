//! names-v1 reader cost on a folder of boards. Prints counts and timings
//! only, never a name.
//!
//! cargo run --release --example name_bench -- <pack_dir> <team 5|6> <board_dir>... [--tesseract]
//!
//! `<pack_dir>` is an installed names-v1 folder (`<data_dir>/templates/names-v1`).
//! Boards are cropped scoreboards or full frames (cropped first). The
//! names-v1 read runs on one thread; `--tesseract` also times today's ocr-v1
//! name path on the same boards (its pool, so its CPU time is every thread's).

use stat_tracker::names::{self, Abstain, NamePack};
use stat_tracker::ocr;
use std::time::Instant;

fn thread_cpu_ms() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid out pointer, a clock id every Linux has.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 * 1e3 + ts.tv_nsec as f64 / 1e6
}

fn usage() -> libc::rusage {
    // SAFETY: getrusage fills the struct it is given.
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
        u
    }
}

fn process_cpu_ms() -> f64 {
    let u = usage();
    let tv = |t: libc::timeval| t.tv_sec as f64 * 1e3 + t.tv_usec as f64 / 1e3;
    tv(u.ru_utime) + tv(u.ru_stime)
}

fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn stats(mut v: Vec<f64>) -> String {
    if v.is_empty() {
        return "n/a".into();
    }
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    format!(
        "median {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        q(0.5),
        q(0.95),
        v[v.len() - 1]
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tesseract = args.iter().any(|a| a == "--tesseract");
    let args: Vec<&String> = args.iter().filter(|a| *a != "--tesseract").collect();
    if args.len() < 3 {
        eprintln!("usage: name_bench <pack_dir> <team 5|6> <board_dir>... [--tesseract]");
        std::process::exit(2);
    }
    let team: usize = args[1].parse().expect("team 5 or 6");
    let rss0 = rss_kb();
    let t = Instant::now();
    let pack = match NamePack::load(std::path::Path::new(args[0])) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    let rss1 = rss_kb();
    println!(
        "pack: version {}, {} templates, {:.1} MB resident (by size of its tables), loaded in {load_ms:.0} ms",
        pack.version,
        pack.templates(),
        pack.resident_bytes() as f64 / 1e6
    );
    if let (Some(a), Some(b)) = (rss0, rss1) {
        println!(
            "process RSS: {:.1} MB before the pack, {:.1} MB after",
            a as f64 / 1024.0,
            b as f64 / 1024.0
        );
    }
    let mut paths = Vec::new();
    for dir in &args[2..] {
        let mut p: Vec<_> = std::fs::read_dir(dir)
            .expect("board dir")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
            .collect();
        p.sort();
        paths.extend(p);
    }
    let (mut rows, mut read, mut low, mut unacc) = (0usize, 0usize, 0usize, 0usize);
    let (mut no_line, mut accent, mut space) = (0usize, 0usize, 0usize);
    let (mut wall, mut cpu, mut t_wall, mut t_cpu) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut t_calls, mut t_text) = (0u64, 0usize);
    for p in &paths {
        let img = image::DynamicImage::ImageRgb8(image::open(p).expect("png").to_rgb8());
        let board = if img.width() as f64 / img.height() as f64 > 1.72 {
            ocr::preprocess::crop_scoreboard(&img)
        } else {
            img
        };
        let (w0, c0) = (Instant::now(), thread_cpu_ms());
        let raws: Vec<_> = (0..team * 2)
            .filter_map(|i| ocr::preprocess::crop_player_row(&board, i, team))
            .map(|row| names::read_row_raw(&row, board.height(), team, &pack))
            .collect();
        wall.push(w0.elapsed().as_secs_f64() * 1e3);
        cpu.push(thread_cpu_ms() - c0);
        for raw in raws {
            rows += 1;
            match raw.text {
                Err(Abstain::NoLine) => no_line += 1,
                Err(Abstain::Accent) => accent += 1,
                Err(Abstain::Space) => space += 1,
                Ok(_) if raw.unaccounted > 0 => unacc += 1,
                Ok(_) => {
                    if names::accept(raw, &pack).is_some() {
                        read += 1
                    } else {
                        low += 1
                    }
                }
            }
        }
        if tesseract {
            let (w0, c0, n0) = (
                Instant::now(),
                process_cpu_ms(),
                ocr::gate::tesseract_calls(),
            );
            let r = ocr::recognize_scoreboard_names_pre_cropped(&board, team);
            t_wall.push(w0.elapsed().as_secs_f64() * 1e3);
            t_cpu.push(process_cpu_ms() - c0);
            t_calls += ocr::gate::tesseract_calls() - n0;
            t_text += r
                .iter()
                .filter(|x| x.name.as_ref().is_some_and(|n| !n.value.is_empty()))
                .count();
        }
    }
    println!("boards {}, team size {team}, rows {rows}", paths.len());
    println!(
        "names-v1: named {read}, abstained {} (no name line {no_line}, accent {accent}, maybe-space {space}, unexplained ink {unacc}, low confidence {low})",
        rows - read
    );
    println!("names-v1 per board, one thread: wall {}", stats(wall));
    println!("names-v1 per board, one thread: cpu  {}", stats(cpu));
    if tesseract {
        println!("ocr-v1 names per board: wall {}", stats(t_wall));
        println!(
            "ocr-v1 names per board: cpu  {} (all threads)",
            stats(t_cpu)
        );
        println!("ocr-v1: {t_calls} Tesseract calls, {t_text} rows with text in window 0");
    }
    println!("peak RSS: {:.1} MB", usage().ru_maxrss as f64 / 1024.0);
}
