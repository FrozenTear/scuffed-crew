//! The layering the stat-tracker lib split (0.6 N4) cuts along, pinned.
//!
//! The lib is to become six crates. Each module below belongs to one of
//! them ([`CRATE_OF`]), and a crate may use only the crates under it:
//!
//! ```text
//!              top: report, reader_apply, tab_retry (+ the bin)
//!          /        |          |           \
//!      game      capture     store       vision
//!   boundary,    capture/*   storage,    ocr, detect, shadow, names,
//!   capture_gate             sync,       packs, mapcrops, report::blank,
//!                            stats       ocr_switch
//!          \        |          |           /
//!     core: config, parse, perf, perf_log, setup, replace, proc_id, fs_mode,
//!           omp, host_probe, sandbox, hero_auth, model, version, suspect,
//!           ocr_status
//! ```
//!
//! The four middle crates do not use each other. The test reads `src/`
//! (the lib only: not `main.rs`, `main_tests/` or `ocr_bench.rs`), follows
//! every `crate::`, `super::` and `self::` path in `use` trees and in code,
//! and fails on any path from a module to a crate it may not use, in
//! production code and in tests alike. A path through a re-export counts
//! where it is written: `crate::detect::MatchOutcome` from `parse` is a
//! back-edge even though the type lives in `model`, because `parse` will
//! not see `detect` once the crates are split.
//!
//! A new top-level module fails [`every_module_has_a_crate`] until it is
//! given one here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The would-be crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Crate {
    Core,
    Game,
    Capture,
    Store,
    Vision,
    Top,
}

impl Crate {
    /// May code in `self` use `to`?
    fn may_use(self, to: Crate) -> bool {
        match self {
            Crate::Core => to == Crate::Core,
            Crate::Game | Crate::Capture | Crate::Store | Crate::Vision => {
                to == self || to == Crate::Core
            }
            Crate::Top => true,
        }
    }
}

/// Module path prefix -> crate. The longest matching prefix wins, so
/// `report::blank` is vision while the rest of `report` is top.
const CRATE_OF: &[(&str, Crate)] = &[
    ("config", Crate::Core),
    ("fs_mode", Crate::Core),
    ("hero_auth", Crate::Core),
    ("host_probe", Crate::Core),
    ("model", Crate::Core),
    ("ocr_status", Crate::Core),
    ("omp", Crate::Core),
    ("parse", Crate::Core),
    ("perf", Crate::Core),
    ("perf_log", Crate::Core),
    ("proc_id", Crate::Core),
    ("replace", Crate::Core),
    ("sandbox", Crate::Core),
    ("setup", Crate::Core),
    ("suspect", Crate::Core),
    ("version", Crate::Core),
    ("boundary", Crate::Game),
    ("capture_gate", Crate::Game),
    ("capture", Crate::Capture),
    ("stats", Crate::Store),
    ("storage", Crate::Store),
    ("sync", Crate::Store),
    ("detect", Crate::Vision),
    ("mapcrops", Crate::Vision),
    ("names", Crate::Vision),
    ("ocr", Crate::Vision),
    ("ocr_switch", Crate::Vision),
    ("packs", Crate::Vision),
    ("report::blank", Crate::Vision),
    ("shadow", Crate::Vision),
    ("reader_apply", Crate::Top),
    ("report", Crate::Top),
    ("tab_retry", Crate::Top),
    ("tests_layering", Crate::Top),
    ("tests_release_pins", Crate::Top),
];

fn crate_of(module: &[String]) -> Option<Crate> {
    if module.is_empty() {
        return Some(Crate::Top);
    }
    let joined = module.join("::");
    CRATE_OF
        .iter()
        .filter(|(prefix, _)| joined == *prefix || joined.starts_with(&format!("{prefix}::")))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, krate)| *krate)
}

