//! GUI build id (alpha.7 G3): a SHA-256 over the inputs compiled into
//! `stat-tracker-gui`. A release keeps the id when it changes only the
//! daemon's `main.rs`, the version string, the changelog or other files
//! outside these inputs. A change to stat-tracker lib code counts even when
//! only the daemon uses it, because the GUI compiles the whole lib; most
//! releases change some lib code.
//!
//! `build.rs` includes this file with `#[path]` and embeds the result as
//! `SST_GUI_BUILD_ID`. The lib compiles it only for its unit tests, which
//! hash synthetic trees. Std and `sha2` only.
//!
//! What goes in:
//! - the files listed per workspace crate (sources, `Cargo.toml`, embedded
//!   assets), read in sorted path order with CRLF turned into LF, and with
//!   the `[package]` `version` line dropped from each `Cargo.toml`;
//! - the locked name, version, source and checksum of every registry or git
//!   package reachable from the GUI in `Cargo.lock` (workspace path
//!   packages count by name only, so their version bumps do not count);
//! - the `[profile.*]` tables of the workspace `Cargo.toml`, and its
//!   `[workspace.dependencies]` lines for the packages the GUI reaches.
//!
//! What stays out: `SST_RELEASE_VERSION`, file timestamps, and any file not
//! listed (the changelog, docs, tests outside `src`, the daemon's
//! `main.rs`).

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Bump when the hash layout changes.
const LAYOUT: &str = "sst-gui-build-id/1";

/// Text files get CRLF turned into LF before hashing.
const TEXT_EXTENSIONS: &[&str] = &["rs", "toml", "md", "sh", "txt", "json", "lock"];

/// The files of one workspace crate that end up in the GUI binary.
pub struct CrateInputs {
    /// Package name as `Cargo.lock` spells it.
    pub package: String,
    pub dir: PathBuf,
    /// Files or directories relative to `dir`, `/`-separated.
    pub include: Vec<String>,
    /// Files or directory prefixes relative to `dir` to leave out.
    pub exclude: Vec<String>,
}

pub struct Inputs {
    /// The GUI package, where the `Cargo.lock` walk starts.
    pub root_package: String,
    pub lockfile: PathBuf,
    pub workspace_manifest: PathBuf,
    /// One entry per workspace crate the GUI links, itself included.
    pub crates: Vec<CrateInputs>,
}

pub struct BuildId {
    /// 64 lowercase hex characters.
    pub id: String,
    /// Every include root that exists, plus the lock file and the workspace
    /// manifest: what `cargo::rerun-if-changed` should watch.
    pub watched: Vec<PathBuf>,
}

/// The real inputs, from the GUI crate's `CARGO_MANIFEST_DIR`. Every
/// workspace crate the GUI links must be listed here: [`compute`] refuses a
/// lock walk that reaches an unlisted one.
///
/// The stat-tracker lib leaves out `src/main.rs`, its test modules
/// (`src/main_tests/`, `src/ocr_bench.rs`) and `src/bin`: those are the
/// daemon binary, which the GUI does not link. Its embedded assets
/// (portraits, digit and letter templates) and `dist/install-paths.sh` (which
/// the GUI embeds for uninstall) are in. Its changelog is out: every release
/// edits it, and a GUI that keeps running only misses the newest entry in
/// What's new until its next start.
///
/// The crates split out of the stat-tracker lib (0.6 N4: today
/// `stat-tracker-core`) are listed the same way: `Cargo.toml` and `src`,
/// with the same [`DAEMON_ONLY`] exclusions.
pub fn workspace_inputs(gui_manifest_dir: &Path) -> Inputs {
    let root = gui_manifest_dir.join("..").join("..");
    let crates = root.join("crates");
    let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Inputs {
        root_package: "scuffed-stat-tracker-ui".into(),
        lockfile: root.join("Cargo.lock"),
        workspace_manifest: root.join("Cargo.toml"),
        crates: vec![
            CrateInputs {
                package: "scuffed-stat-tracker-ui".into(),
                dir: gui_manifest_dir.to_path_buf(),
                include: strings(&["Cargo.toml", "build.rs", "src", "fonts"]),
                exclude: Vec::new(),
            },
            CrateInputs {
                package: "scuffed-stat-tracker".into(),
                dir: crates.join("stat-tracker"),
                include: strings(&[
                    "Cargo.toml",
                    "build.rs",
                    "src",
                    "portraits",
                    "assets/shadow-digits",
                    "assets/blank-letters",
                    "dist/install-paths.sh",
                ]),
                exclude: strings(DAEMON_ONLY),
            },
            CrateInputs {
                package: "scuffed-stat-tracker-core".into(),
                dir: crates.join("stat-tracker-core"),
                include: strings(&["Cargo.toml", "build.rs", "src"]),
                exclude: strings(DAEMON_ONLY),
            },
            CrateInputs {
                package: "scuffed-types".into(),
                dir: crates.join("types"),
                include: strings(&["Cargo.toml", "src"]),
                exclude: Vec::new(),
            },
        ],
    }
}

