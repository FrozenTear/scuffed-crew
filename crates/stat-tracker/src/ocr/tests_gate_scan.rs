//! The source scan behind OCR-off mode: no production code outside the gate
//! starts Tesseract or opens the gate.
//!
//! "Production code" is every `.rs` file below the `src/` of every tracker
//! crate but the GUI, with its test items left out: each item behind a
//! `#[cfg(test)]` (or `#[cfg(all(test, …))]`) attribute, wherever in the file
//! it is declared. The rest of the file is scanned, also the code after an
//! early `#[cfg(test)] mod tests;`. Files that are test code as a whole (named
//! `tests*`, or reached only through a `#[cfg(test)] mod x;`) are left out.

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
/// attributes and the item; `item` is where the item itself starts.
struct TestItem {
    start: usize,
    item: usize,
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
                item: k,
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
                let mut k = i;
                while k > 0 && t[k - 1].is_whitespace() {
                    k -= 1;
                }
                let name_end = k;
                while k > 0 && is_ident(t[k - 1]) {
                    k -= 1;
                }
                let name: String = t[k..name_end].iter().collect();
                while k > 0 && t[k - 1].is_whitespace() {
                    k -= 1;
                }
                let is_mod = k >= 3
                    && t[k - 3..k] == ['m', 'o', 'd']
                    && (k == 3 || !is_ident(t[k - 4]))
                    && !name.is_empty();
                stack.push(is_mod.then_some(name));
            }
            '}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.into_iter().flatten().collect()
}

/// The files (and module directories) that the `#[cfg(test)] mod x;`
/// declarations of `path` load: test code as a whole, reached only through
/// a test-only declaration. Paths follow the Rust reference, `#[path]`
/// included; they need not exist.
pub(super) fn test_module_files(path: &Path, source: &str) -> Vec<PathBuf> {
    let src: Vec<char> = source.chars().collect();
    let t = code_only(&src);
    let dir = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let mod_rs = matches!(stem, "mod" | "lib" | "main");
    let mut out = Vec::new();
    for item in test_items(&t) {
        if item_kind(&t, item.item) != "mod" || t.get(item.end - 1) != Some(&';') {
            continue;
        }
        let at = item.item
            + t[item.item..]
                .iter()
                .collect::<String>()
                .find("mod ")
                .unwrap_or(0)
            + 4;
        let name = word_at(&t, skip_ws(&t, at));
        let inline = inline_modules_at(&t, item.start);
        // `#[path = "…"]` from the raw text (strings are blank in `t`).
        let attrs: String = src[item.start..item.item].iter().collect();
        let path_attr = attrs.find("#[path").and_then(|at| {
            let rest = attrs[at + 6..].trim_start().strip_prefix('=')?;
            let rest = rest.trim_start().strip_prefix('"')?;
            Some(rest[..rest.find('"')?].to_string())
        });
        let mut base = dir.to_path_buf();
        if !mod_rs && (path_attr.is_none() || !inline.is_empty()) {
            base.push(stem);
        }
        base.extend(&inline);
        match path_attr {
            Some(file) => {
                let file = base.join(file);
                out.push(file.with_extension(""));
                out.push(file);
            }
            None => {
                out.push(base.join(format!("{name}.rs")));
                out.push(base.join(&name));
            }
        }
    }
    out
}