// ------------------------------------------------------------------ lexer

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` with comments, string literals and char literals replaced by
/// spaces. Newlines stay, so offsets and line numbers still match.
fn blank(text: &str) -> Vec<char> {
    let src: Vec<char> = text.chars().collect();
    let mut out = src.clone();
    let n = src.len();
    let wipe = |out: &mut Vec<char>, a: usize, b: usize| {
        for c in out.iter_mut().take(b.min(n)).skip(a) {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let mut i = 0;
    while i < n {
        let c = src[i];
        let next = src.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            let end = (i..n).find(|&k| src[k] == '\n').unwrap_or(n);
            wipe(&mut out, i, end);
            i = end;
        } else if c == '/' && next == Some('*') {
            let (mut depth, mut k) = (1, i + 2);
            while k < n && depth > 0 {
                if src[k] == '/' && src.get(k + 1) == Some(&'*') {
                    depth += 1;
                    k += 2;
                } else if src[k] == '*' && src.get(k + 1) == Some(&'/') {
                    depth -= 1;
                    k += 2;
                } else {
                    k += 1;
                }
            }
            wipe(&mut out, i, k);
            i = k;
        } else if let Some(end) = string_end(&src, i) {
            wipe(&mut out, i, end);
            i = end;
        } else if c == '\'' {
            // A char literal ('x', '\n', '\u{..}'), or a lifetime / label.
            let end = if next == Some('\\') {
                // the escaped char may itself be a quote: '\''
                (i + 3..n).find(|&k| src[k] == '\'').map(|k| k + 1)
            } else if src.get(i + 2) == Some(&'\'') {
                Some(i + 3)
            } else {
                None
            };
            match end {
                Some(end) => {
                    wipe(&mut out, i, end);
                    i = end;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The end of the string literal starting at `i` (`"…"`, `b"…"`, `c"…"`,
/// `r#"…"#`, `br"…"`), or `None` when none starts there.
fn string_end(src: &[char], i: usize) -> Option<usize> {
    if i > 0 && is_ident(src[i - 1]) {
        return None;
    }
    let mut k = i;
    if matches!(src.get(k), Some('b') | Some('c')) {
        k += 1;
    }
    let raw = src.get(k) == Some(&'r');
    if raw {
        k += 1;
    }
    let hashes = src[k..].iter().take_while(|c| **c == '#').count();
    k += hashes;
    if src.get(k) != Some(&'"') || (!raw && hashes > 0) {
        return None;
    }
    k += 1;
    while k < src.len() {
        if raw {
            if src[k] == '"'
                && src[k + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|c| **c == '#')
                    .count()
                    == hashes
            {
                return Some(k + 1 + hashes);
            }
            k += 1;
        } else if src[k] == '\\' {
            k += 2;
        } else if src[k] == '"' {
            return Some(k + 1);
        } else {
            k += 1;
        }
    }
    Some(src.len())
}

