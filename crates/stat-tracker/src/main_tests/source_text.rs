//! The daemon's source as text, for the tests that check it by reading it
//! (0.6 MAIN-14).
//!
//! The 0.6 split moves code out of `main.rs` into `src/daemon/`. A test
//! that scans `main.rs` alone would then pass on less code than it means
//! to check, and a slice that ends at "the next function in the file" would
//! end somewhere else. These helpers read every daemon file and find a
//! function by its name, wherever it lives.

use std::path::Path;
use std::sync::OnceLock;

/// `main.rs` and every `.rs` file under `src/daemon/` (sorted by path),
/// joined by a newline. Read from disk once per test run, so a new daemon
/// file is never left out. Dot files (editor locks and swap files) are
/// skipped, as the GUI build id's walk skips them.
pub(crate) fn daemon_source() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut daemon = Vec::new();
        let mut todo = vec![src.join("daemon")];
        while let Some(dir) = todo.pop() {
            let entries =
                std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("dir entry").path();
                if path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                if path.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    daemon.push(path);
                }
            }
        }
        assert!(!daemon.is_empty(), "no .rs file under src/daemon");
        daemon.sort();
        std::iter::once(src.join("main.rs"))
            .chain(daemon)
            .map(|path| {
                std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// The top-level function `name` in [`daemon_source`], from its `fn` line
/// (`pub(crate) ` and `async ` allowed) to its closing `}` at column 0.
/// Panics unless exactly one such function exists, so a renamed or doubled
/// function fails the test that names it instead of letting it pass on
/// other text.
pub(crate) fn fn_source(name: &str) -> &'static str {
    let src = daemon_source();
    let mut starts = Vec::new();
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let rest = line.strip_prefix("pub(crate) ").unwrap_or(line);
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let after = rest.strip_prefix("fn ").and_then(|r| r.strip_prefix(name));
        if after.is_some_and(|a| a.starts_with('(') || a.starts_with('<')) {
            starts.push(at);
        }
        at += line.len();
    }
    assert_eq!(starts.len(), 1, "fn {name} found {} times", starts.len());
    let body = &src[starts[0]..];
    let end = body
        .find("\n}\n")
        .unwrap_or_else(|| panic!("fn {name} has no closing brace"));
    &body[..end + 2]
}