/// Paths the stat-tracker crates leave out of the build id: the daemon
/// binary (`src/main.rs` and its modules under `src/daemon/`), its test
/// modules and any other binary. The GUI does not link them.
pub const DAEMON_ONLY: &[&str] = &[
    "src/main.rs",
    "src/daemon",
    "src/main_tests",
    "src/ocr_bench.rs",
    "src/bin",
];

pub fn compute(inputs: &Inputs) -> Result<BuildId, String> {
    let lock = read_text(&inputs.lockfile)?;
    let packages = parse_lock(&lock);
    let reached = reachable(&packages, &inputs.root_package)?;

    let mut lock_lines = BTreeSet::new();
    let mut names = BTreeSet::new();
    for &index in &reached {
        let pkg = &packages[index];
        names.insert(pkg.name.clone());
        match &pkg.source {
            Some(source) => {
                lock_lines.insert(format!(
                    "{} {} {} {}",
                    pkg.name,
                    pkg.version,
                    source,
                    pkg.checksum.as_deref().unwrap_or("-")
                ));
            }
            None => {
                if !inputs.crates.iter().any(|c| c.package == pkg.name) {
                    return Err(format!(
                        "workspace crate {} is linked by {} but has no build-id inputs; list its files in build.rs",
                        pkg.name, inputs.root_package
                    ));
                }
                lock_lines.insert(format!("{} path", pkg.name));
            }
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(LAYOUT.as_bytes());
    hasher.update(b"\n");
    for line in &lock_lines {
        hasher.update(b"lock\0");
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }

    let manifest = read_text(&inputs.workspace_manifest)?;
    hasher.update(b"workspace\0");
    hasher.update(workspace_manifest_part(&manifest, &names).as_bytes());

    let mut watched = vec![inputs.lockfile.clone(), inputs.workspace_manifest.clone()];
    let mut crates: Vec<&CrateInputs> = inputs.crates.iter().collect();
    crates.sort_by(|a, b| a.package.cmp(&b.package));
    for krate in crates {
        let mut files = Vec::new();
        for include in &krate.include {
            let path = krate.dir.join(include);
            if !path.exists() {
                continue;
            }
            watched.push(path.clone());
            collect_files(&krate.dir, &path, &krate.exclude, &mut files)?;
        }
        files.sort();
        files.dedup();
        for rel in files {
            let bytes = std::fs::read(krate.dir.join(&rel))
                .map_err(|e| format!("could not read {}/{rel}: {e}", krate.dir.display()))?;
            let bytes = normalize(&rel, bytes);
            hasher.update(b"file\0");
            hasher.update(krate.package.as_bytes());
            hasher.update(b"/");
            hasher.update(rel.as_bytes());
            hasher.update(b"\0");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
    }

    let id = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(BuildId { id, watched })
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))
}

/// `/`-separated path of `path` under `base`.
fn relative(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

fn excluded(rel: &str, exclude: &[String]) -> bool {
    exclude.iter().any(|ex| {
        rel == ex
            || rel
                .strip_prefix(ex.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Files under `path` (or `path` itself), relative to `base`. Dot files
/// (editor swap files, `.gitignore`) are skipped.
fn collect_files(
    base: &Path,
    path: &Path,
    exclude: &[String],
    out: &mut Vec<String>,
) -> Result<(), String> {
    let Some(rel) = relative(base, path) else {
        return Ok(());
    };
    if excluded(&rel, exclude) {
        return Ok(());
    }
    if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        return Ok(());
    }
    if path.is_dir() {
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("could not list {}: {e}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("could not list {}: {e}", path.display()))?;
            collect_files(base, &entry.path(), exclude, out)?;
        }
    } else if path.is_file() {
        out.push(rel);
    }
    Ok(())
}

fn normalize(rel: &str, bytes: Vec<u8>) -> Vec<u8> {
    let ext = rel.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    if !TEXT_EXTENSIONS.contains(&ext) {
        return bytes;
    }
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    let is_manifest = rel == "Cargo.toml" || rel.ends_with("/Cargo.toml");
    if is_manifest {
        strip_package_version(&text).into_bytes()
    } else {
        text.into_bytes()
    }
}

/// Drop `version = ...` (or `version.workspace = true`) from `[package]`.
/// The stat-tracker crate's version is the release version.
pub fn strip_package_version(manifest: &str) -> String {
    let mut out = String::with_capacity(manifest.len());
    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
        } else if in_package && is_version_key(trimmed) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn is_version_key(line: &str) -> bool {
    line.strip_prefix("version").is_some_and(|rest| {
        let rest = rest.trim_start();
        rest.starts_with('=') || rest.starts_with('.')
    })
}

/// `[profile.*]` tables plus the `[workspace.dependencies]` lines for
/// `names`. Blank lines and comments are left out.
fn workspace_manifest_part(manifest: &str, names: &BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut section = String::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            section = trimmed.trim_matches(|c| c == '[' || c == ']').to_string();
            if section == "profile" || section.starts_with("profile.") {
                out.push_str(trimmed);
                out.push('\n');
            }
            continue;
        }
        let keep = if section == "profile" || section.starts_with("profile.") {
            true
        } else if section == "workspace.dependencies" {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            names.contains(key.trim_matches('"'))
        } else {
            false
        };
        if keep {
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out
}

#[derive(Debug, Default)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    dependencies: Vec<String>,
}

fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_matches('"')
        .to_string()
}

/// The `[[package]]` entries of a `Cargo.lock` (format v3 and v4).
fn parse_lock(text: &str) -> Vec<LockPackage> {
    let mut packages: Vec<LockPackage> = Vec::new();
    let mut in_package = false;
    let mut in_deps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if in_deps {
            if trimmed.starts_with(']') {
                in_deps = false;
            } else if let Some(pkg) = packages.last_mut()
                && !trimmed.is_empty()
            {
                pkg.dependencies.push(unquote(trimmed));
            }
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = trimmed == "[[package]]";
            if in_package {
                packages.push(LockPackage::default());
            }
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(pkg) = packages.last_mut() else {
            continue;
        };
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "name" => pkg.name = unquote(value),
            "version" => pkg.version = unquote(value),
            "source" => pkg.source = Some(unquote(value)),
            "checksum" => pkg.checksum = Some(unquote(value)),
            "dependencies" => {
                let inner = value.trim_start_matches('[');
                if let Some(end) = inner.find(']') {
                    pkg.dependencies.extend(
                        inner[..end]
                            .split(',')
                            .map(unquote)
                            .filter(|d| !d.is_empty()),
                    );
                } else {
                    in_deps = true;
                }
            }
            _ => {}
        }
    }
    packages
}

/// A dependency entry is `name`, `name version` or `name version (source)`.
fn resolve(packages: &[LockPackage], dep: &str) -> Option<usize> {
    let mut parts = dep.splitn(3, ' ');
    let name = parts.next()?;
    let version = parts.next();
    let source = parts
        .next()
        .map(|s| s.trim_start_matches('(').trim_end_matches(')'));
    packages.iter().position(|p| {
        p.name == name
            && version.is_none_or(|v| p.version == v)
            && source.is_none_or(|s| p.source.as_deref() == Some(s))
    })
}

/// Indexes of every package reachable from `root`, `root` included.
fn reachable(packages: &[LockPackage], root: &str) -> Result<BTreeSet<usize>, String> {
    let start = packages
        .iter()
        .position(|p| p.name == root && p.source.is_none())
        .ok_or_else(|| format!("{root} is not in Cargo.lock"))?;
    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(index) = queue.pop_front() {
        for dep in &packages[index].dependencies {
            let next = resolve(packages, dep).ok_or_else(|| {
                format!(
                    "Cargo.lock: {} depends on {dep:?}, which has no entry",
                    packages[index].name
                )
            })?;
            if seen.insert(next) {
                queue.push_back(next);
            }
        }
    }
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    const LOCK: &str = r#"# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "gui"
version = "0.1.0"
dependencies = [
 "lib",
 "serde 1.0.0",
]

[[package]]
name = "lib"
version = "0.5.0-alpha.6"
dependencies = [
 "serde 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)",
]

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaa"

[[package]]
name = "serde"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bbbb"

[[package]]
name = "server-only"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "cccc"
"#;

    const WORKSPACE: &str = r#"[workspace]
members = ["crates/gui", "crates/lib"]

[profile.release]
lto = "thin"

[workspace.dependencies]
serde = "1"
axum = "0.8"
"#;

    struct Tree {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    impl Tree {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let root = dir.path().to_path_buf();
            let tree = Tree { _dir: dir, root };
            tree.write("Cargo.lock", LOCK);
            tree.write("Cargo.toml", WORKSPACE);
            tree.write(
                "crates/gui/Cargo.toml",
                "[package]\nname = \"gui\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib = { path = \"../lib\" }\n",
            );
            tree.write("crates/gui/src/main.rs", "fn main() { app::run(); }\n");
            tree.write(
                "crates/gui/src/app.rs",
                "pub fn run() { println!(\"hi\"); }\n",
            );
            tree.write("crates/gui/fonts/a.ttf", "\x00\x01font");
            tree.write(
                "crates/lib/Cargo.toml",
                "[package]\nname = \"lib\"\nversion = \"0.5.0-alpha.6\"\n\n[dependencies]\nserde = \"1\"\n",
            );
            tree.write("crates/lib/src/lib.rs", "pub fn shared() -> u32 { 1 }\n");
            tree.write("crates/lib/src/main.rs", "fn main() { daemon(); }\n");
            tree.write("crates/lib/src/daemon/mod.rs", "mod poll;\n");
            tree.write("crates/lib/src/daemon/poll/mod.rs", "fn tick() {}\n");
            tree.write("crates/lib/src/bin/tool.rs", "fn main() {}\n");
            tree.write("crates/lib/src/main_tests/mod.rs", "mod area;\n");
            tree.write("crates/lib/src/main_tests/area.rs", "#[test]\nfn t() {}\n");
            tree.write("crates/lib/assets/d.png", "PNG digits");
            tree.write("crates/lib/CHANGELOG.md", "## 0.5.0-alpha.6\n");
            tree
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }

        fn write(&self, rel: &str, body: &str) {
            let path = self.path(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }

        fn edit(&self, rel: &str, from: &str, to: &str) {
            let body = fs::read_to_string(self.path(rel)).unwrap();
            assert!(body.contains(from), "{rel} has no {from:?}");
            self.write(rel, &body.replacen(from, to, 1));
        }

        fn inputs(&self) -> Inputs {
            let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            Inputs {
                root_package: "gui".into(),
                lockfile: self.path("Cargo.lock"),
                workspace_manifest: self.path("Cargo.toml"),
                crates: vec![
                    CrateInputs {
                        package: "gui".into(),
                        dir: self.path("crates/gui"),
                        include: strings(&["Cargo.toml", "build.rs", "src", "fonts"]),
                        exclude: Vec::new(),
                    },
                    CrateInputs {
                        package: "lib".into(),
                        dir: self.path("crates/lib"),
                        include: strings(&["Cargo.toml", "src", "assets"]),
                        exclude: strings(DAEMON_ONLY),
                    },
                ],
            }
        }

        fn id(&self) -> String {
            compute(&self.inputs()).expect("build id").id
        }
    }

    #[test]
    fn build_id_is_64_hex_and_repeatable() {
        let tree = Tree::new();
        let id = tree.id();
        assert_eq!(id.len(), 64);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_eq!(id, tree.id());
    }

    #[test]
    fn build_id_watches_existing_inputs_only() {
        let tree = Tree::new();
        let watched = compute(&tree.inputs()).expect("build id").watched;
        assert!(watched.contains(&tree.path("Cargo.lock")));
        assert!(watched.contains(&tree.path("Cargo.toml")));
        assert!(watched.contains(&tree.path("crates/gui/src")));
        assert!(watched.contains(&tree.path("crates/lib/assets")));
        // A missing path would make cargo rerun build.rs on every build.
        assert!(!watched.contains(&tree.path("crates/gui/build.rs")));
    }

    #[test]
    fn build_id_ignores_a_version_only_release() {
        let tree = Tree::new();
        let before = tree.id();
        tree.edit(
            "crates/lib/Cargo.toml",
            "version = \"0.5.0-alpha.6\"",
            "version = \"0.5.0-alpha.7\"",
        );
        tree.edit(
            "Cargo.lock",
            "name = \"lib\"\nversion = \"0.5.0-alpha.6\"",
            "name = \"lib\"\nversion = \"0.5.0-alpha.7\"",
        );
        tree.write(
            "crates/lib/CHANGELOG.md",
            "## 0.5.0-alpha.7\n\n- daemon fix\n",
        );
        assert_eq!(tree.id(), before, "version bump and changelog");
        tree.edit("crates/lib/src/main.rs", "daemon();", "daemon(); retry();");
        tree.write("crates/lib/src/bin/tool.rs", "fn main() { other(); }\n");
        tree.edit("crates/lib/src/main_tests/area.rs", "fn t()", "fn t2()");
        tree.edit(
            "crates/lib/src/daemon/poll/mod.rs",
            "fn tick()",
            "fn tick2()",
        );
        tree.write("crates/lib/src/daemon/new_area.rs", "fn n() {}\n");
        tree.write(
            "crates/lib/src/main_tests/new_area.rs",
            "#[test]\nfn n() {}\n",
        );
        assert_eq!(tree.id(), before, "daemon-only files");
    }

    #[test]
    fn build_id_ignores_timestamps() {
        let tree = Tree::new();
        let before = tree.id();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        for rel in [
            "crates/gui/src/app.rs",
            "crates/gui/Cargo.toml",
            "crates/lib/src/lib.rs",
            "Cargo.lock",
        ] {
            fs::File::options()
                .write(true)
                .open(tree.path(rel))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        assert_eq!(tree.id(), before);
    }

    #[test]
    fn build_id_changes_with_one_gui_source_byte() {
        let tree = Tree::new();
        let before = tree.id();
        tree.edit("crates/gui/src/app.rs", "hi", "ho");
        let gui_edit = tree.id();
        assert_ne!(gui_edit, before, "GUI source");
        tree.edit("crates/lib/src/lib.rs", "1 }", "2 }");
        let lib_edit = tree.id();
        assert_ne!(lib_edit, gui_edit, "linked lib source");
        tree.write("crates/lib/assets/d.png", "PNG digitz");
        assert_ne!(tree.id(), lib_edit, "embedded asset");
    }

    #[test]
    fn build_id_counts_a_new_gui_file() {
        let tree = Tree::new();
        let before = tree.id();
        tree.write("crates/gui/src/extra.rs", "");
        assert_ne!(tree.id(), before);
    }

    #[test]
    fn build_id_treats_crlf_like_lf() {
        let tree = Tree::new();
        let before = tree.id();
        let body = fs::read_to_string(tree.path("crates/gui/src/app.rs")).unwrap();
        tree.write("crates/gui/src/app.rs", &body.replace('\n', "\r\n"));
        let manifest = fs::read_to_string(tree.path("crates/lib/Cargo.toml")).unwrap();
        tree.write("crates/lib/Cargo.toml", &manifest.replace('\n', "\r\n"));
        assert_eq!(tree.id(), before);
    }

    #[test]
    fn build_id_counts_reached_locked_dependencies_only() {
        let tree = Tree::new();
        let before = tree.id();
        tree.edit("Cargo.lock", "checksum = \"cccc\"", "checksum = \"dddd\"");
        tree.edit(
            "Cargo.lock",
            "name = \"serde\"\nversion = \"2.0.0\"",
            "name = \"serde\"\nversion = \"2.0.1\"",
        );
        assert_eq!(tree.id(), before, "packages the GUI does not reach");
        tree.edit("Cargo.lock", "checksum = \"aaaa\"", "checksum = \"abab\"");
        assert_ne!(tree.id(), before, "a package the GUI links");
    }

    #[test]
    fn build_id_counts_profiles_and_used_workspace_dependencies() {
        let tree = Tree::new();
        let before = tree.id();
        tree.edit("Cargo.toml", "axum = \"0.8\"", "axum = \"0.9\"");
        tree.edit(
            "Cargo.toml",
            "members = [\"crates/gui\", \"crates/lib\"]",
            "members = [\"crates/gui\", \"crates/lib\", \"crates/server\"]",
        );
        assert_eq!(tree.id(), before, "server-only workspace edits");
        tree.edit("Cargo.toml", "lto = \"thin\"", "lto = \"fat\"");
        let profile = tree.id();
        assert_ne!(profile, before, "release profile");
        tree.edit(
            "Cargo.toml",
            "serde = \"1\"",
            "serde = { version = \"1\", features = [\"derive\"] }",
        );
        assert_ne!(tree.id(), profile, "a workspace dependency the GUI uses");
    }

    #[test]
    fn build_id_refuses_an_unlisted_workspace_crate() {
        let tree = Tree::new();
        tree.edit("Cargo.lock", " \"lib\",\n", " \"lib\",\n \"extra\",\n");
        let lock = fs::read_to_string(tree.path("Cargo.lock")).unwrap();
        tree.write(
            "Cargo.lock",
            &format!("{lock}\n[[package]]\nname = \"extra\"\nversion = \"0.1.0\"\n"),
        );
        let err = compute(&tree.inputs()).err().expect("unlisted crate");
        assert!(err.contains("extra"), "{err}");
    }

    #[test]
    fn strip_package_version_keeps_dependency_versions() {
        let manifest = "[package]\nname = \"x\"\nversion = \"1.2.3\"\n\n[dependencies]\nfoo = { version = \"1\" }\n\n[dependencies.bar]\nversion = \"2\"\n";
        let stripped = strip_package_version(manifest);
        assert!(!stripped.contains("1.2.3"));
        assert!(stripped.contains("foo = { version = \"1\" }"));
        assert!(stripped.contains("version = \"2\""));
    }

    /// The daemon binary and its tests are out of the GUI build id: a release
    /// that only touches them must not ask a running GUI to restart (G3).
    /// The same rules hold for every stat-tracker lib crate.
    #[test]
    fn workspace_inputs_leave_out_the_daemon_binary_and_its_tests() {
        let inputs = workspace_inputs(Path::new(env!("CARGO_MANIFEST_DIR")));
        for package in ["scuffed-stat-tracker", "scuffed-stat-tracker-core"] {
            let tracker = inputs
                .crates
                .iter()
                .find(|c| c.package == package)
                .unwrap_or_else(|| panic!("{package} is listed"));
            for rel in [
                "src/main.rs",
                "src/main_tests/mod.rs",
                "src/main_tests/poll_cadence.rs",
                "src/ocr_bench.rs",
                "src/bin/tool.rs",
            ] {
                assert!(excluded(rel, &tracker.exclude), "{package}: {rel}");
            }
            for rel in [
                "src/lib.rs",
                "src/main_tests_helper.rs",
                "src/ocr/mod.rs",
                "src/config.rs",
            ] {
                assert!(!excluded(rel, &tracker.exclude), "{package}: {rel}");
            }
            assert!(tracker.include.iter().any(|i| i == "src"), "{package}");
            assert!(
                tracker.include.iter().any(|i| i == "Cargo.toml"),
                "{package}"
            );
        }
    }

    /// Every `crates/stat-tracker*` crate but the GUI is a lib crate the GUI
    /// links (the 0.6 N4 split adds them one by one): each one is listed,
    /// by its package name, with its `src` hashed. A crate the GUI does not
    /// reach would be listed for nothing, so this also catches a crate the
    /// split forgot to wire up.
    #[test]
    fn workspace_inputs_list_every_stat_tracker_lib_crate() {
        let gui = Path::new(env!("CARGO_MANIFEST_DIR"));
        let inputs = workspace_inputs(gui);
        let crates_dir = gui.parent().expect("crates/");
        let mut found = Vec::new();
        for entry in fs::read_dir(crates_dir).expect("read crates/") {
            let dir = entry.expect("dir entry").path();
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with("stat-tracker") || name == "stat-tracker-ui" {
                continue;
            }
            let manifest = fs::read_to_string(dir.join("Cargo.toml")).expect("Cargo.toml");
            let package = manifest
                .lines()
                .find_map(|l| {
                    let rest = l.trim().strip_prefix("name")?.trim_start();
                    Some(rest.strip_prefix('=')?.trim().trim_matches('"').to_string())
                })
                .expect("[package] name");
            let listed = inputs
                .crates
                .iter()
                .find(|c| c.package == package)
                .unwrap_or_else(|| panic!("crates/{name} ({package}) has no build-id inputs"));
            assert_eq!(listed.dir.file_name(), dir.file_name(), "{package}");
            found.push(package);
        }
        found.sort();
        assert_eq!(
            found,
            ["scuffed-stat-tracker", "scuffed-stat-tracker-core"],
            "the walk found the tracker crates"
        );
        let watched = compute(&inputs).expect("workspace build id").watched;
        assert!(
            watched.iter().any(|p| p.ends_with("stat-tracker-core/src")),
            "{watched:?}"
        );
    }

    /// A lower lib crate, listed with the same exclusions: its sources count,
    /// its daemon-only files and its version do not.
    #[test]
    fn build_id_counts_a_listed_lower_lib_crate() {
        let tree = Tree::new();
        tree.edit(
            "Cargo.lock",
            " \"serde 1.0.0 (registry",
            " \"core\",\n \"serde 1.0.0 (registry",
        );
        let lock = fs::read_to_string(tree.path("Cargo.lock")).unwrap();
        tree.write(
            "Cargo.lock",
            &format!("{lock}\n[[package]]\nname = \"core\"\nversion = \"0.5.0-alpha.6\"\n"),
        );
        tree.write(
            "crates/core/Cargo.toml",
            "[package]\nname = \"core\"\nversion = \"0.5.0-alpha.6\"\n",
        );
        tree.write("crates/core/src/lib.rs", "pub fn base() -> u32 { 1 }\n");
        tree.write("crates/core/src/bin/tool.rs", "fn main() {}\n");
        let unlisted = compute(&tree.inputs()).err().expect("core is unlisted");
        assert!(unlisted.contains("core"), "{unlisted}");
        let inputs = || {
            let mut inputs = tree.inputs();
            inputs.crates.push(CrateInputs {
                package: "core".into(),
                dir: tree.path("crates/core"),
                include: vec!["Cargo.toml".into(), "src".into()],
                exclude: DAEMON_ONLY.iter().map(|s| s.to_string()).collect(),
            });
            compute(&inputs).expect("build id").id
        };
        let before = inputs();
        tree.edit(
            "crates/core/Cargo.toml",
            "version = \"0.5.0-alpha.6\"",
            "version = \"0.5.0-alpha.7\"",
        );
        tree.edit(
            "Cargo.lock",
            "name = \"core\"\nversion = \"0.5.0-alpha.6\"",
            "name = \"core\"\nversion = \"0.5.0-alpha.7\"",
        );
        tree.write("crates/core/src/bin/tool.rs", "fn main() { other(); }\n");
        assert_eq!(inputs(), before, "version bump and a core binary");
        tree.edit("crates/core/src/lib.rs", "1 }", "2 }");
        assert_ne!(inputs(), before, "core lib source");
    }

    #[test]
    fn embedded_build_id_matches_the_workspace_inputs() {
        let inputs = workspace_inputs(Path::new(env!("CARGO_MANIFEST_DIR")));
        let computed = compute(&inputs).expect("workspace build id");
        assert_eq!(computed.id, env!("SST_GUI_BUILD_ID"));
    }
}
