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
//!   capture_gate,            sync        packs, mapcrops, report::blank,
//!   input,                               ocr_switch, frame
//!   game_running
//!          \        |          |           /
//!     core: config, parse, perf, perf_log, setup, replace, proc_id, fs_mode,
//!           omp, host_probe, sandbox, hero_auth, model, version, suspect,
//!           ocr_status
//! ```
//!
//! The four middle crates do not use each other. The test reads the `src/`
//! of every crate the split has made so far ([`PHYSICAL`]: the lib only,
//! not `main.rs`, `daemon/`, `main_tests/` or `ocr_bench.rs`), follows every
//! `crate::`, `super::` and `self::` path (and every path through an
//! extracted crate's name, such as `stat_tracker_core::`) in `use` trees
//! and in code, and fails on any path from a module to a crate it may not
//! use, in production code and in tests alike. A path through a re-export
//! counts where it is written: `crate::detect::MatchOutcome` from `parse`
//! is a back-edge even though the type lives in `model`, because `parse`
//! will not see `detect` once the crates are split.
//!
//! An extracted crate's modules keep their names: `scuffed-stat-tracker`
//! re-exports each one at its old path ([`the_extracted_crates_are_reexported_at_their_old_paths`]),
//! so `crate::config` written in this crate and in `stat-tracker-core` name
//! the same module, and the walk joins both trees into one module map.
//! The compiler now checks every path out of an extracted crate; the scan
//! still checks the would-be crates that share this one.
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

/// Module path prefix -> crate; the longest wins (`report::blank` is vision, `report` top).
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
    ("game_running", Crate::Game),
    ("input", Crate::Game),
    ("capture", Crate::Capture),
    ("storage", Crate::Store),
    ("sync", Crate::Store),
    ("detect", Crate::Vision),
    ("frame", Crate::Vision),
    ("mapcrops", Crate::Vision),
    ("names", Crate::Vision),
    ("ocr", Crate::Vision),
    ("ocr_switch", Crate::Vision),
    ("packs", Crate::Vision),
    ("report::blank", Crate::Vision),
    ("shadow", Crate::Vision),
    ("games", Crate::Top),
    ("reader_apply", Crate::Top),
    ("report", Crate::Top),
    ("tab_retry", Crate::Top),
    ("tests_file_size", Crate::Top),
    ("tests_layering", Crate::Top),
    ("tests_readme_config", Crate::Top),
    ("tests_release_pins", Crate::Top),
];

/// The crates the split has made so far: directory under `crates/`, the
/// would-be crate it holds, and its lib name. `stat-tracker` still holds
/// every would-be crate not yet extracted. An extraction PR adds its crate
/// here, and the test then walks its `src/` too, from its `lib.rs`.
const PHYSICAL: &[(&str, Crate, &str)] = &[
    ("stat-tracker", Crate::Top, "stat_tracker"),
    ("stat-tracker-core", Crate::Core, "stat_tracker_core"),
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
    cfg_regions(t, |cfg| {
        cfg.split(|c: char| !is_ident(c)).any(|word| word == "test")
            && !cfg.replace(' ', "").contains("not(test)")
    })
}

/// `(start, end)` of every item behind a `#[cfg(...)]` whose predicate (the
/// text inside `cfg(`…`)`) `needs_test` accepts.
fn cfg_regions(t: &[char], needs_test: impl Fn(&str) -> bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if !starts_with(t, i, "#[cfg(") {
            i += 1;
            continue;
        }
        let close = closing(t, i + 1);
        // `t[close]` is the `]`; drop the `)` of `cfg(` before it too.
        let cfg: String = t[i + 6..close].iter().collect();
        let cfg = cfg.trim_end();
        let needs_test = needs_test(cfg.strip_suffix(')').unwrap_or(cfg));
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
    /// The crate directory under `crates/` ([`PHYSICAL`]).
    dir: &'static str,
    /// The would-be crate of that directory: the crate of its root.
    home: Crate,
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

    /// The would-be crate of this file's module. A crate root (`lib.rs`) is
    /// the crate of its directory.
    fn krate(&self) -> Option<Crate> {
        if self.module.is_empty() {
            Some(self.home)
        } else {
            crate_of(&self.module)
        }
    }

    fn site(&self, at: usize) -> String {
        format!("{}/src/{}:{}", self.dir, self.name, self.line(at))
    }
}