/// Index of the bracket that closes the one at `open`.
fn closing(t: &[char], open: usize) -> usize {
    let (o, c) = match t[open] {
        '{' => ('{', '}'),
        '(' => ('(', ')'),
        _ => ('[', ']'),
    };
    let mut depth = 0usize;
    for (k, ch) in t.iter().enumerate().skip(open) {
        if *ch == o {
            depth += 1;
        } else if *ch == c {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    t.len().saturating_sub(1)
}

fn starts_with(t: &[char], at: usize, s: &str) -> bool {
    s.chars()
        .enumerate()
        .all(|(k, c)| t.get(at + k) == Some(&c))
}

fn skip_ws(t: &[char], mut k: usize) -> usize {
    while k < t.len() && t[k].is_whitespace() {
        k += 1;
    }
    k
}

/// The end of the item that starts at `k`: its `;` or its closing brace.
fn item_end(t: &[char], mut k: usize) -> usize {
    let mut depth = 0i32;
    while k < t.len() {
        match t[k] {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ';' if depth <= 0 => return k,
            '{' if depth <= 0 => return closing(t, k),
            _ => {}
        }
        k += 1;
    }
    t.len()
}

/// `(start, end)` of every item behind a `#[cfg(...)]` that needs `test`
/// (`cfg(test)`, `cfg(all(test, unix))`, not `cfg(not(test))`).
fn test_regions(t: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if !starts_with(t, i, "#[cfg(") {
            i += 1;
            continue;
        }
        let close = closing(t, i + 1);
        let cfg: String = t[i + 6..close].iter().collect();
        let needs_test = cfg.split(|c: char| !is_ident(c)).any(|word| word == "test")
            && !cfg.replace(' ', "").contains("not(test)");
        let mut k = skip_ws(t, close + 1);
        while starts_with(t, k, "#[") {
            k = skip_ws(t, closing(t, k + 1) + 1);
        }
        if needs_test {
            out.push((i, item_end(t, k)));
        }
        i = close + 1;
    }
    out
}

// ------------------------------------------------------------ module tree

struct Source {
    /// `src/`-relative file name.
    name: String,
    module: Vec<String>,
    /// Reached only through a `#[cfg(test)] mod`.
    test_file: bool,
    raw: Vec<char>,
    text: Vec<char>,
    tests: Vec<(usize, usize)>,
    /// `(open, close)` of each inline `mod name { ... }`.
    inline: Vec<(usize, usize)>,
}

impl Source {
    fn is_test(&self, at: usize) -> bool {
        self.test_file || self.tests.iter().any(|&(a, b)| a <= at && at <= b)
    }

    fn line(&self, at: usize) -> usize {
        self.raw[..at].iter().filter(|c| **c == '\n').count() + 1
    }
}

fn parse_source(name: String, module: Vec<String>, test_file: bool, raw: &str) -> Source {
    let text = blank(raw);
    let tests = test_regions(&text);
    Source {
        name,
        module,
        test_file,
        raw: raw.chars().collect(),
        tests,
        inline: Vec::new(),
        text,
    }
}

/// The `mod name;` / `mod name { … }` declarations of `s`: the out-of-line
/// ones as `(name, #[path], is_test)`. Inline ones go into `s.inline`.
fn declared_modules(s: &mut Source) -> Vec<(String, Option<String>, bool)> {
    let t = &s.text;
    let mut out = Vec::new();
    let mut inline = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let word_start = i == 0 || !is_ident(t[i - 1]);
        if !(word_start
            && starts_with(t, i, "mod")
            && t.get(i + 3).is_some_and(|c| c.is_whitespace()))
        {
            i += 1;
            continue;
        }
        let k = skip_ws(t, i + 3);
        let end = (k..t.len()).find(|&e| !is_ident(t[e])).unwrap_or(t.len());
        let name: String = t[k..end].iter().collect();
        let after = skip_ws(t, end);
        match t.get(after) {
            Some(';') => {
                // `#[path = "…"]` in the lines above, read from the raw text
                // (the blanked text has no strings).
                let line_start = s.raw[..i].iter().rposition(|c| *c == '\n').unwrap_or(0);
                let above: String = s.raw[line_start.saturating_sub(200)..line_start]
                    .iter()
                    .collect();
                let path = above
                    .lines()
                    .rev()
                    .take_while(|l| l.trim_start().starts_with("#["))
                    .find_map(|l| {
                        let l = l.trim();
                        let rest = l.strip_prefix("#[path")?.trim_start().strip_prefix('=')?;
                        Some(
                            rest.trim()
                                .trim_end_matches(']')
                                .trim()
                                .trim_matches('"')
                                .to_string(),
                        )
                    });
                out.push((name, path, s.is_test(i)));
            }
            Some('{') if !name.is_empty() => inline.push((after, closing(t, after))),
            _ => {}
        }
        i = end;
    }
    s.inline = inline;
    out
}

