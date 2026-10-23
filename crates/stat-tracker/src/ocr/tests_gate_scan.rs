//! The source scan behind OCR-off mode: no production code outside the gate
//! starts Tesseract or opens the gate.
//!
//! "Production code" is every `.rs` file below the `src/` of every tracker
//! crate but the GUI, with its test items left out: each item behind a
//! `#[cfg(test)]` (or `#[cfg(all(test, …))]`) attribute, wherever in the file
//! it is declared. The rest of the file is scanned, also the code after an
//! early `#[cfg(test)] mod tests;`. A file is left out as a whole only when
//! every declaration that loads it is test-only (`#[cfg(test)] mod x;`, or a
//! declaration inside a test module); never by its name.
//!
//! It is a text search, not name resolution: it misses an alias
//! (`use leptess::LepTess as T; T::new(…)`, `<LepTess>::new(…)`) and a
//! function pointer (`let read = ocr_with;`). It does see `name (` with a
//! space. Review such indirection by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `src` with comments, string literals and char literals replaced by
/// spaces. Newlines stay, so offsets and line numbers still match.
fn code_only(src: &[char]) -> Vec<char> {
    let n = src.len();
    let mut out = src.to_vec();
    let wipe = |out: &mut Vec<char>, a: usize, b: usize| {
        for c in &mut out[a..b.min(n)] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let mut i = 0;
    while i < n {
        let next = src.get(i + 1).copied();
        if src[i] == '/' && next == Some('/') {
            let end = (i..n).find(|&k| src[k] == '\n').unwrap_or(n);
            wipe(&mut out, i, end);
            i = end;
        } else if src[i] == '/' && next == Some('*') {
            // Block comments nest.
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
        } else if let Some(end) = string_end(src, i) {
            wipe(&mut out, i, end);
            i = end;
        } else if src[i] == '\'' {
            // A char literal ('x', '\n', '\'', '\u{7d}'), or a lifetime or
            // label ('a, 'outer:), which is code.
            let end = if next == Some('\\') {
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
/// `r#"…"#`, `br"…"`, `cr"…"`), or `None` when none starts there.
fn string_end(src: &[char], i: usize) -> Option<usize> {
    if i > 0 && is_ident(src[i - 1]) {
        return None;
    }
    let mut k = i;
    if matches!(src.get(k), Some('b' | 'c')) {
        k += 1;
    }
    let raw = src.get(k) == Some(&'r');
    if raw {
        k += 1;
    }
    let hashes = src[k.min(src.len())..]
        .iter()
        .take_while(|c| **c == '#')
        .count();
    k += hashes;
    if src.get(k) != Some(&'"') || (!raw && hashes > 0) {
        return None;
    }
    k += 1;
    while k < src.len() {
        if raw {
            if src[k] == '"' && src[k + 1..].iter().take(hashes).all(|c| *c == '#') {
                let end = k + 1 + hashes;
                if end <= src.len() {
                    return Some(end);
                }
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

/// Index of the bracket that closes the one at `open` (code-only text).
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
    t.len() - 1
}

fn skip_ws(t: &[char], mut k: usize) -> usize {
    while k < t.len() && t[k].is_whitespace() {
        k += 1;
    }
    k
}

fn starts_with(t: &[char], at: usize, s: &str) -> bool {
    s.chars()
        .enumerate()
        .all(|(k, c)| t.get(at + k) == Some(&c))
}

/// The identifier at `k` (empty when none starts there).
fn word_at(t: &[char], k: usize) -> String {
    t[k.min(t.len())..]
        .iter()
        .take_while(|c| is_ident(**c))
        .collect()
}

/// The cfg predicate `pred` (no whitespace) holds only when `test` does:
/// `test`, or `all(…)` with such an argument. `not(test)` and `any(test, …)`
/// do not, so the code behind them stays in the scan.
fn needs_test(pred: &str) -> bool {
    if pred == "test" {
        return true;
    }
    let Some(inner) = pred
        .strip_prefix("all(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return false;
    };
    let (mut depth, mut start) = (0i32, 0);
    let mut args = Vec::new();
    for (k, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&inner[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    args.push(&inner[start..]);
    args.into_iter().any(needs_test)
}

/// What starts at `k`: the item keyword after any visibility and qualifiers
/// (`fn`, `mod`, `impl`, `use`, `const`, `static`, `let`, …), the first word
/// of anything else (a field, a variant, a macro call), or "" (a block).
fn item_kind(t: &[char], mut k: usize) -> String {
    loop {
        k = skip_ws(t, k);
        let word = word_at(t, k);
        k += word.chars().count();
        match word.as_str() {
            "pub" => {
                let j = skip_ws(t, k);
                if t.get(j) == Some(&'(') {
                    k = closing(t, j) + 1;
                }
            }
            "unsafe" | "async" | "extern" | "default" | "safe" => {}
            // An inline `const { … }` block, not a `const` item.
            "const" if t.get(skip_ws(t, k)) == Some(&'{') => return String::new(),
            "const"
                if matches!(
                    word_at(t, skip_ws(t, k)).as_str(),
                    "fn" | "unsafe" | "async" | "extern"
                ) => {}
            _ => return word,
        }
    }
}

/// The end (exclusive) of the item, statement, field or arm that starts at
/// `k` in code-only text `t`. When unsure it ends early: what is left over is
/// scanned, which can only make the scan stricter.
fn item_end(t: &[char], k: usize) -> usize {
    let kind = item_kind(t, k);
    // Items that end at their `;` even past a brace group: `use a::{b};`,
    // `const X: S = S { a: 1 };`.
    let to_semicolon = matches!(kind.as_str(), "use" | "const" | "static" | "type" | "let");
    // Items with commas outside brackets (`fn f<A, B>`, `impl<A, B>`).
    let commas_inside = to_semicolon
        || matches!(
            kind.as_str(),
            "fn" | "mod" | "impl" | "trait" | "struct" | "enum" | "union" | "macro_rules"
        );
    let mut depth = 0i32;
    for (i, c) in t.iter().enumerate().skip(k) {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                // It closes a group around the item: the item ended before.
                if depth < 0 {
                    return i;
                }
                if depth == 0 && *c == '}' && !to_semicolon {
                    return i + 1;
                }
            }
            ';' if depth == 0 => return i + 1,
            ',' if depth == 0 && !commas_inside => return i + 1,
            _ => {}
        }
    }
    t.len()
}

/// One item behind a test-only `#[cfg(…)]`: `start..end` covers its
/// attributes and the item.
struct TestItem {
    start: usize,
    end: usize,
}

/// Every item of `t` (code-only) behind a test-only `#[cfg(…)]`, nested ones
/// included, in order of their start.
fn test_items(t: &[char]) -> Vec<TestItem> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if !starts_with(t, i, "#[cfg(") {
            i += 1;
            continue;
        }
        let close = closing(t, i + 5);
        let pred: String = t[i + 6..close]
            .iter()
            .filter(|c| !c.is_whitespace())
            .collect();
        // Past this attribute and any other attributes on the same item.
        let mut k = skip_ws(t, closing(t, i + 1) + 1);
        while starts_with(t, k, "#[") {
            k = skip_ws(t, closing(t, k + 1) + 1);
        }
        if needs_test(&pred) {
            out.push(TestItem {
                start: i,
                end: item_end(t, k),
            });
        }
        i = close + 1;
    }
    out
}

/// `source` with every test item replaced by spaces (newlines stay, so line
/// numbers still match): the production code of one file.
pub(super) fn production_text(source: &str) -> String {
    let src: Vec<char> = source.chars().collect();
    let mut out = src.clone();
    for item in test_items(&code_only(&src)) {
        for c in &mut out[item.start..item.end] {
            if *c != '\n' {
                *c = ' ';
            }
        }
    }
    out.into_iter().collect()
}

/// The names of the inline modules (`mod name { … }`) around offset `at`.
fn inline_modules_at(t: &[char], at: usize) -> Vec<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    for (i, c) in t.iter().enumerate().take(at) {
        match c {
            '{' => {
                // `mod name {`: walk back over the name and the keyword.
                let (name, start) = word_before(t, i);
                let (keyword, _) = word_before(t, start);
                stack.push((keyword == "mod" && !name.is_empty()).then_some(name));
            }
            '}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.into_iter().flatten().collect()
}

/// The identifier that ends right before `at` (whitespace skipped), and
/// where it starts; empty when there is none.
fn word_before(t: &[char], at: usize) -> (String, usize) {
    let mut end = at.min(t.len());
    while end > 0 && t[end - 1].is_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident(t[start - 1]) {
        start -= 1;
    }
    (t[start..end].iter().collect(), start)
}

/// Index of the bracket that opens the one that closes at `close`.
fn opening(t: &[char], close: usize) -> usize {
    let (o, c) = match t[close] {
        '}' => ('{', '}'),
        ')' => ('(', ')'),
        _ => ('[', ']'),
    };
    let mut depth = 0usize;
    for k in (0..=close).rev() {
        if t[k] == c {
            depth += 1;
        } else if t[k] == o {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    0
}

/// Where the item whose keyword is at `at` starts: before its visibility
/// (`pub`, `pub(crate)`) and its attributes.
fn item_start(t: &[char], mut at: usize) -> usize {
    loop {
        let mut q = at;
        while q > 0 && t[q - 1].is_whitespace() {
            q -= 1;
        }
        if q == 0 {
            return at;
        }
        match t[q - 1] {
            ']' => {
                let open = opening(t, q - 1);
                if open == 0 || t[open - 1] != '#' {
                    return at;
                }
                at = open - 1;
            }
            ')' => {
                let (word, start) = word_before(t, opening(t, q - 1));
                if word != "pub" {
                    return at;
                }
                at = start;
            }
            _ => {
                let (word, start) = word_before(t, q);
                if word != "pub" {
                    return at;
                }
                at = start;
            }
        }
    }
}

/// One out-of-line `mod name;` declaration.
#[derive(Debug)]
struct ModDecl {
    /// Where its file may be; the first one that exists is it.
    candidates: Vec<PathBuf>,
    /// Loaded through `#[path]`: such a file owns its directory.
    path_attr: bool,
    /// Behind a test-only cfg: its own, or that of an item around it.
    test: bool,
}

/// The out-of-line `mod name;` declarations of the file `path`, resolved by
/// the Rust reference's rules. `mod_rs`: the file owns its directory (a
/// crate root, a `mod.rs`, a file loaded through `#[path]`); otherwise its
/// child modules live in `dir/stem/`. The file name alone does not say: a
/// `lib.rs` that is not a crate root is a plain module file.
fn mod_decls(path: &Path, mod_rs: bool, source: &str) -> Vec<ModDecl> {
    let src: Vec<char> = source.chars().collect();
    let t = code_only(&src);
    let tests = test_items(&t);
    let dir = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mut out = Vec::new();
    for at in 0..t.len() {
        if !starts_with(&t, at, "mod")
            || (at > 0 && is_ident(t[at - 1]))
            || t.get(at + 3).is_some_and(|c| is_ident(*c))
        {
            continue;
        }
        let name_at = skip_ws(&t, at + 3);
        let name = word_at(&t, name_at);
        if name.is_empty() || t.get(skip_ws(&t, name_at + name.chars().count())) != Some(&';') {
            continue;
        }
        // `#[path = "…"]` from the raw text (strings are blank in `t`).
        let attrs: String = src[item_start(&t, at)..at].iter().collect();
        let path_attr = attrs.find("#[path").and_then(|p| {
            let rest = attrs[p + 6..].trim_start().strip_prefix('=')?;
            let rest = rest.trim_start().strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        });
        let inline = inline_modules_at(&t, at);
        let mut base = dir.to_path_buf();
        if !mod_rs && (path_attr.is_none() || !inline.is_empty()) {
            base.push(stem);
        }
        base.extend(&inline);
        out.push(ModDecl {
            candidates: match &path_attr {
                Some(file) => vec![base.join(file)],
                None => vec![
                    base.join(format!("{name}.rs")),
                    base.join(&name).join("mod.rs"),
                ],
            },
            path_attr: path_attr.is_some(),
            test: tests.iter().any(|item| item.start <= at && at < item.end),
        });
    }
    out
}

/// The crate roots among `files` below `src`: `lib.rs`, `main.rs`,
/// `bin/*.rs`, `bin/*/main.rs`.
fn crate_roots(files: &BTreeMap<PathBuf, String>, src: &Path) -> Vec<PathBuf> {
    files
        .keys()
        .filter(|path| {
            let Ok(rel) = path.strip_prefix(src) else {
                return false;
            };
            let parts: Vec<&str> = rel.iter().filter_map(|p| p.to_str()).collect();
            matches!(parts.as_slice(), ["lib.rs" | "main.rs"] | ["bin", _])
                || matches!(parts.as_slice(), ["bin", _, "main.rs"])
        })
        .cloned()
        .collect()
}

/// The files the module tree reaches from `roots`, through every
/// declaration (`with_tests`) or through those that are not test-only.
fn reached(
    files: &BTreeMap<PathBuf, String>,
    roots: &[PathBuf],
    with_tests: bool,
) -> BTreeSet<PathBuf> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut todo: Vec<(PathBuf, bool)> = roots.iter().map(|root| (root.clone(), true)).collect();
    while let Some((path, mod_rs)) = todo.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        for decl in mod_decls(&path, mod_rs, &files[&path]) {
            if decl.test && !with_tests {
                continue;
            }
            if let Some(file) = decl.candidates.iter().find(|c| files.contains_key(*c)) {
                let owns_dir = decl.path_attr || file.file_name().is_some_and(|n| n == "mod.rs");
                todo.push((file.clone(), owns_dir));
            }
        }
    }
    seen
}

/// The test-only files among `files` (a crate's `src/`): reached through a
/// test-only declaration and through no other. A file that a production
/// declaration also loads (a `cfg(not(test))` twin, a `#[path]`) is
/// production code, and so is a file no declaration reaches.
fn test_only_files(files: &BTreeMap<PathBuf, String>, src: &Path) -> BTreeSet<PathBuf> {
    let roots = crate_roots(files, src);
    let production = reached(files, &roots, false);
    reached(files, &roots, true)
        .into_iter()
        .filter(|path| !production.contains(path))
        .collect()
}

/// The scanned tree: every `.rs` file below the `src/` of every
/// stat-tracker crate but the GUI (`crates/stat-tracker*/src`: this lib,
/// its bin `main.rs` and the crates split out of it in 0.6 N4) that is not
/// test-only, with its production code ([`production_text`]); and the
/// test-only files left out. Names are crate-qualified
/// (`stat-tracker/src/ocr/mod.rs`), so two crates' `lib.rs` stay apart.
pub(super) fn production_sources() -> (Vec<(String, String)>, Vec<String>) {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read src") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/");
    let mut roots: Vec<PathBuf> = std::fs::read_dir(crates)
        .expect("read crates/")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|dir| {
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // The GUI is no OCR caller, and its `set_mode(` calls are
            // std file permissions.
            name.starts_with("stat-tracker") && name != "stat-tracker-ui"
        })
        .map(|dir| dir.join("src"))
        .filter(|src| src.is_dir())
        .collect();
    roots.sort();
    let (mut scanned, mut left_out) = (Vec::new(), Vec::new());
    for src in roots {
        let mut paths = Vec::new();
        walk(&src, &mut paths);
        let files: BTreeMap<PathBuf, String> = paths
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).expect("read a source file");
                (path, text)
            })
            .collect();
        let test_only = test_only_files(&files, &src);
        let crate_dir = src
            .parent()
            .and_then(|dir| dir.file_name())
            .and_then(|name| name.to_str())
            .expect("a crate directory")
            .to_string();
        for (path, text) in files {
            let name = format!(
                "{crate_dir}/src/{}",
                path.strip_prefix(&src)
                    .expect("below src")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            if test_only.contains(&path) {
                left_out.push(name);
            } else {
                scanned.push((name, production_text(&text)));
            }
        }
    }
    (scanned, left_out)
}

/// Each call of `name` in `sources` (`name(`, also `name (`), as
/// `file:line`.
fn sites(sources: &[(String, String)], name: &str) -> Vec<String> {
    sources
        .iter()
        .flat_map(|(file, text)| {
            text.match_indices(name)
                .filter(|(at, _)| text[at + name.len()..].trim_start().starts_with('('))
                .map(move |(at, _)| format!("{file}:{}", text[..at].matches('\n').count() + 1))
        })
        .collect()
}

/// The files of `sites`, each once, in order.
fn files_of(sites: &[String]) -> Vec<String> {
    let mut files: Vec<String> = sites
        .iter()
        .map(|site| site.rsplit_once(':').expect("file:line").0.to_string())
        .collect();
    files.dedup();
    files
}

/// The scanned tree has at least this many files (one less than on
/// 2026-10-23), so a module-tree bug cannot drop files from the scan
/// unseen.
const MIN_SCANNED: usize = 128;

/// Nothing outside the gate can start Tesseract or open the gate:
/// - one `LepTess` construction in the whole tree, in `ocr_with`;
/// - `ocr_with` is called from `ocr/mod.rs` only;
/// - the process-wide mode is set from `ocr_switch.rs` only, and no
///   code outside tests sets a scoped mode.
#[test]
fn nothing_outside_the_gate_starts_tesseract_or_opens_the_gate() {
    // Built at run time, so this test does not find itself (its file is
    // test code and left out anyway).
    let construct = format!("LepTess::{}", "new");
    let read = format!("ocr_{}", "with");
    let set_mode = format!("set_{}", "mode");
    let scoped = format!("scop{}", "ed");
    let (sources, left_out) = production_sources();
    assert!(
        sources.len() >= MIN_SCANNED,
        "the walk found the tree: {} files scanned",
        sources.len()
    );
    // The daemon binary and stat-tracker-core are in the walk, not only
    // the crate that hosts this test.
    for file in [
        "stat-tracker/src/main.rs",
        "stat-tracker/src/lib.rs",
        "stat-tracker/src/ocr/mod.rs",
        "stat-tracker-core/src/lib.rs",
        "stat-tracker-core/src/config.rs",
        "stat-tracker-core/src/ocr_status.rs",
    ] {
        assert!(
            sources.iter().any(|(name, _)| name == file),
            "the walk reached {file}"
        );
    }
    // Test code as a whole stays out: main.rs's `#[cfg(test)] mod
    // main_tests;` and the files below it.
    assert!(
        left_out
            .iter()
            .any(|name| name == "stat-tracker/src/main_tests/mod.rs")
    );
    assert!(
        !sources
            .iter()
            .any(|(name, _)| name.starts_with("stat-tracker/src/main_tests/")),
        "main_tests/ is test code"
    );
    // A file left out is named as test code (`tests*`, `*_tests`,
    // `ocr_bench`, or below such a module), or pinned here: a new
    // exclusion of any other file is a visible diff.
    let test_named = |name: &str| {
        name.split('/').skip(2).any(|part| {
            let stem = part.strip_suffix(".rs").unwrap_or(part);
            stem.starts_with("tests") || stem.ends_with("_tests") || stem == "ocr_bench"
        })
    };
    let other: Vec<&str> = left_out
        .iter()
        .map(String::as_str)
        .filter(|name| !test_named(name))
        .collect();
    assert_eq!(
        other,
        [
            "stat-tracker/src/report/blank/port_check.rs",
            "stat-tracker/src/report/blank/synth.rs",
        ],
        "files left out as test-only, all: {left_out:?}"
    );
    let built = sites(&sources, &construct);
    assert_eq!(
        built.len(),
        1,
        "one Tesseract instance is built, in ocr_with: {built:?}"
    );
    assert_eq!(
        files_of(&built),
        ["stat-tracker/src/ocr/mod.rs"],
        "one Tesseract instance is built, in ocr_with: {built:?}"
    );
    let callers = sites(&sources, &read);
    assert_eq!(
        files_of(&callers),
        ["stat-tracker/src/ocr/mod.rs"],
        "ocr_with is private to ocr/mod.rs: {callers:?}"
    );
    let setters = sites(&sources, &set_mode);
    assert_eq!(
        files_of(&setters),
        [
            "stat-tracker/src/ocr/gate.rs",
            "stat-tracker/src/ocr_switch.rs"
        ],
        "only the switch sets the process-wide mode: {setters:?}"
    );
    let scopes = sites(&sources, &scoped);
    assert_eq!(
        files_of(&scopes),
        ["stat-tracker/src/ocr/gate.rs"],
        "a scoped mode is for tests: {scopes:?}"
    );
}

/// The lines of `text` that hold `needle`, 1-based.
fn lines_with(text: &str, needle: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(needle))
        .map(|(k, _)| k + 1)
        .collect()
}

/// The blind spot this scanner closes: a test module declared near the top
/// of a file used to end the scan there.
#[test]
fn production_code_after_an_early_test_module_is_scanned() {
    let source = "\
//! A module.

#[cfg(test)]
mod tests;

#[cfg(test)]
mod inline_tests {
    fn t() { gate::set_mode(Off); }
}

pub fn production() {
    gate::set_mode(On);
}
";
    let text = production_text(source);
    assert_eq!(lines_with(&text, "set_mode("), [12]);
    assert!(!text.contains("mod tests;"));
    assert!(!text.contains("inline_tests"));
    assert_eq!(text.lines().count(), source.lines().count());
}

/// Test items of every kind are left out; production items around them
/// stay, and so do `not(test)` and `any(test, …)` items.
#[test]
fn test_items_of_every_kind_are_left_out() {
    let source = r#"
#[cfg(test)]
use crate::{a, b::{c, d}};
#[cfg(test)]
fn helper<A, B>(a: A, b: B) -> [u8; 2] { LepTess::new("eng"); [0; 2] }
#[cfg(test)]
#[allow(dead_code)]
impl Drop for Guard { fn drop(&mut self) { set_mode(Off); } }
#[cfg(all(test, unix))]
static LOCK: Mutex<()> = Mutex::new(());
#[cfg(test)]
const X: S = S { a: scoped(Off) };
#[cfg(test)]
thread_local! { static C: Cell<u32> = const { Cell::new(0) }; }
#[cfg(test)]
pub(crate) const fn probe() -> bool { ocr_with() }
#[cfg(not(test))]
fn not_test() { set_mode(A); }
#[cfg(any(test, feature = "bench"))]
fn any_test() { set_mode(B); }
struct S {
    #[cfg(test)]
    seen: HashMap<u8, u32>,
    kept: u8,
}
fn production() {
    #[cfg(test)]
    let _guard = S { a: scoped(Off) };
    #[cfg(test)]
    {
        set_mode(Off);
    }
    #[cfg(test)]
    const { assert!(true) }
    set_mode(D);
    set_mode(C);
}
"#;
    let text = production_text(source);
    assert_eq!(lines_with(&text, "set_mode("), [18, 20, 35, 36]);
    assert!(!text.contains("assert!(true)"));
    for gone in [
        "LepTess",
        "scoped(",
        "ocr_with",
        "use crate",
        "LOCK",
        "thread_local",
        "Guard",
    ] {
        assert!(!text.contains(gone), "{gone} is test code:\n{text}");
    }
    for kept in ["struct S {", "kept: u8,", "fn production() {"] {
        assert!(text.contains(kept), "{kept} is production code:\n{text}");
    }
    // The field's type is cut at its first comma: what is left over is
    // scanned, never more than the item is skipped.
    assert!(!text.contains("seen"));
}

/// Braces, quotes and `#[cfg(test)]` inside strings, raw strings, chars
/// and comments neither end a test item nor start one.
#[test]
fn braces_in_strings_and_comments_do_not_move_item_ends() {
    let source = r##"
#[cfg(test)]
fn t() {
    let s = "}\"}";
    let r = r#"}"{"#;
    let b = br"}";
    let c = '}';
    let q = '\'';
    let l: &'static str = "{";
    // }
    /* } /* } */ { */
    set_mode(Off);
}
const S: &str = "#[cfg(test)]\nmod x {";
// #[cfg(test)]
fn production<'a>(x: &'a str) {
    set_mode(On);
}
"##;
    let text = production_text(source);
    assert_eq!(lines_with(&text, "set_mode("), [17]);
    assert!(text.contains("const S"));
}

/// Test modules inside production modules, and modules nested in test
/// modules, are left out; the production code around them stays.
#[test]
fn nested_modules() {
    let source = "
mod outer {
    #[cfg(test)]
    mod inner {
        mod deeper {
            fn t() { set_mode(Off); }
        }
        fn u() { set_mode(Off); }
    }
    fn production() { set_mode(On); }
    mod kept {
        #[cfg(test)]
        mod tests { fn t() { set_mode(Off); } }
        fn production() { set_mode(On); }
    }
}
fn after() { set_mode(On); }
";
    let text = production_text(source);
    assert_eq!(lines_with(&text, "set_mode("), [10, 14, 17]);
}

/// The test-only files of a synthetic `src/`, as `src/…` names.
fn test_only_of(files: &[(&str, &str)]) -> Vec<String> {
    let files: BTreeMap<PathBuf, String> = files
        .iter()
        .map(|(path, text)| (PathBuf::from(path), text.to_string()))
        .collect();
    test_only_files(&files, Path::new("src"))
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect()
}

/// A file is never left out for its name: a production module named
/// `tests_*` is scanned. Only test-only declarations leave a file out, and
/// with it the modules below it.
#[test]
fn files_are_left_out_by_their_declarations_not_their_names() {
    assert_eq!(
        test_only_of(&[
            (
                "src/lib.rs",
                "pub mod tests_prod_helper;\n#[cfg(test)]\nmod tests;\n"
            ),
            ("src/tests_prod_helper.rs", "fn f() { set_mode(On); }\n"),
            ("src/tests.rs", ""),
        ]),
        ["src/tests.rs"]
    );
    assert_eq!(
        test_only_of(&[
            (
                "src/main.rs",
                "mod run;\n#[cfg(test)]\nmod\n    main_tests;\n#[cfg(test)]\nmod ocr_bench;\n"
            ),
            ("src/run.rs", ""),
            ("src/ocr_bench.rs", ""),
            ("src/main_tests/mod.rs", "mod cell_trust;\n"),
            ("src/main_tests/cell_trust.rs", ""),
        ]),
        [
            "src/main_tests/cell_trust.rs",
            "src/main_tests/mod.rs",
            "src/ocr_bench.rs"
        ]
    );
}

/// A file that a production declaration also loads is production code: a
/// `cfg(not(test))` twin of a test declaration, or a `#[path]` to a file
/// that also has its own declaration.
#[test]
fn a_file_is_test_only_when_every_declaration_of_it_is() {
    assert_eq!(
        test_only_of(&[
            (
                "src/lib.rs",
                r#"#[cfg(test)]
pub(crate) mod synth;
#[cfg(not(test))]
pub(crate) mod synth;
mod prod;
#[cfg(test)]
#[path = "prod.rs"]
mod again;
#[cfg(test)]
#[path = "only_test.rs"]
mod t;
"#
            ),
            ("src/synth.rs", ""),
            ("src/prod.rs", ""),
            ("src/only_test.rs", ""),
        ]),
        ["src/only_test.rs"]
    );
}

/// Where a declaration's file is: below a non-mod-rs file (a `lib.rs` that
/// is no crate root is one), inside an inline module, inside a test module,
/// and through `#[path]`. A file no declaration reaches is scanned.
#[test]
fn test_only_files_follow_the_module_rules() {
    assert_eq!(
        test_only_of(&[
            ("src/lib.rs", "mod a;\npub mod detect;\n"),
            ("src/a/mod.rs", "mod lib;\nmod b;\n"),
            ("src/a/lib.rs", "#[cfg(test)]\nmod t;\n"),
            ("src/a/t.rs", ""),
            ("src/a/lib/t.rs", ""),
            ("src/a/b.rs", "#[cfg(test)]\npub(crate) mod synth;\n"),
            ("src/a/b/synth.rs", ""),
            ("src/detect/mod.rs", "mod own_row;\n"),
            (
                "src/detect/own_row.rs",
                "#[cfg(test)]\n#[path = \"own_row_tests.rs\"]\nmod tests;\n"
            ),
            ("src/detect/own_row_tests.rs", ""),
        ]),
        [
            "src/a/b/synth.rs",
            "src/a/lib/t.rs",
            "src/detect/own_row_tests.rs"
        ]
    );
    assert_eq!(
        test_only_of(&[
            (
                "src/lib.rs",
                "mod outer {\n    #[cfg(test)]\n    mod t;\n}\n#[cfg(test)]\nmod inl {\n    mod deeper;\n}\nmod my_mod;\n"
            ),
            ("src/outer/t.rs", ""),
            ("src/inl/deeper.rs", ""),
            ("src/t.rs", ""),
            ("src/my_mod.rs", ""),
        ]),
        ["src/inl/deeper.rs", "src/outer/t.rs"]
    );
}

/// A call is found with a space before its parenthesis too.
#[test]
fn sites_see_a_space_before_the_parenthesis() {
    let sources = [(
        "a.rs".to_string(),
        "fn f() {\n    set_mode (On);\n    set_mode(Off);\n    let x = set_mode;\n}\n".to_string(),
    )];
    assert_eq!(sites(&sources, "set_mode"), ["a.rs:2", "a.rs:3"]);
}