/// Every `.rs` file below the `src/` of every stat-tracker crate but the
/// GUI (`crates/stat-tracker*/src`: this lib, its bin `main.rs` and the
/// crates split out of it in 0.6 N4), with its production code
/// ([`production_text`]). Names are crate-qualified
/// (`stat-tracker/src/ocr/mod.rs`), so two crates' `lib.rs` stay apart.
pub(super) fn production_sources() -> Vec<(String, String)> {
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
    let mut out = Vec::new();
    for src in roots {
        let mut files = Vec::new();
        walk(&src, &mut files);
        files.sort();
        let texts: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|path| {
                let text = std::fs::read_to_string(&path).expect("read a source file");
                (path, text)
            })
            .collect();
        let test_only: Vec<PathBuf> = texts
            .iter()
            .flat_map(|(path, text)| test_module_files(path, text))
            .collect();
        let crate_dir = src
            .parent()
            .and_then(|dir| dir.file_name())
            .and_then(|name| name.to_str())
            .expect("a crate directory")
            .to_string();
        for (path, text) in texts {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // `mod tests;` files, the OCR-off bench, the daemon's
            // `main_tests/` (main.rs's test module) and every other module
            // loaded only by a `#[cfg(test)] mod x;` are test code.
            if stem.starts_with("tests")
                || stem == "ocr_bench"
                || path.starts_with(src.join("main_tests"))
                || test_only.iter().any(|test| path.starts_with(test))
            {
                continue;
            }
            let name = format!(
                "{crate_dir}/src/{}",
                path.strip_prefix(&src)
                    .expect("below src")
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            out.push((name, production_text(&text)));
        }
    }
    out
}

/// Each place `needle` occurs in `sources`, as `file:line`.
fn sites(sources: &[(String, String)], needle: &str) -> Vec<String> {
    sources
        .iter()
        .flat_map(|(name, text)| {
            text.match_indices(needle)
                .map(move |(at, _)| format!("{name}:{}", text[..at].matches('\n').count() + 1))
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

/// Nothing outside the gate can start Tesseract or open the gate:
/// - one `LepTess` construction in the whole tree, in `ocr_with`;
/// - `ocr_with` is called from `ocr/mod.rs` only;
/// - the process-wide mode is set from `ocr_switch.rs` only, and no
///   code outside tests sets a scoped mode.
#[test]
fn nothing_outside_the_gate_starts_tesseract_or_opens_the_gate() {
    // Built at run time, so this test does not find itself (its file is
    // test code and left out anyway).
    let construct = format!("LepTess::{}(", "new");
    let read = format!("ocr_{}(", "with");
    let set_mode = format!("set_{}(", "mode");
    let scoped = format!("scop{}(", "ed");
    let sources = production_sources();
    assert!(
        sources.len() > 20,
        "the walk found the tree: {}",
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
        !sources
            .iter()
            .any(|(name, _)| name.starts_with("stat-tracker/src/main_tests/")),
        "main_tests/ is test code"
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
    set_mode(C);
}
"#;
    let text = production_text(source);
    assert_eq!(lines_with(&text, "set_mode("), [18, 20, 33]);
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

/// The files a `#[cfg(test)] mod x;` loads, by the Rust reference's rules:
/// from a mod-rs file, from a non-mod-rs file, inside an inline module and
/// through `#[path]`. Non-test declarations load nothing here.
#[test]
fn test_module_files_follow_the_module_rules() {
    let files = |path: &str, source: &str| -> Vec<String> {
        test_module_files(Path::new(path), source)
            .into_iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect()
    };
    assert_eq!(
        files(
            "src/main.rs",
            "mod prod;\n#[cfg(test)]\nmod main_tests;\nfn main() {}\n"
        ),
        ["src/main_tests.rs", "src/main_tests"]
    );
    assert_eq!(
        files("src/a/b.rs", "#[cfg(test)]\npub(crate) mod synth;\n"),
        ["src/a/b/synth.rs", "src/a/b/synth"]
    );
    assert_eq!(
        files(
            "src/detect/own_row.rs",
            "#[cfg(test)]\n#[path = \"own_row_tests.rs\"]\nmod tests;\n"
        ),
        ["src/detect/own_row_tests", "src/detect/own_row_tests.rs"]
    );
    assert_eq!(
        files(
            "src/lib.rs",
            "mod outer {\n    #[cfg(test)]\n    mod t;\n}\n#[cfg(test)]\nmod inline { fn x() {} }\n"
        ),
        ["src/outer/t.rs", "src/outer/t"]
    );
}