/// Every lib source file, reached from `lib.rs` through its `mod`s.
fn lib_sources(src: &Path) -> Vec<Source> {
    let mut out = Vec::new();
    let mut todo = vec![(src.join("lib.rs"), Vec::<String>::new(), false)];
    while let Some((file, module, test_file)) = todo.pop() {
        let raw = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        let name = file
            .strip_prefix(src)
            .expect("below src")
            .to_string_lossy()
            .replace('\\', "/");
        let mut s = parse_source(name, module.clone(), test_file, &raw);
        let dir = file.parent().expect("a directory").to_path_buf();
        let stem = file.file_stem().and_then(|x| x.to_str()).unwrap_or("");
        for (child, path, child_test) in declared_modules(&mut s) {
            let candidates: Vec<PathBuf> = match path {
                Some(p) => vec![dir.join(p)],
                None if matches!(stem, "lib" | "mod") => {
                    vec![
                        dir.join(format!("{child}.rs")),
                        dir.join(&child).join("mod.rs"),
                    ]
                }
                None => vec![
                    dir.join(stem).join(format!("{child}.rs")),
                    dir.join(stem).join(&child).join("mod.rs"),
                ],
            };
            let found = candidates
                .into_iter()
                .find(|c| c.exists())
                .unwrap_or_else(|| panic!("no file for `mod {child};` in {}", s.name));
            let mut child_module = module.clone();
            child_module.push(child);
            todo.push((found, child_module, test_file || child_test));
        }
        out.push(s);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ------------------------------------------------------------------ paths

/// `a::{b, c::{self, d as e}, f::*}` -> `[a,b]`, `[a,c]`, `[a,c,d]`, `[a,f]`.
fn expand_use(tree: &str) -> Vec<Vec<String>> {
    // Drop each `as alias`, keeping what follows the alias (`b,` -> `,`).
    let mut flat = String::new();
    let mut alias = false;
    for word in tree.split_whitespace() {
        if word == "as" {
            alias = true;
        } else if alias {
            alias = false;
            flat.push_str(word.trim_start_matches(is_ident));
        } else {
            flat.push_str(word);
        }
    }
    fn rec(prefix: &[String], s: &str, out: &mut Vec<Vec<String>>) {
        let (mut parts, mut depth, mut cur) = (Vec::new(), 0, String::new());
        for ch in s.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if ch == ',' && depth == 0 {
                parts.push(std::mem::take(&mut cur));
            } else {
                cur.push(ch);
            }
        }
        parts.push(cur);
        for part in parts.iter().filter(|p| !p.is_empty()) {
            if let Some(open) = part.find('{') {
                let mut p = prefix.to_vec();
                p.extend(
                    part[..open]
                        .split("::")
                        .filter(|x| !x.is_empty())
                        .map(String::from),
                );
                let close = part.rfind('}').unwrap_or(part.len());
                rec(&p, &part[open + 1..close], out);
            } else {
                let mut p = prefix.to_vec();
                p.extend(part.split("::").filter(|x| !x.is_empty()).map(String::from));
                if matches!(p.last().map(String::as_str), Some("self" | "*")) {
                    p.pop();
                }
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    rec(&[], flat.trim_start_matches("::"), &mut out);
    out
}

/// Every `crate::`/`super::`/`self::` path in `s`, as written, with its
/// offset: the paths of `use` trees and the qualified paths in code.
fn written_paths(s: &Source) -> Vec<(usize, Vec<String>)> {
    let t = &s.text;
    let mut out = Vec::new();
    let mut in_use = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let word_start = i == 0 || !is_ident(t[i - 1]);
        if word_start && starts_with(t, i, "use") && t.get(i + 3).is_some_and(|c| c.is_whitespace())
        {
            let end = (i..t.len()).find(|&k| t[k] == ';').unwrap_or(t.len());
            let tree: String = t[i + 3..end].iter().collect();
            for path in expand_use(&tree) {
                out.push((i, path));
            }
            in_use.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }
    let mut i = 0;
    while i < t.len() {
        let free = (i == 0 || !(is_ident(t[i - 1]) || t[i - 1] == ':'))
            && !in_use.iter().any(|&(a, b)| a <= i && i < b);
        let head = ["crate::", "super::", "self::"]
            .into_iter()
            .find(|h| starts_with(t, i, h));
        if !(free && head.is_some()) {
            i += 1;
            continue;
        }
        let mut segs = Vec::new();
        let mut k = i;
        loop {
            let end = (k..t.len()).find(|&e| !is_ident(t[e])).unwrap_or(t.len());
            if end == k {
                break;
            }
            segs.push(t[k..end].iter().collect::<String>());
            if starts_with(t, end, "::") && t.get(end + 2).is_some_and(|c| is_ident(*c)) {
                k = end + 2;
            } else {
                k = end;
                break;
            }
        }
        out.push((i, segs));
        i = k.max(i + 1);
    }
    out
}

/// The absolute path (from the crate root) a written path names, or `None`
/// when it is not a path into this crate or stays inside an inline module
/// of the same file.
fn absolute(s: &Source, at: usize, segs: &[String]) -> Option<Vec<String>> {
    let first = segs.first()?;
    if first == "crate" {
        return Some(segs[1..].to_vec());
    }
    if first != "super" && first != "self" {
        return None;
    }
    let mut inline = s
        .inline
        .iter()
        .filter(|&&(a, b)| a <= at && at <= b)
        .count();
    let mut base = s.module.clone();
    let mut rest = segs;
    while let Some(head) = rest.first() {
        match head.as_str() {
            "super" if inline > 0 => inline -= 1,
            "super" => {
                base.pop()?;
            }
            "self" => {}
            _ => break,
        }
        rest = &rest[1..];
    }
    if inline > 0 {
        return None;
    }
    base.extend(rest.iter().cloned());
    Some(base)
}

/// One path that crosses a would-be crate boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    from: Crate,
    to: Crate,
    from_module: String,
    to_module: String,
    test: bool,
    site: String,
}

/// Every path in `sources` from one would-be crate into another.
fn crate_edges(sources: &[Source]) -> Vec<Edge> {
    let modules: BTreeSet<Vec<String>> = sources.iter().map(|s| s.module.clone()).collect();
    let mut out = Vec::new();
    for s in sources {
        let Some(from) = crate_of(&s.module) else {
            continue;
        };
        for (at, segs) in written_paths(s) {
            let Some(abs) = absolute(s, at, &segs) else {
                continue;
            };
            let target = (0..=abs.len())
                .rev()
                .map(|k| abs[..k].to_vec())
                .find(|p| modules.contains(p))
                .unwrap_or_default();
            if target == s.module {
                continue;
            }
            let Some(to) = crate_of(&target) else {
                continue;
            };
            if to == from {
                continue;
            }
            out.push(Edge {
                from,
                to,
                from_module: s.module.join("::"),
                to_module: target.join("::"),
                test: s.is_test(at),
                site: format!("src/{}:{}: {}", s.name, s.line(at), segs.join("::")),
            });
        }
    }
    out.sort();
    out
}

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

// ------------------------------------------------------------------ tests

/// No module uses a crate it may not: not in production code and not in
/// tests. A back-edge here would stop the split (PR1 onward) from being a
/// plain move of files. Fix it by moving the item down to the crate of its
/// lowest user (and leaving a `pub use` at the old path for the others), or
/// by moving the test up to the crate that has everything it needs.
#[test]
fn the_would_be_crates_have_no_back_edges() {
    let sources = lib_sources(&src_dir());
    let edges = crate_edges(&sources);
    let back: Vec<String> = edges
        .iter()
        .filter(|e| !e.from.may_use(e.to))
        .map(|e| {
            format!(
                "{} ({} {:?} -> {} {:?}{})",
                e.site,
                e.from_module,
                e.from,
                e.to_module,
                e.to,
                if e.test { ", test" } else { "" }
            )
        })
        .collect();
    assert!(
        back.is_empty(),
        "{} back-edge(s) between the would-be crates:\n{}",
        back.len(),
        back.join("\n")
    );

    // The scan sees the tree: the crate edges it must find, in production
    // and in tests, are there.
    let seen = |from: &str, to: &str, test: bool| {
        edges
            .iter()
            .any(|e| e.from_module == from && e.to_module == to && e.test == test)
    };
    for (from, to, test) in [
        ("reader_apply", "shadow", false),
        ("capture", "perf", false),
        ("boundary", "model", false),
        ("detect", "model", false),
        ("storage", "suspect", false),
        ("ocr_switch", "ocr_status", false),
        ("report::kit", "report::blank", false),
        ("report::kit", "report::blank::synth", true),
        ("reader_apply::tests_board_merge", "sync", true),
    ] {
        assert!(seen(from, to, test), "the scan missed {from} -> {to}");
    }
    let pairs: BTreeSet<(&str, &str)> = edges
        .iter()
        .filter(|e| !e.test)
        .map(|e| (e.from_module.as_str(), e.to_module.as_str()))
        .collect();
    assert!(
        pairs.len() > 50,
        "cross-crate module pairs: {}",
        pairs.len()
    );
}

/// Every module of the lib has a crate, and the walk from `lib.rs` reached
/// every lib file under `src/` (so no file escapes the scan).
#[test]
fn every_module_has_a_crate() {
    let src = src_dir();
    let sources = lib_sources(&src);
    let unassigned: Vec<String> = sources
        .iter()
        .filter(|s| crate_of(&s.module).is_none())
        .map(|s| s.name.clone())
        .collect();
    assert!(
        unassigned.is_empty(),
        "give these modules a crate in CRATE_OF: {unassigned:?}"
    );

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&src, &mut files);
    let on_disk: BTreeSet<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&src)
                .expect("below src")
                .to_string_lossy()
                .replace('\\', "/")
        })
        // the bin and its tests
        .filter(|n| n != "main.rs" && n != "ocr_bench.rs" && !n.starts_with("main_tests/"))
        .collect();
    let walked: BTreeSet<String> = sources.iter().map(|s| s.name.clone()).collect();
    assert_eq!(walked, on_disk, "files the walk from lib.rs did not reach");
    assert!(walked.len() > 80, "{}", walked.len());

    let per_crate: BTreeMap<Crate, usize> = sources.iter().fold(BTreeMap::new(), |mut acc, s| {
        *acc.entry(crate_of(&s.module).expect("assigned"))
            .or_default() += 1;
        acc
    });
    assert_eq!(per_crate.len(), 6, "{per_crate:?}");
}