fn parse_source(
    dir: &'static str,
    home: Crate,
    name: String,
    module: Vec<String>,
    test_file: bool,
    raw: &str,
) -> Source {
    let text = blank(raw);
    let tests = test_regions(&text);
    Source {
        dir,
        home,
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

/// `crates/`, next to this crate.
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .to_path_buf()
}

/// Every lib source file of every crate in [`PHYSICAL`].
fn all_sources() -> Vec<Source> {
    PHYSICAL
        .iter()
        .flat_map(|&(dir, home, _)| crate_sources(dir, home))
        .collect()
}

/// Every lib source file of `crates/{dir}`, reached from its `lib.rs`
/// through its `mod`s.
fn crate_sources(dir: &'static str, home: Crate) -> Vec<Source> {
    let src = crates_dir().join(dir).join("src");
    let src = src.as_path();
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
        let mut s = parse_source(dir, home, name, module.clone(), test_file, &raw);
        let file_dir = file.parent().expect("a directory").to_path_buf();
        let stem = file.file_stem().and_then(|x| x.to_str()).unwrap_or("");
        for (child, path, child_test) in declared_modules(&mut s) {
            let candidates: Vec<PathBuf> = match path {
                Some(p) => vec![file_dir.join(p)],
                None if matches!(stem, "lib" | "mod") => {
                    vec![
                        file_dir.join(format!("{child}.rs")),
                        file_dir.join(&child).join("mod.rs"),
                    ]
                }
                None => vec![
                    file_dir.join(stem).join(format!("{child}.rs")),
                    file_dir.join(stem).join(&child).join("mod.rs"),
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

/// A path segment as the name it is: `r#TabKey` is `TabKey`.
fn raw_ident(segment: &str) -> String {
    segment.strip_prefix("r#").unwrap_or(segment).to_string()
}

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
                        .map(raw_ident),
                );
                let close = part.rfind('}').unwrap_or(part.len());
                rec(&p, &part[open + 1..close], out);
            } else {
                let mut p = prefix.to_vec();
                p.extend(part.split("::").filter(|x| !x.is_empty()).map(raw_ident));
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

/// Every `crate::`/`super::`/`self::` path in `s` (and every path through
/// the lib name of a crate in [`PHYSICAL`]), as written, with its offset:
/// the paths of `use` trees and the qualified paths in code.
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
    let heads: Vec<String> = ["crate", "super", "self"]
        .into_iter()
        .chain(PHYSICAL.iter().map(|&(_, _, lib)| lib))
        .map(|h| format!("{h}::"))
        .collect();
    let mut i = 0;
    while i < t.len() {
        let free = (i == 0 || !(is_ident(t[i - 1]) || t[i - 1] == ':'))
            && !in_use.iter().any(|&(a, b)| a <= i && i < b);
        let head = heads.iter().find(|h| starts_with(t, i, h));
        if !(free && head.is_some()) {
            i += 1;
            continue;
        }
        let mut segs = Vec::new();
        let mut k = i;
        loop {
            // A raw identifier (`r#TabKey`) names `TabKey`.
            if starts_with(t, k, "r#") && t.get(k + 2).is_some_and(|c| is_ident(*c)) {
                k += 2;
            }
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

/// The absolute path (from a crate root) a written path names, and the
/// would-be crate of that root; or `None` when it is not a path into a
/// crate of [`PHYSICAL`] or stays inside an inline module of the same file.
fn absolute(s: &Source, at: usize, segs: &[String]) -> Option<(Vec<String>, Crate)> {
    let first = segs.first()?;
    if first == "crate" {
        return Some((segs[1..].to_vec(), s.home));
    }
    if let Some(&(_, root, _)) = PHYSICAL.iter().find(|(_, _, lib)| first == lib) {
        return Some((segs[1..].to_vec(), root));
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
    Some((base, s.home))
}

/// One path that crosses a would-be crate boundary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    from: Crate,
    to: Crate,
    from_module: String,
    to_module: String,
    test: bool,
    /// The path is in a module-level, bare `pub use` with no glob: a
    /// re-export for users outside the crate ([`is_pub_use`]).
    reexport: bool,
    site: String,
    /// The path named, absolute below its crate root.
    path: Vec<String>,
}

/// Whether the `use` at `at` in `s` is a re-export [`COMPAT_REEXPORTS`]
/// can take: a bare `pub use` (not `pub(crate)`, `pub(super)`, `pub(in …)`
/// or `pub(self)`, which serve users inside the crate), at module level
/// (not inside a function or an inline module), with no `*` and no `as` in
/// its tree: a renamed re-export (`TabKey as Key2`, `game_running as gr`)
/// would add a name nothing pins.
fn is_pub_use(s: &Source, at: usize) -> bool {
    let t = &s.text;
    if !starts_with(t, at, "use") {
        return false;
    }
    let mut k = at;
    while k > 0 && t[k - 1].is_whitespace() {
        k -= 1;
    }
    let bare_pub = k >= 3 && starts_with(t, k - 3, "pub") && (k == 3 || !is_ident(t[k - 4]));
    let depth = t[..at].iter().fold(0i32, |d, c| match c {
        '{' => d + 1,
        '}' => d - 1,
        _ => d,
    });
    let end = (at..t.len()).find(|&e| t[e] == ';').unwrap_or(t.len());
    let renames = (at..end).any(|k| {
        starts_with(t, k, "as")
            && !is_ident(t[k - 1])
            && t.get(k + 2).is_some_and(|c| !is_ident(*c))
    });
    bare_pub && depth == 0 && !t[at..end].contains(&'*') && !renames
}

/// One re-export that keeps an old path working across a would-be crate
/// boundary until every user names the new path.
struct Compat {
    /// The module the old path is in.
    from: &'static str,
    /// The module the item moved to.
    to: &'static str,
    /// The names re-exported, each as `pub use crate::<to>::<name>;`. Empty:
    /// the module itself, `pub use crate::<to>;`.
    names: &'static [&'static str],
}

impl Compat {
    /// The names the re-export adds to `from`.
    fn aliases(&self) -> Vec<&'static str> {
        if self.names.is_empty() {
            vec![self.to]
        } else {
            self.names.to_vec()
        }
    }
}

/// The re-exports the bin still needs. Each entry allows exactly its
/// `pub use` lines ([`compat_reexport`]): any other path from `from` into
/// `to` is still a back-edge, and so is any pair not listed here. Nothing in
/// the lib may name the old path ([`compat_aliases_are_not_named_in_the_lib`]),
/// and an entry fails once main.rs and main_tests stop naming it
/// ([`compat_old_paths_are_still_used_by_the_bin`]).
const COMPAT_REEXPORTS: &[Compat] = &[
    // `detect::{MultiKeyboardStream, TabKey}`: the Tab key reader moved to
    // `input`. Remove when main.rs and main_tests name `input::` (the GUI and
    // tab_retry already do).
    Compat {
        from: "detect",
        to: "input",
        names: &["MultiKeyboardStream", "TabKey"],
    },
    // `detect::game_running`: the game-process check moved to the top-level
    // `game_running`. Remove when main.rs names `game_running::` (the GUI
    // already does).
    Compat {
        from: "detect",
        to: "game_running",
        names: &[],
    },
];

/// Whether `e` is one of the re-exports [`COMPAT_REEXPORTS`] allows.
fn compat_reexport(e: &Edge) -> bool {
    e.reexport
        && !e.test
        && COMPAT_REEXPORTS.iter().any(|c| {
            e.from_module == c.from
                && e.to_module == c.to
                && if c.names.is_empty() {
                    e.path == [c.to]
                } else {
                    e.path.len() == 2 && e.path[0] == c.to && c.names.contains(&e.path[1].as_str())
                }
        })
}

/// Every place in `sources` that reaches a [`COMPAT_REEXPORTS`] item
/// through its old path, as `file:line: what`:
/// * a path in any form (`crate::`, `super::`, `self::`, the lib name) to
///   `<from>::<alias>`;
/// * a `use` of the `from` module itself, or a glob of it, outside it
///   (`detect::TabKey` after `use crate::detect;`, or a bare name after
///   `use crate::detect::*;`);
/// * a bare alias name in `from`'s own production code, outside the
///   re-export lines.
fn compat_alias_uses(sources: &[Source]) -> Vec<String> {
    let mut bad = Vec::new();
    let froms: BTreeSet<&str> = COMPAT_REEXPORTS.iter().map(|c| c.from).collect();
    for s in sources {
        for (at, segs) in written_paths(s) {
            let Some((abs, _)) = absolute(s, at, &segs) else {
                continue;
            };
            for c in COMPAT_REEXPORTS {
                for alias in c.aliases() {
                    let mut old = path_of(c.from);
                    old.push(alias.to_string());
                    if abs.starts_with(&old) {
                        bad.push(format!("{}: {}", s.site(at), segs.join("::")));
                    }
                }
            }
            let module_use = starts_with(&s.text, at, "use")
                && froms
                    .iter()
                    .any(|f| abs == path_of(f) && s.module != path_of(f));
            if module_use {
                bad.push(format!("{}: use of {}", s.site(at), segs.join("::")));
            }
        }
        for from in &froms {
            if s.module != path_of(from) {
                continue;
            }
            let aliases: BTreeSet<&str> = COMPAT_REEXPORTS
                .iter()
                .filter(|c| c.from == *from)
                .flat_map(|c| c.aliases())
                .collect();
            let t = &s.text;
            let mut skip = cfg_regions(t, cfg_needs_test);
            for (at, segs) in written_paths(s) {
                let into_to = absolute(s, at, &segs).is_some_and(|(abs, _)| {
                    COMPAT_REEXPORTS
                        .iter()
                        .any(|c| c.from == *from && abs.first().is_some_and(|f| f == c.to))
                });
                if is_pub_use(s, at) && into_to {
                    let end = (at..t.len()).find(|&e| t[e] == ';').unwrap_or(t.len());
                    skip.push((at, end));
                }
            }
            let mut i = 0;
            while i < t.len() {
                if !is_ident(t[i]) || (i > 0 && is_ident(t[i - 1])) {
                    i += 1;
                    continue;
                }
                let end = (i..t.len()).find(|&e| !is_ident(t[e])).unwrap_or(t.len());
                let word: String = t[i..end].iter().collect();
                let skipped = skip.iter().any(|&(a, b)| a <= i && i <= b);
                if !skipped && aliases.contains(word.as_str()) {
                    bad.push(format!("{}: bare {word}", s.site(i)));
                }
                i = end;
            }
        }
    }
    bad
}

/// Every path in `sources` from one would-be crate into another.
fn crate_edges(sources: &[Source]) -> Vec<Edge> {
    let modules: BTreeSet<Vec<String>> = sources.iter().map(|s| s.module.clone()).collect();
    let mut out = Vec::new();
    for s in sources {
        let Some(from) = s.krate() else {
            continue;
        };
        for (at, segs) in written_paths(s) {
            let Some((abs, root)) = absolute(s, at, &segs) else {
                continue;
            };
            let mut target = (0..=abs.len())
                .rev()
                .map(|k| abs[..k].to_vec())
                .find(|p| modules.contains(p))
                .unwrap_or_default();
            if target.is_empty() && !abs.is_empty() && crate_of(&abs[..1]).is_some() {
                // A module of a crate this scan does not walk, re-exported
                // at the root: it counts as its own crate, not the root's
                // (PR #452 review, F2).
                target = abs[..1].to_vec();
            }
            if target == s.module && root == s.home {
                continue;
            }
            let Some(to) = (if target.is_empty() {
                Some(root)
            } else {
                crate_of(&target)
            }) else {
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
                reexport: is_pub_use(s, at),
                site: format!("{}: {}", s.site(at), segs.join("::")),
                path: abs.clone(),
            });
        }
    }
    out.sort();
    out
}

// ------------------------------------------------------------------ impls

/// One `impl` block: the trait it implements (last path segment), if any,
/// and its self type (last path segment).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Impl {
    at: usize,
    trait_name: Option<String>,
    self_type: String,
}

/// The last path segment of the type or trait that starts `text`
/// (`fmt::Display` -> `Display`, `&'a mut Foo<T>` -> `Foo`), or `None` for
/// a tuple, slice, array, pointer or fn-pointer type.
fn last_segment(text: &str) -> Option<String> {
    let mut t = text.trim_start();
    loop {
        let before = t;
        t = t.trim_start_matches(['&', '!', '?']).trim_start();
        if let Some(rest) = t.strip_prefix('\'') {
            t = rest.trim_start_matches(is_ident).trim_start();
        }
        for keyword in ["mut ", "dyn ", "const "] {
            if let Some(rest) = t.strip_prefix(keyword) {
                t = rest.trim_start();
            }
        }
        if t == before {
            break;
        }
    }
    let end = t
        .find(|c: char| !(is_ident(c) || c == ':'))
        .unwrap_or(t.len());
    let last = t[..end].rsplit("::").next()?;
    (!last.is_empty() && !last.starts_with(|c: char| c.is_ascii_digit()) && last != "fn")
        .then(|| last.to_string())
}

/// Every `impl` item in `s` (not `impl Trait` in argument or return
/// position): its offset, trait and self type.
fn impls(s: &Source) -> Vec<Impl> {
    let t = &s.text;
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let word = (i == 0 || !is_ident(t[i - 1]))
            && starts_with(t, i, "impl")
            && t.get(i + 4).is_some_and(|c| c.is_whitespace() || *c == '<');
        if !word {
            i += 1;
            continue;
        }
        // An item starts after the end of an item or attribute, at a block
        // start, or after `unsafe`. `-> impl`, `: impl`, `(impl`, `<impl`
        // and `, impl` are types.
        let prev = (0..i).rev().find(|&k| !t[k].is_whitespace());
        let item = match prev {
            None => true,
            Some(k) if matches!(t[k], '}' | ';' | '{' | ']') => true,
            Some(k) => {
                k >= 5
                    && t[k - 5..=k].iter().collect::<String>() == "unsafe"
                    && (k == 5 || !is_ident(t[k - 6]))
            }
        };
        if !item {
            i += 4;
            continue;
        }
        // The header: optional generics, then up to the body's `{` (or a
        // `where`) outside brackets. `->` inside `Fn(..) -> T` is no `>`.
        let mut k = skip_ws(t, i + 4);
        let mut depth = 0i32;
        let mut header = String::new();
        let mut generics = t.get(k) == Some(&'<');
        while k < t.len() {
            let c = t[k];
            match c {
                '<' | '(' | '[' => depth += 1,
                '>' if k > 0 && t[k - 1] == '-' => {}
                '>' | ')' | ']' => depth -= 1,
                _ => {}
            }
            if depth == 0 && generics {
                // the closing `>` of `impl<…>`
                generics = false;
                k += 1;
                continue;
            }
            if !generics {
                if depth == 0 && (c == '{' || c == ';') {
                    break;
                }
                if depth == 0
                    && starts_with(t, k, "where")
                    && !is_ident(t[k - 1])
                    && t.get(k + 5).is_some_and(|c| !is_ident(*c))
                {
                    break;
                }
                header.push(c);
            }
            k += 1;
        }
        // `Trait for Type`: the first `for` word outside brackets that is
        // not `for<'a>` (a higher-ranked bound).
        let chars: Vec<char> = header.chars().collect();
        let mut depth = 0i32;
        let mut split = None;
        for (j, c) in chars.iter().enumerate() {
            match c {
                '<' | '(' | '[' => depth += 1,
                '>' if j > 0 && chars[j - 1] == '-' => {}
                '>' | ')' | ']' => depth -= 1,
                _ => {}
            }
            if depth == 0
                && j > 0
                && chars[j - 1].is_whitespace()
                && starts_with(&chars, j, "for")
                && chars.get(j + 3).is_some_and(|c| c.is_whitespace())
            {
                split = Some(j);
                break;
            }
        }
        let (trait_text, type_text) = match split {
            Some(j) => (
                Some(chars[..j].iter().collect::<String>()),
                chars[j + 3..].iter().collect::<String>(),
            ),
            None => (None, header.clone()),
        };
        if let Some(self_type) = last_segment(&type_text) {
            out.push(Impl {
                at: i,
                trait_name: trait_text.as_deref().and_then(last_segment),
                self_type,
            });
        }
        i = k.max(i + 4);
    }
    out
}

/// The would-be crates that define a type (`struct`, `enum`, `union`) or a
/// trait of each name. `type` aliases and associated types are left out.
fn defined_names(
    sources: &[Source],
) -> (
    BTreeMap<String, BTreeSet<Crate>>,
    BTreeMap<String, BTreeSet<Crate>>,
) {
    let mut types: BTreeMap<String, BTreeSet<Crate>> = BTreeMap::new();
    let mut traits: BTreeMap<String, BTreeSet<Crate>> = BTreeMap::new();
    for s in sources {
        let Some(krate) = s.krate() else {
            continue;
        };
        let t = &s.text;
        for keyword in ["struct", "enum", "union", "trait"] {
            let into = if keyword == "trait" {
                &mut traits
            } else {
                &mut types
            };
            let mut i = 0;
            while i < t.len() {
                if (i == 0 || !is_ident(t[i - 1]))
                    && starts_with(t, i, keyword)
                    && t.get(i + keyword.len()).is_some_and(|c| c.is_whitespace())
                {
                    let k = skip_ws(t, i + keyword.len());
                    let end = (k..t.len()).find(|&e| !is_ident(t[e])).unwrap_or(t.len());
                    if end > k {
                        let name: String = t[k..end].iter().collect();
                        into.entry(name).or_default().insert(krate);
                    }
                    i = end.max(i + 1);
                } else {
                    i += 1;
                }
            }
        }
    }
    (types, traits)
}

// ------------------------------------------------------------------ tests

/// No module uses a crate it may not: not in production code and not in
/// tests. A back-edge here would stop the split (PR1 onward) from being a
/// plain move of files. Fix it by moving the item down to the crate of its
/// lowest user (and leaving a `pub use` at the old path for the others), or
/// by moving the test up to the crate that has everything it needs.
#[test]
fn the_would_be_crates_have_no_back_edges() {
    let sources = all_sources();
    let edges = crate_edges(&sources);
    let back: Vec<String> = edges
        .iter()
        .filter(|e| !e.from.may_use(e.to) && !compat_reexport(e))
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
        ("boundary::machine", "model", false),
        ("detect", "model", false),
        ("storage::edits", "suspect", false),
        ("ocr_switch", "ocr_status", false),
        ("report::kit", "report::blank", false),
        ("report::kit", "report::blank::tests_synth", true),
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

/// Every module of the lib has a crate, every module sits in the crate
/// directory of its would-be crate (or in `stat-tracker` while that crate
/// is not extracted yet), and the walk from each `lib.rs` reached every lib
/// file under that crate's `src/` (so no file escapes the scan).
#[test]
fn every_module_has_a_crate() {
    let sources = all_sources();
    let unassigned: Vec<String> = sources
        .iter()
        .filter(|s| s.krate().is_none())
        .map(|s| format!("{}/src/{}", s.dir, s.name))
        .collect();
    assert!(
        unassigned.is_empty(),
        "give these modules a crate in CRATE_OF: {unassigned:?}"
    );

    let extracted: BTreeSet<Crate> = PHYSICAL
        .iter()
        .map(|&(_, krate, _)| krate)
        .filter(|krate| *krate != Crate::Top)
        .collect();
    let misplaced: Vec<String> = sources
        .iter()
        .filter(|s| {
            let krate = s.krate().expect("assigned");
            if s.home == Crate::Top {
                extracted.contains(&krate)
            } else {
                krate != s.home
            }
        })
        .map(|s| format!("{}/src/{} ({:?})", s.dir, s.name, s.krate()))
        .collect();
    assert!(
        misplaced.is_empty(),
        "modules outside the crate directory of their CRATE_OF crate: {misplaced:?}"
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
    let mut on_disk = BTreeSet::new();
    for &(dir, _, _) in PHYSICAL {
        let src = crates_dir().join(dir).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        on_disk.extend(
            files
                .iter()
                .map(|p| {
                    p.strip_prefix(&src)
                        .expect("below src")
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                // the bin and its tests
                .filter(|n| {
                    dir != "stat-tracker"
                        || (n != "main.rs"
                            && n != "ocr_bench.rs"
                            && !n.starts_with("daemon/")
                            && !n.starts_with("main_tests/"))
                })
                .map(|n| format!("{dir}/src/{n}")),
        );
    }
    let walked: BTreeSet<String> = sources
        .iter()
        .map(|s| format!("{}/src/{}", s.dir, s.name))
        .collect();
    assert_eq!(walked, on_disk, "files the walks from lib.rs did not reach");
    assert!(walked.len() > 80, "{}", walked.len());

    let per_crate: BTreeMap<Crate, usize> = sources.iter().fold(BTreeMap::new(), |mut acc, s| {
        *acc.entry(s.krate().expect("assigned")).or_default() += 1;
        acc
    });
    assert_eq!(per_crate.len(), 6, "{per_crate:?}");
    for &(dir, krate, _) in PHYSICAL {
        let walked_here = sources.iter().filter(|s| s.dir == dir).count();
        assert!(
            walked_here > 10,
            "{dir} ({krate:?}): {walked_here} files walked"
        );
    }
}

/// `scuffed-stat-tracker` re-exports every top-level module of each
/// extracted crate at the root, under its own name, so the old paths
/// (`stat_tracker::config`, `crate::config`) keep working, and nothing
/// else comes in that way. A module added to an extracted crate fails here
/// until it is re-exported (or, if it should stay private to that crate,
/// until it is not `pub mod`).
#[test]
fn the_extracted_crates_are_reexported_at_their_old_paths() {
    let sources = all_sources();
    let top_lib = sources
        .iter()
        .find(|s| s.dir == "stat-tracker" && s.name == "lib.rs")
        .expect("stat-tracker/src/lib.rs");
    let reexported: Vec<Vec<String>> = written_paths(top_lib)
        .into_iter()
        .map(|(_, segs)| segs)
        .collect();
    let mut checked = 0;
    for &(dir, krate, lib) in PHYSICAL {
        if krate == Crate::Top {
            continue;
        }
        let declared: BTreeSet<String> = sources
            .iter()
            .filter(|s| s.dir == dir && s.module.len() == 1 && !s.test_file)
            .map(|s| s.module[0].clone())
            .collect();
        let through: BTreeSet<String> = reexported
            .iter()
            .filter(|segs| segs.len() == 2 && segs[0] == lib)
            .map(|segs| segs[1].clone())
            .collect();
        assert_eq!(
            through, declared,
            "{lib}: pub use in stat-tracker/src/lib.rs"
        );
        checked += declared.len();
    }
    assert!(checked >= 16, "{checked}");
}

/// No `impl` of a type of one would-be crate is written in another one:
/// no inherent impl, and no trait impl unless the trait belongs to the
/// impl's own crate. Once the two crates are split, the first is E0116
/// and the second E0117 (orphan rule), which is what forced
/// `PackReadiness::on_disk` out of `ocr_status` in PR #452.
///
/// For the types of an extracted crate (`stat-tracker-core` today) rustc
/// already enforces this, and this test agrees with it. It earns its place
/// for the would-be crates still inside `stat-tracker` (game, capture,
/// store, vision and top), where it reports now what would otherwise only
/// fail at the PR that extracts them. Types are matched by name; a name
/// defined in more than one would-be crate is skipped.
#[test]
fn no_impl_for_a_type_of_another_crate() {
    let sources = all_sources();
    let (types, traits) = defined_names(&sources);
    let mut bad = Vec::new();
    let mut allowed = Vec::new();
    let mut total = 0;
    let mut core_types_seen = 0;
    for s in &sources {
        let Some(from) = s.krate() else {
            continue;
        };
        for imp in impls(s) {
            total += 1;
            let Some(owners) = types.get(&imp.self_type) else {
                continue;
            };
            if owners.len() != 1 {
                continue;
            }
            let owner = *owners.iter().next().expect("one owner");
            if owner == Crate::Core {
                core_types_seen += 1;
            }
            if owner == from {
                continue;
            }
            let line = format!(
                "{}: impl {}{} ({from:?} impl of a {owner:?} type)",
                s.site(imp.at),
                imp.trait_name
                    .as_deref()
                    .map(|t| format!("{t} for "))
                    .unwrap_or_default(),
                imp.self_type
            );
            let own_trait = imp
                .trait_name
                .as_ref()
                .and_then(|t| traits.get(t))
                .is_some_and(|crates| crates.len() == 1 && crates.contains(&from));
            if own_trait {
                allowed.push(line);
            } else {
                bad.push(line);
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} impl(s) of another would-be crate's type:\n{}",
        bad.len(),
        bad.join("\n")
    );
    // The scan sees the tree.
    // 310 at PR1: every line-start `impl` under both src trees.
    assert!(total > 250, "impl blocks found: {total}");
    assert!(
        core_types_seen > 20,
        "impls of core types found: {core_types_seen}"
    );
    assert!(
        allowed
            .iter()
            .any(|l| l.contains("impl TabKeySource for MultiKeyboardStream")),
        "{allowed:?}"
    );
}

/// The impl scan on a small source: generics with `Fn(..) -> T`, paths,
/// references, `unsafe impl`, `where`, and `impl Trait` in types.
#[test]
fn the_impl_scan_reads_item_headers_only() {
    let src = parse_source(
        "stat-tracker",
        Crate::Top,
        "x.rs".into(),
        vec!["x".into()],
        false,
        r#"
impl<F: Fn() -> u8> fmt::Display for Wrap<F> {}
impl Plain { fn f(&self) -> impl Iterator<Item = u8> { std::iter::empty() } }
fn g(_: impl Sized) {}
unsafe impl Send for crate::model::Raw {}
impl<'a> From<&'a str> for &'a mut Thing where Thing: Sized {}
impl<T> Bound for T where T: for<'b> Fn(&'b u8) {}
"#,
    );
    let found: Vec<(Option<String>, String)> = impls(&src)
        .into_iter()
        .map(|imp| (imp.trait_name, imp.self_type))
        .collect();
    let own = |t: Option<&str>, ty: &str| (t.map(String::from), ty.to_string());
    assert_eq!(
        found,
        vec![
            own(Some("Display"), "Wrap"),
            own(None, "Plain"),
            own(Some("Send"), "Raw"),
            own(Some("From"), "Thing"),
            own(Some("Bound"), "T"),
        ]
    );
}

/// The scanner on small sources: `use` trees, `super::` out of inline test
/// modules, `#[cfg(test)]` regions, comments and strings, and a back-edge.
#[test]
fn the_scan_follows_use_trees_super_paths_and_test_regions() {
    let src = parse_source(
        "stat-tracker",
        Crate::Top,
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
        .filter_map(|(at, segs)| Some((absolute(&src, at, &segs)?.0, src.is_test(at))))
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
        "stat-tracker-core",
        Crate::Core,
        "parse.rs".into(),
        vec!["parse".into()],
        false,
        "use crate::reader_apply::upload_suspect_fields;\nfn f() { crate::model::is_clean_stat(\"1\"); }\n",
    );
    // From top, a path through core's lib name resolves into core, and a
    // root name that is no walked module (`sync` here) counts as its crate.
    let upper = parse_source(
        "stat-tracker",
        Crate::Top,
        "reader_apply.rs".into(),
        vec!["reader_apply".into()],
        false,
        "use crate::parse::canonical_hero;\nfn g() { stat_tracker_core::model::f(); crate::sync::up(); }\n",
    );
    let modules = vec![
        lower,
        upper,
        parse_source(
            "stat-tracker-core",
            Crate::Core,
            "model.rs".into(),
            vec!["model".into()],
            false,
            "",
        ),
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
            .starts_with("stat-tracker-core/src/parse.rs:1: crate::reader_apply"),
        "{}",
        back[0].site
    );
    let from_top: BTreeSet<(Crate, &str)> = edges
        .iter()
        .filter(|e| e.from_module == "reader_apply")
        .map(|e| (e.to, e.to_module.as_str()))
        .collect();
    assert_eq!(
        from_top,
        BTreeSet::from([
            (Crate::Core, "model"),
            (Crate::Core, "parse"),
            (Crate::Store, "sync")
        ]),
        "{edges:?}"
    );
    assert!(Crate::Top.may_use(Crate::Vision));
    assert!(!Crate::Store.may_use(Crate::Vision));
    assert!(!Crate::Vision.may_use(Crate::Store));
    assert!(Crate::Game.may_use(Crate::Core));
}

// ------------------------------------------------------------ frame

/// Whether a `#[cfg(...)]` predicate holds only in test builds: `test`, or
/// `all(…)` with such a predicate among its arguments. `any(test, …)`,
/// `not(…)` and anything else can hold in a release build, so they guard
/// production code.
fn cfg_needs_test(cfg: &str) -> bool {
    let cfg: String = cfg.chars().filter(|c| !c.is_whitespace()).collect();
    if cfg == "test" {
        return true;
    }
    let Some(inner) = cfg.strip_prefix("all(").and_then(|r| r.strip_suffix(')')) else {
        return false;
    };
    let (mut depth, mut arg, mut args) = (0i32, String::new(), Vec::new());
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            args.push(std::mem::take(&mut arg));
        } else {
            arg.push(ch);
        }
    }
    args.push(arg);
    args.iter().any(|a| cfg_needs_test(a))
}

/// Whether `s` is test code in every build: reached through a `mod`
/// declaration that [`cfg_needs_test`] accepts, at some level. A file
/// declared under `#[cfg(any(test, …))]` is production.
fn strictly_test_file(sources: &[Source], s: &Source) -> bool {
    if !s.test_file {
        return false;
    }
    let Some((name, parent)) = s.module.split_last() else {
        return false;
    };
    let Some(p) = sources.iter().find(|p| p.module == parent) else {
        return false;
    };
    let strict = cfg_regions(&p.text, cfg_needs_test);
    let t = &p.text;
    let declared_strictly = (0..t.len()).any(|i| {
        (i == 0 || !is_ident(t[i - 1]))
            && starts_with(t, i, "mod")
            && t.get(i + 3).is_some_and(|c| c.is_whitespace())
            && {
                let k = skip_ws(t, i + 3);
                let end = (k..t.len()).find(|&e| !is_ident(t[e])).unwrap_or(t.len());
                t[k..end].iter().collect::<String>() == *name
                    && t.get(skip_ws(t, end)) == Some(&';')
            }
            && strict.iter().any(|&(a, b)| a <= i && i <= b)
    });
    declared_strictly || strictly_test_file(sources, p)
}

/// The leaf modules of vision: each one's production code names only its
/// own subtree and the core modules (and outside crates), never another
/// module of this crate. `frame` is the shared pixel layer (geometry,
/// layout, stability), `packs::layout` the on-disk pack layout the readers
/// share (so the `names` <-> `packs` loop cannot come back), and
/// `shadow::ncc` the correlation kernel of the words-v1 readers. All the
/// modules they must not reach are vision like them, so
/// [`the_would_be_crates_have_no_back_edges`] cannot see such a path.
/// Re-exports at the old paths point the other way.
const LEAVES: &[&str] = &["frame", "packs::layout", "shadow::ncc"];

/// Whether a production path from the leaf `leaf` may name `abs` (an
/// absolute path below the crate root `root`): the leaf itself or a core
/// module (through `stat_tracker_core::` or its re-export at this crate's
/// root).
fn leaf_may_name(leaf: &[String], abs: &[String], root: Crate) -> bool {
    if root == Crate::Core {
        return true;
    }
    abs.starts_with(leaf)
        || abs.first().is_some_and(|first| {
            CRATE_OF
                .iter()
                .any(|(m, krate)| *krate == Crate::Core && m == first)
        })
}

/// Each production path in `s`, a file of the leaf `leaf`, that
/// [`leaf_may_name`] refuses, as `(site, path as written)`. A path that
/// names a module above the leaf is refused too, the crate root included
/// (`use crate as root;`, `use crate::*;`, or `use super::*;` from the
/// leaf's top file): a glob or an alias of it would let a bare path such as
/// `detect::` through. Comments, doc comments and strings do not
/// count; an item behind `#[cfg(test)]` or `#[cfg(all(test, …))]` does not
/// either, and one behind `#[cfg(any(test, …))]` does. `test_file` is
/// [`strictly_test_file`] for `s`.
fn leaf_paths_outside(s: &Source, leaf: &[String], test_file: bool) -> Vec<(String, String)> {
    if test_file {
        return Vec::new();
    }
    let tests = cfg_regions(&s.text, cfg_needs_test);
    written_paths(s)
        .into_iter()
        .filter(|&(at, _)| !tests.iter().any(|&(a, b)| a <= at && at <= b))
        .filter_map(|(at, segs)| {
            let (abs, root) = absolute(s, at, &segs)?;
            (!leaf_may_name(leaf, &abs, root)).then(|| (s.site(at), segs.join("::")))
        })
        .collect()
}

fn path_of(module: &str) -> Vec<String> {
    module.split("::").map(String::from).collect()
}

/// Every [`LEAVES`] module names only itself and core.
#[test]
fn leaves_name_only_themselves_and_core() {
    let sources = all_sources();
    let mut bad = Vec::new();
    for leaf in LEAVES {
        let leaf = path_of(leaf);
        let files: Vec<&Source> = sources
            .iter()
            .filter(|s| s.module.starts_with(&leaf))
            .collect();
        let want = if leaf == ["frame"] { 4 } else { 1 };
        assert!(
            files.len() >= want,
            "the walk found {} file(s) of {}",
            files.len(),
            leaf.join("::")
        );
        for s in files {
            for (site, path) in leaf_paths_outside(s, &leaf, strictly_test_file(&sources, s)) {
                bad.push(format!("{site}: {path} (leaf {})", leaf.join("::")));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} path(s) from a leaf to something other than itself or core:\n{}",
        bad.len(),
        bad.join("\n")
    );
    // The scan reads the leaves' production paths: frame's layout uses
    // geometry.
    let layout = sources
        .iter()
        .find(|s| s.name == "frame/layout.rs")
        .expect("frame/layout.rs");
    let to_geometry = |(at, segs): &(usize, Vec<String>)| {
        !layout.is_test(*at)
            && absolute(layout, *at, segs)
                .is_some_and(|(abs, _)| abs.starts_with(&path_of("frame::geometry")))
    };
    assert!(
        written_paths(layout).iter().any(to_geometry),
        "the scan saw no frame::geometry path in frame/layout.rs"
    );
}

/// The leaf check on small sources.
#[test]
fn the_leaf_check_allows_only_the_leaf_and_core() {
    let frame = path_of("frame");
    let mut src = parse_source(
        "stat-tracker",
        Crate::Top,
        "frame/layout.rs".into(),
        vec!["frame".into(), "layout".into()],
        false,
        r#"
/// See [`crate::detect::own_row`].
use crate::ocr::preprocess::prepare;
use crate::frame::geometry::crop_player_row;
use super::geometry::game_rect_16_9;
use crate::model::RowOcrResult;
use stat_tracker_core::config::Config;
use std::path::Path;
use crate::packs::PACKS;
use crate as root;
use crate::*;
use super::super::*;
fn f() -> &'static str { "crate::names::read_names" }
fn g() { super::super::shadow::read_board(); }
#[cfg(any(test, feature = "x"))]
fn h() { crate::ocr_switch::live_recognizer_id(); }
#[cfg(not(test))]
fn k() { crate::mapcrops::save(); }
#[cfg(all(test, unix))]
mod t2 {
    use crate::detect::hero_portrait::PortraitMatcher;
}
#[cfg(test)]
mod tests {
    use crate::detect::hero_portrait::PortraitMatcher;
}
"#,
    );
    declared_modules(&mut src);
    let bad: Vec<String> = leaf_paths_outside(&src, &frame, false)
        .into_iter()
        .map(|(_, path)| path)
        .collect();
    assert_eq!(
        bad,
        [
            "crate::ocr::preprocess::prepare",
            "crate::packs::PACKS",
            "crate",
            "crate",
            "super::super",
            "super::super::shadow::read_board",
            "crate::ocr_switch::live_recognizer_id",
            "crate::mapcrops::save",
        ]
    );
    assert!(leaf_paths_outside(&src, &frame, true).is_empty());

    for (cfg, needs) in [
        ("test", true),
        ("all(test, unix)", true),
        ("all(unix, all(test, debug_assertions))", true),
        ("any(test, feature = \"x\")", false),
        ("all(unix, any(test, debug_assertions))", false),
        ("not(test)", false),
        ("unix", false),
    ] {
        assert_eq!(cfg_needs_test(cfg), needs, "{cfg}");
    }

    // A file declared under `cfg(any(test, …))` is production; one under
    // `cfg(test)` is test code.
    let parent = parse_source(
        "stat-tracker",
        Crate::Top,
        "frame/mod.rs".into(),
        vec!["frame".into()],
        false,
        "pub mod geometry;\n#[cfg(any(test, feature = \"x\"))]\nmod tests_a;\n#[cfg(test)]\nmod tests_b;\n",
    );
    let child = |name: &str| {
        parse_source(
            "stat-tracker",
            Crate::Top,
            format!("frame/{name}.rs"),
            vec!["frame".into(), name.into()],
            true,
            "use crate::detect::GamePhase;\n",
        )
    };
    let sources = vec![parent, child("tests_a"), child("tests_b")];
    assert!(!strictly_test_file(&sources, &sources[1]));
    assert!(strictly_test_file(&sources, &sources[2]));
    assert_eq!(leaf_paths_outside(&sources[1], &frame, false).len(), 1);

    // packs::layout may name itself and core, not the downloader above it
    // (`super::`) and not a reader.
    let layout = path_of("packs::layout");
    let src = parse_source(
        "stat-tracker",
        Crate::Top,
        "packs/layout.rs".into(),
        layout.clone(),
        false,
        r#"
use super::PACK_UNSAFE;
use super::*;
use crate::packs::layout::PACKS;
use self::inner::X;
use crate::config::Config;
fn f() -> std::path::PathBuf { crate::names::pack::dir_in(std::path::Path::new("")) }
"#,
    );
    let bad: Vec<String> = leaf_paths_outside(&src, &layout, false)
        .into_iter()
        .map(|(_, path)| path)
        .collect();
    assert_eq!(
        bad,
        ["super::PACK_UNSAFE", "super", "crate::names::pack::dir_in"]
    );

    // shadow::ncc may not name the readers it serves.
    let ncc = path_of("shadow::ncc");
    let src = parse_source(
        "stat-tracker",
        Crate::Top,
        "shadow/ncc.rs".into(),
        ncc.clone(),
        false,
        "use super::result::FLOOR;\nuse super::*;\nuse crate::shadow::ncc::Gray;\nuse crate::frame::geometry::game_rect_16_9;\n",
    );
    let bad: Vec<String> = leaf_paths_outside(&src, &ncc, false)
        .into_iter()
        .map(|(_, path)| path)
        .collect();
    assert_eq!(
        bad,
        [
            "super::result::FLOOR",
            "super",
            "crate::frame::geometry::game_rect_16_9"
        ]
    );
}

/// Every [`COMPAT_REEXPORTS`] re-export is still there: a stale entry (the
/// old path was dropped) fails here, so the list only shrinks.
#[test]
fn compat_reexports_are_still_there() {
    let edges = crate_edges(&all_sources());
    for c in COMPAT_REEXPORTS {
        let wanted: Vec<Vec<String>> = if c.names.is_empty() {
            vec![path_of(c.to)]
        } else {
            c.names
                .iter()
                .map(|n| vec![c.to.to_string(), n.to_string()])
                .collect()
        };
        for path in wanted {
            assert!(
                edges.iter().any(|e| e.from_module == c.from
                    && e.path == path
                    && compat_reexport(e)
                    && !e.from.may_use(e.to)),
                "COMPAT_REEXPORTS: no `pub use crate::{}` left in {}: remove it",
                path.join("::"),
                c.from
            );
        }
    }
}

/// Nothing in the lib reaches a moved item through its old path: in-lib
/// users name the new path, so the re-export serves the bin only.
#[test]
fn compat_aliases_are_not_named_in_the_lib() {
    let bad = compat_alias_uses(&all_sources());
    assert!(
        bad.is_empty(),
        "{} use(s) of a compat re-export inside the lib (name the new path):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Each [`COMPAT_REEXPORTS`] alias is still named by main.rs, `daemon/` or main_tests
/// (comments and strings do not count). When none is left, the re-export
/// has no user and goes.
#[test]
fn compat_old_paths_are_still_used_by_the_bin() {
    let src = crates_dir().join("stat-tracker").join("src");
    let mut files = vec![src.join("main.rs")];
    let mut todo = vec![src.join("daemon"), src.join("main_tests")];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).expect("read daemon or main_tests") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                todo.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    let text: String = files
        .iter()
        .map(|f| {
            blank(&std::fs::read_to_string(f).expect("read"))
                .into_iter()
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    for c in COMPAT_REEXPORTS {
        for alias in c.aliases() {
            let needle = format!("{}::{alias}", c.from);
            let used = text.match_indices(&needle).any(|(at, _)| {
                let before = text[..at].chars().next_back();
                let after = text[at + needle.len()..].chars().next();
                !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
            });
            assert!(
                used,
                "{needle}: nothing uses the old path; delete the alias and this entry \
                 (or the bin names it in a form this check does not read, such as a \
                 grouped `as` import)"
            );
        }
    }
}

/// The re-export rule and the alias check on small sources.
#[test]
fn the_compat_exception_takes_only_listed_pub_uses() {
    let source = |name: &str, module: &[&str], text: &str| {
        let mut s = parse_source(
            "stat-tracker",
            Crate::Top,
            name.into(),
            module.iter().map(|m| m.to_string()).collect(),
            false,
            text,
        );
        declared_modules(&mut s);
        s
    };
    let detect = source(
        "detect/mod.rs",
        &["detect"],
        "pub use crate::input::{TabKey, tab_key_event};
pub use crate::game_running;
pub(crate) use crate::input::MultiKeyboardStream;
use crate::input::MAX_KEY_EVENT_AGE;
fn f() { crate::input::key_event_instant(); }
pub use crate::capture_gate::Gate;
pub use crate::input::*;
pub use crate::input;
mod inner {
    pub use crate::input::TabKey;
}
#[cfg(test)]
pub use crate::input::TabKey as Probe;
pub use crate::input::TabKey as Key2;
pub use crate::game_running as gr;
pub use crate::input::r#MultiKeyboardStream;
",
    );
    let sources = vec![
        detect,
        source("input.rs", &["input"], ""),
        source("game_running.rs", &["game_running"], ""),
        source("capture_gate.rs", &["capture_gate"], ""),
    ];
    let mut back: Vec<String> = crate_edges(&sources)
        .iter()
        .filter(|e| !e.from.may_use(e.to) && !compat_reexport(e))
        .map(|e| e.site.rsplit(": ").next().unwrap_or("").to_string())
        .collect();
    back.sort();
    assert_eq!(
        back,
        [
            "crate::capture_gate::Gate",
            "crate::game_running",
            "crate::input",
            "crate::input",
            "crate::input::MAX_KEY_EVENT_AGE",
            "crate::input::MultiKeyboardStream",
            "crate::input::TabKey",
            "crate::input::TabKey",
            "crate::input::TabKey",
            "crate::input::key_event_instant",
            "crate::input::tab_key_event",
        ]
    );
    let src = &sources[0];
    let at = |needle: &str| {
        src.raw
            .iter()
            .collect::<String>()
            .find(needle)
            .expect(needle)
    };
    assert!(is_pub_use(src, at("use crate::input::{")));
    assert!(is_pub_use(src, at("use crate::game_running")));
    assert!(!is_pub_use(src, at("use crate::input::Multi")));
    assert!(!is_pub_use(src, at("use crate::input::MAX")));
    assert!(!is_pub_use(src, at("use crate::input::*")));
    assert!(!is_pub_use(src, at("use crate::input::TabKey;")));
    assert!(!is_pub_use(src, at("use crate::input::TabKey as Key2")));
    assert!(!is_pub_use(src, at("use crate::game_running as gr")));
    // A raw identifier is its name: `r#MultiKeyboardStream` is the pinned
    // name, so this one passes.
    assert!(is_pub_use(src, at("use crate::input::r#Multi")));
    assert_eq!(
        expand_use(" crate::input::{r#TabKey, r#game_running::r#X}"),
        [
            path_of("crate::input::TabKey"),
            path_of("crate::input::game_running::X")
        ]
    );

    // The alias check: old paths in any form, a use of `detect` itself,
    // and a bare alias in detect's own code.
    let clean = source(
        "detect/mod.rs",
        &["detect"],
        "pub use crate::game_running;\npub use crate::input::{MultiKeyboardStream, TabKey};\n#[cfg(test)]\nfn t(_: TabKey) {}\n",
    );
    assert!(compat_alias_uses(&[clean]).is_empty());
    let sources = vec![
        source(
            "detect/mod.rs",
            &["detect"],
            "pub use crate::game_running;\npub use crate::input::{MultiKeyboardStream, TabKey};\nfn f(_: TabKey) { self::game_running::x(); }\n",
        ),
        source(
            "detect/own_row.rs",
            &["detect", "own_row"],
            "use super::TabKey;\n",
        ),
        source(
            "shadow/worker.rs",
            &["shadow", "worker"],
            "fn g() { crate::detect::game_running::GameProcessGate::new(); }\nuse crate::detect::*;\nuse stat_tracker::detect;\nfn h() { crate::detect::r#TabKey::f(); }\nuse crate::detect::{r#MultiKeyboardStream};\n",
        ),
    ];
    let mut bad: Vec<String> = compat_alias_uses(&sources)
        .into_iter()
        .map(|b| b.rsplit(": ").next().unwrap_or("").to_string())
        .collect();
    bad.sort();
    assert_eq!(
        bad,
        [
            "bare TabKey",
            "bare game_running",
            "crate::detect::MultiKeyboardStream",
            "crate::detect::TabKey::f",
            "crate::detect::game_running::GameProcessGate::new",
            "self::game_running::x",
            "super::TabKey",
            "use of crate::detect",
            "use of stat_tracker::detect",
        ]
    );
}