/// The scanner on small sources: `use` trees, `super::` out of inline test
/// modules, `#[cfg(test)]` regions, comments and strings, and a back-edge.
#[test]
fn the_scan_follows_use_trees_super_paths_and_test_regions() {
    let src = parse_source(
        "report/kit.rs".into(),
        vec!["report".into(), "kit".into()],
        false,
        r#"
use crate::{config::{self, Config}, shadow::*};
use super::blank::{self as b, TeamLayout};
// use crate::sync::Nope;
fn f() -> &'static str { let _c = 'x'; "crate::sync::Nope" }
fn g() { crate::storage::open(); }
#[cfg(test)]
mod tests {
    use super::super::bundle::sha256_hex;
    use super::*;
    fn h() { self::h(); super::f(); }
}
"#,
    );
    let mut src = src;
    let declared = declared_modules(&mut src);
    assert!(declared.is_empty(), "{declared:?}");
    assert_eq!(src.inline.len(), 1);
    let found: Vec<(Vec<String>, bool)> = written_paths(&src)
        .into_iter()
        .filter_map(|(at, segs)| Some((absolute(&src, at, &segs)?, src.is_test(at))))
        .collect();
    let path = |p: &str| p.split("::").map(String::from).collect::<Vec<_>>();
    for (want, test) in [
        ("config", false),
        ("config::Config", false),
        ("shadow", false),
        ("report::blank", false),
        ("report::blank::TeamLayout", false),
        ("storage::open", false),
        ("report::bundle::sha256_hex", true),
        ("report::kit", true),
        ("report::kit::f", true),
    ] {
        assert!(
            found.contains(&(path(want), test)),
            "{want} (test {test}) not in {found:?}"
        );
    }
    assert!(
        !found.iter().any(|(p, _)| p.contains(&"sync".to_string())),
        "comments and strings are not paths: {found:?}"
    );

    // From core, `crate::reader_apply` and `super::` into `report` are
    // back-edges; from top they are not.
    let lower = parse_source(
        "parse.rs".into(),
        vec!["parse".into()],
        false,
        "use crate::reader_apply::upload_suspect_fields;\nfn f() { crate::model::is_clean_stat(\"1\"); }\n",
    );
    let upper = parse_source(
        "reader_apply.rs".into(),
        vec!["reader_apply".into()],
        false,
        "use crate::parse::canonical_hero;\n",
    );
    let modules = vec![
        lower,
        upper,
        parse_source("model.rs".into(), vec!["model".into()], false, ""),
    ];
    let edges = crate_edges(&modules);
    let back: Vec<&Edge> = edges.iter().filter(|e| !e.from.may_use(e.to)).collect();
    assert_eq!(back.len(), 1, "{edges:?}");
    assert_eq!(
        (back[0].from, back[0].to, back[0].to_module.as_str()),
        (Crate::Core, Crate::Top, "reader_apply")
    );
    assert!(
        back[0]
            .site
            .starts_with("src/parse.rs:1: crate::reader_apply")
    );
    assert!(Crate::Top.may_use(Crate::Vision));
    assert!(!Crate::Store.may_use(Crate::Vision));
    assert!(!Crate::Vision.may_use(Crate::Store));
    assert!(Crate::Game.may_use(Crate::Core));
}
