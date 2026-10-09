//! Uninstall the tracker from this computer.
//!
//! The path list is `crates/stat-tracker/dist/install-paths.sh`. The installer
//! writes a manifest of the files it installed. This module removes exactly
//! those entries, and only when each one resolves under the home folder or
//! the install folder. An older install with no manifest falls back to that
//! list and shows the files that are still there.
//!
//! A system package is whoever owns the running program. The folder is not
//! consulted. An AppImage is removed like any other copy, including the
//! AppImage file. The dialog shows `sudo pacman -R <pkg>` or
//! `sudo apt remove <pkg>` when a package owns the program.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use iced::widget::{button, checkbox, column, container, opaque, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length, Padding};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_FEATURED,
    SIZE_LABEL, SIZE_META, SIZE_TITLE, TEXT, TEXT_2, TEXT_3,
};

/// Shell source of the install path list. Parsed, not executed.
pub const INSTALL_PATHS_SH: &str = include_str!("../../stat-tracker/dist/install-paths.sh");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    Bin,
    LibDir,
    Desktop,
    Unit,
    DropIn,
    Autostart,
    Helper,
    List,
    Manifest,
    Data,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Always,
    Purge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub class: PathClass,
    pub when: When,
    pub template: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Pacman,
    Apt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageOwner {
    pub manager: PackageManager,
    pub package: String,
}

impl PackageOwner {
    pub fn remove_command(&self) -> String {
        match self.manager {
            PackageManager::Pacman => format!("sudo pacman -R {}", self.package),
            PackageManager::Apt => format!("sudo apt remove {}", self.package),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOrigin {
    /// User install folder, under the home folder.
    Bootstrap { prefix: PathBuf },
    /// Owned by a system package. `command` is what the user should run.
    Package { command: String },
    /// Not a user install. Nothing is deleted.
    Outside { exe: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallDialog {
    Confirm {
        home: PathBuf,
        prefix: PathBuf,
        /// Unchecked until the user opts in. Local data stays.
        delete_data: bool,
        /// Saved games folder the app is using.
        data_dir: PathBuf,
        /// The .AppImage file, when this copy is one.
        appimage: Option<PathBuf>,
        /// No install record. The dialog lists the original files that exist.
        fallback: bool,
    },
    /// Package manager, or a copy this app must not delete.
    Manual {
        command: Option<String>,
        detail: String,
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallRequest {
    pub home: PathBuf,
    pub prefix: PathBuf,
    /// Folder the running app is using for saved games. Not the fixed default
    /// when Settings points somewhere else.
    pub data_dir: PathBuf,
    pub appimage: Option<PathBuf>,
    pub delete_data: bool,
    pub origin: InstallOrigin,
    pub systemctl: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    /// True when this copy is not a user install. No path was touched.
    pub skipped: bool,
    pub removed_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewGroup {
    pub heading: &'static str,
    pub note: &'static str,
    pub paths: Vec<PathBuf>,
}

pub fn parse_specs(src: &str) -> Vec<Spec> {
    let mut specs = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(class) = parts.next().and_then(parse_class) else {
            continue;
        };
        let Some(when) = parts.next().and_then(parse_when) else {
            continue;
        };
        let Some(template) = parts.next() else {
            continue;
        };
        if parts.next().is_some() {
            continue;
        }
        if !template.contains("{prefix}") && !template.contains("{home}") {
            continue;
        }
        specs.push(Spec {
            class,
            when,
            template: template.to_string(),
        });
    }
    specs
}

fn parse_class(raw: &str) -> Option<PathClass> {
    Some(match raw {
        "bin" => PathClass::Bin,
        "libdir" => PathClass::LibDir,
        "desktop" => PathClass::Desktop,
        "unit" => PathClass::Unit,
        "dropin" => PathClass::DropIn,
        "autostart" => PathClass::Autostart,
        "helper" => PathClass::Helper,
        "list" => PathClass::List,
        "manifest" => PathClass::Manifest,
        "data" => PathClass::Data,
        "config" => PathClass::Config,
        _ => return None,
    })
}

fn parse_when(raw: &str) -> Option<When> {
    match raw {
        "always" => Some(When::Always),
        "purge" => Some(When::Purge),
        _ => None,
    }
}

pub fn expand_template(template: &str, home: &Path, prefix: &Path) -> PathBuf {
    PathBuf::from(
        template
            .replace("{home}", &home.to_string_lossy())
            .replace("{prefix}", &prefix.to_string_lossy()),
    )
}

pub fn expanded_specs(home: &Path, prefix: &Path) -> Vec<(Spec, PathBuf)> {
    parse_specs(INSTALL_PATHS_SH)
        .into_iter()
        .map(|spec| {
            let path = expand_template(&spec.template, home, prefix);
            (spec, path)
        })
        .collect()
}

pub fn valid_package_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-'))
}

pub fn parse_pacman_qo(text: &str) -> Option<String> {
    let line = text.lines().find(|line| line.contains(" is owned by "))?;
    let rest = line.split(" is owned by ").nth(1)?;
    let pkg = rest.split_whitespace().next()?;
    valid_package_name(pkg).then(|| pkg.to_string())
}

pub fn parse_dpkg_s(text: &str) -> Option<String> {
    let line = text.lines().find(|line| line.contains(':'))?;
    let pkg = line.split(':').next()?.trim();
    valid_package_name(pkg).then(|| pkg.to_string())
}

pub fn probe_package_owner(path: &Path) -> Option<PackageOwner> {
    probe_package_owner_with(path, Path::new("pacman"), Path::new("dpkg"))
}

/// `pacman` and `dpkg` may be absolute paths. The app calls [`probe_package_owner`].
pub fn probe_package_owner_with(path: &Path, pacman: &Path, dpkg: &Path) -> Option<PackageOwner> {
    let target = path.display().to_string();
    if let Some(text) = command_stdout(pacman, &["-Qo".to_string(), target.clone()])
        && let Some(package) = parse_pacman_qo(&text)
    {
        return Some(PackageOwner {
            manager: PackageManager::Pacman,
            package,
        });
    }
    if let Some(text) = command_stdout(dpkg, &["-S".to_string(), target])
        && let Some(package) = parse_dpkg_s(&text)
    {
        return Some(PackageOwner {
            manager: PackageManager::Apt,
            package,
        });
    }
    None
}

fn command_stdout(cmd: &Path, args: &[String]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// AppImage runtime mounts under `/tmp/.mount_*`, or the user launches the
/// `.AppImage` file itself. Either one is a script install.
pub fn is_appimage(exe: &Path) -> bool {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    if name.ends_with(".AppImage") || name.ends_with(".appimage") {
        return true;
    }
    exe.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .starts_with(".mount_")
    })
}

/// Unit names from `systemd_units_to_disable` in install-paths.sh.
pub fn units_to_disable() -> Vec<String> {
    let Some(start) = INSTALL_PATHS_SH.find("systemd_units_to_disable()") else {
        return Vec::new();
    };
    let rest = &INSTALL_PATHS_SH[start..];
    let Some(marker) = rest.find("<<'EOF'") else {
        return Vec::new();
    };
    let body = &rest[marker + "<<'EOF'".len()..];
    let Some(end) = body.find("\nEOF") else {
        return Vec::new();
    };
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

pub fn prefix_from_gui(exe: &Path) -> Option<PathBuf> {
    if exe.file_name()? != "stat-tracker-gui" {
        return None;
    }
    let bin = exe.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    Some(bin.parent()?.to_path_buf())
}

pub fn decide(exe: &Path, home: &Path, owner: Option<PackageOwner>) -> InstallOrigin {
    // The package manager's answer about this binary wins.
    if let Some(owner) = owner {
        return InstallOrigin::Package {
            command: owner.remove_command(),
        };
    }
    if is_appimage(exe) {
        return InstallOrigin::Bootstrap {
            prefix: home.join(".local"),
        };
    }
    if let Some(prefix) = prefix_from_gui(exe) {
        // /usr and any other prefix outside the home folder is not removed.
        if prefix_within_home(&prefix, home) {
            return InstallOrigin::Bootstrap { prefix };
        }
    }
    InstallOrigin::Outside {
        exe: exe.to_path_buf(),
    }
}

fn prefix_within_home(prefix: &Path, home: &Path) -> bool {
    if contains_dotdot(prefix) || contains_dotdot(home) {
        return false;
    }
    let prefix = lexical(prefix);
    let home = lexical(home);
    prefix.starts_with(&home)
}

fn spec_path(home: &Path, prefix: &Path, class: PathClass) -> PathBuf {
    expanded_specs(home, prefix)
        .into_iter()
        .find(|(spec, _)| spec.class == class)
        .map(|(_, path)| path)
        .unwrap_or_default()
}

pub fn open_dialog(
    exe: &Path,
    home: &Path,
    owner: Option<PackageOwner>,
    data_dir: &Path,
) -> UninstallDialog {
    let appimage = appimage_file(exe);
    match decide(exe, home, owner) {
        InstallOrigin::Bootstrap { prefix } => {
            let plan = removal_plan(home, &prefix, appimage.as_deref());
            UninstallDialog::Confirm {
                home: home.to_path_buf(),
                prefix,
                data_dir: data_dir.to_path_buf(),
                appimage: appimage.clone(),
                delete_data: false,
                fallback: plan.fallback,
            }
        }
        InstallOrigin::Package { command } => UninstallDialog::Manual {
            detail:
                "This copy was installed by your system package manager. Nothing will be removed."
                    .into(),
            command: Some(command),
            paths: Vec::new(),
        },
        InstallOrigin::Outside { exe } => {
            let outside_prefix =
                prefix_from_gui(&exe).filter(|prefix| !prefix_within_home(prefix, home));
            let mut paths = outside_prefix
                .as_ref()
                .map(|prefix| display_paths(home, prefix))
                .unwrap_or_default();
            if outside_prefix.is_some() && path_exists(&exe) && !paths.contains(&exe) {
                paths.push(exe.clone());
            }
            let detail = if outside_prefix.is_some() {
                "Remove it manually.".into()
            } else {
                "Nothing will be removed.".into()
            };
            UninstallDialog::Manual {
                detail,
                command: None,
                paths,
            }
        }
    }
}

fn appimage_file(exe: &Path) -> Option<PathBuf> {
    let name = exe.file_name()?.to_string_lossy();
    if name.ends_with(".AppImage") || name.ends_with(".appimage") {
        return Some(exe.to_path_buf());
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalPlan {
    /// True when no manifest exists and `files` is the fixed install list.
    pub fallback: bool,
    /// Exact files. Directories and `config.toml` are not included.
    pub files: Vec<PathBuf>,
}

/// Manifest entries when that file exists. Otherwise the fixed install list.
/// Never expands a directory or a glob. `extra` is the AppImage file, if any.
pub fn removal_plan(home: &Path, prefix: &Path, extra: Option<&Path>) -> RemovalPlan {
    let manifest = spec_path(home, prefix, PathClass::Manifest);
    let mut plan = if manifest.is_file() {
        let files = manifest_lines(home, prefix)
            .into_iter()
            .filter(|path| keep_exact_file(path, home, prefix))
            .collect();
        RemovalPlan {
            fallback: false,
            files,
        }
    } else {
        let files = expanded_specs(home, prefix)
            .into_iter()
            .filter(|(spec, path)| {
                spec.when == When::Always
                    && !is_directory_class(spec.class)
                    && keep_exact_file(path, home, prefix)
            })
            .map(|(_, path)| path)
            .collect();
        RemovalPlan {
            fallback: true,
            files,
        }
    };
    if let Some(extra) = extra
        && keep_exact_file(extra, home, prefix)
        && !plan.files.iter().any(|path| path == extra)
    {
        plan.files.push(extra.to_path_buf());
    }
    plan
}

/// Paths to show when this app will not delete anything.
fn display_paths(home: &Path, prefix: &Path) -> Vec<PathBuf> {
    let manifest = spec_path(home, prefix, PathClass::Manifest);
    let candidates = if manifest.is_file() {
        raw_manifest_lines(&manifest)
    } else {
        expanded_specs(home, prefix)
            .into_iter()
            .filter(|(spec, _)| spec.when == When::Always && !is_directory_class(spec.class))
            .map(|(_, path)| path)
            .collect()
    };
    candidates
        .into_iter()
        .filter(|path| !contains_dotdot(path) && path_exists(path))
        .collect()
}

fn keep_exact_file(path: &Path, home: &Path, prefix: &Path) -> bool {
    confined_path(path, home, prefix)
        && !is_config_toml(path)
        && !is_data_or_config_dir(path, home, prefix)
        && !too_broad(path, home)
}

/// Reject `..`, resolve the parent, and require the file to sit under the
/// home folder or the install prefix.
fn confined_path(path: &Path, home: &Path, prefix: &Path) -> bool {
    if !path.is_absolute() || contains_dotdot(path) || path_has_pattern(path) {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(name) = path.file_name() else {
        return false;
    };
    if name.is_empty() || name == ".." || name == "." {
        return false;
    }
    let Ok(canon_parent) = fs::canonicalize(parent) else {
        return false;
    };
    let resolved = canon_parent.join(name);
    let home_root = fs::canonicalize(home).unwrap_or_else(|_| lexical(home));
    let prefix_root = fs::canonicalize(prefix).unwrap_or_else(|_| lexical(prefix));
    strictly_inside(&resolved, &home_root) || strictly_inside(&resolved, &prefix_root)
}

fn contains_dotdot(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.split(['/', '\\']).any(|part| part == "..")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

fn strictly_inside(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root)
        .is_ok_and(|rest| rest.components().next().is_some())
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn path_has_pattern(path: &Path) -> bool {
    path.to_string_lossy()
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '['))
}

fn is_config_toml(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("config.toml")
}

fn is_data_or_config_dir(path: &Path, home: &Path, prefix: &Path) -> bool {
    path == spec_path(home, prefix, PathClass::Data)
        || path == spec_path(home, prefix, PathClass::Config)
}

pub fn preview(
    home: &Path,
    prefix: &Path,
    delete_data: bool,
    data_dir: &Path,
    extra: Option<&Path>,
) -> Vec<PreviewGroup> {
    let plan = removal_plan(home, prefix, extra);
    let files: Vec<PathBuf> = if plan.fallback {
        plan.files
            .iter()
            .filter(|path| path_exists(path))
            .cloned()
            .collect()
    } else {
        plan.files.clone()
    };
    let mut groups = vec![
        PreviewGroup {
            heading: "Files",
            note: if plan.fallback {
                "These are the files from the original install."
            } else {
                "These files will be removed."
            },
            paths: files,
        },
        PreviewGroup {
            heading: "Tracker service",
            note: "The tracker is stopped before anything is removed.",
            paths: vec![],
        },
    ];
    if plan
        .files
        .iter()
        .any(|path| path.ends_with("scuffed-stat-tracker.desktop"))
    {
        groups.push(PreviewGroup {
            heading: "Desktop entry and icon",
            note: "No separate icon file is removed.",
            paths: vec![],
        });
    }
    if delete_data {
        let data_note = if confined_path(data_dir, home, prefix) {
            "Saved games, debug images, and logs."
        } else {
            "This folder is outside your home folder, so it will be left in place."
        };
        groups.push(PreviewGroup {
            heading: "Saved games",
            note: data_note,
            paths: vec![data_dir.to_path_buf()],
        });
        groups.push(PreviewGroup {
            heading: "Settings",
            note: "Settings, including the sync token.",
            paths: vec![spec_path(home, prefix, PathClass::Config)],
        });
    }
    groups
}

fn raw_manifest_lines(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

fn manifest_lines(home: &Path, prefix: &Path) -> Vec<PathBuf> {
    let path = spec_path(home, prefix, PathClass::Manifest);
    raw_manifest_lines(&path)
        .into_iter()
        .filter(|line| confined_path(line, home, prefix))
        .collect()
}

pub fn apply(req: &UninstallRequest) -> Result<UninstallReport, String> {
    let InstallOrigin::Bootstrap { prefix } = &req.origin else {
        return Ok(UninstallReport {
            skipped: true,
            removed_files: 0,
        });
    };
    if prefix != &req.prefix || !prefix_within_home(prefix, &req.home) {
        return Ok(UninstallReport {
            skipped: true,
            removed_files: 0,
        });
    }

    // Stop the service and any timer before any file is unlinked.
    stop_units(&req.systemctl)?;

    let plan = removal_plan(&req.home, &req.prefix, req.appimage.as_deref());
    let mut removed: Vec<PathBuf> = Vec::new();
    let mut left: Vec<PathBuf> = plan.files.clone();
    while !left.is_empty() {
        let path = left[0].clone();
        match remove_exact_file(&path, &req.home) {
            Ok(true) => {
                removed.push(left.remove(0));
            }
            Ok(false) => {
                left.remove(0);
            }
            Err(_) => {
                return Err(partial_message(&removed, &left));
            }
        }
    }
    if let Some(lib_dir) = lib_dir(&req.home, &req.prefix) {
        rmdir_empty_tree(&lib_dir);
    }
    let _ = run_systemctl(&req.systemctl, &["--user", "daemon-reload"]);

    if req.delete_data {
        let config = spec_path(&req.home, &req.prefix, PathClass::Config);
        let mut dirs = Vec::new();
        if confined_path(&req.data_dir, &req.home, &req.prefix) {
            dirs.push(req.data_dir.clone());
        }
        if confined_path(&config, &req.home, &req.prefix) && !dirs.iter().any(|dir| dir == &config)
        {
            dirs.push(config);
        }
        for dir in dirs {
            if remove_listed_tree(&dir, &req.home).is_err() {
                left.push(dir);
                return Err(partial_message(&removed, &left));
            }
            removed.push(dir);
        }
    }

    Ok(UninstallReport {
        skipped: false,
        removed_files: removed.len(),
    })
}

fn partial_message(removed: &[PathBuf], left: &[PathBuf]) -> String {
    let removed_text = plain_list(removed);
    let left_text = plain_list(left);
    if removed.is_empty() {
        format!("Uninstall stopped. Nothing was removed. Still there: {left_text}.")
    } else {
        format!("Uninstall stopped. Removed {removed_text}. Still there: {left_text}.")
    }
}

fn plain_list(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "nothing".into();
    }
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn stop_units(systemctl: &Path) -> Result<(), String> {
    if !systemctl_exists(systemctl) {
        return Err("Could not stop the tracker. Nothing was removed.".into());
    }
    for unit in units_to_disable() {
        let output = run_systemctl(systemctl, &["--user", "disable", "--now", unit.as_str()])?;
        if !output.status.success() && !unit_is_missing(&output) {
            return Err("Could not stop the tracker. Nothing was removed.".into());
        }
        if unit == "scuffed-stat-tracker.service" {
            let active = run_systemctl(systemctl, &["--user", "is-active", unit.as_str()])?;
            let state = String::from_utf8_lossy(&active.stdout);
            if active.status.success() && state.trim() == "active" {
                return Err("The tracker is still running. Nothing was removed.".into());
            }
        }
    }
    Ok(())
}

fn systemctl_exists(path: &Path) -> bool {
    if path.components().count() == 1 {
        return Command::new(path).arg("--version").output().is_ok();
    }
    path.is_file()
}

fn unit_is_missing(output: &std::process::Output) -> bool {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let lower = text.to_ascii_lowercase();
    lower.contains("not found")
        || lower.contains("does not exist")
        || lower.contains("not loaded")
        || lower.contains("no such file")
}

fn run_systemctl(bin: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(bin)
        .args(args)
        .output()
        .map_err(|_| "Could not stop the tracker. Nothing was removed.".to_string())
}

fn lib_dir(home: &Path, prefix: &Path) -> Option<PathBuf> {
    let path = spec_path(home, prefix, PathClass::LibDir);
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

fn is_directory_class(class: PathClass) -> bool {
    matches!(
        class,
        PathClass::LibDir | PathClass::Data | PathClass::Config
    )
}

fn remove_exact_file(path: &Path, home: &Path) -> Result<bool, String> {
    if too_broad(path, home) || is_config_toml(path) {
        return Ok(false);
    }
    if !parents_are_real(path) {
        return Ok(false);
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || meta.is_file() => {
            fs::remove_file(path)
                .map_err(|err| format!("could not remove {}: {err}", path.display()))?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("could not read {}: {err}", path.display())),
    }
}

fn remove_listed_tree(dir: &Path, home: &Path) -> Result<(), String> {
    if too_broad(dir, home) || dir == home {
        return Err(format!("refusing to delete {}", dir.display()));
    }
    if !parents_are_real(dir) {
        return Err(format!(
            "refusing to delete {} because a parent is a symlink",
            dir.display()
        ));
    }
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(format!("refusing to follow symlink {}", dir.display()))
        }
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(dir)
            .map_err(|err| format!("could not delete {}: {err}", dir.display())),
        Ok(_) => {
            fs::remove_file(dir).map_err(|err| format!("could not delete {}: {err}", dir.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("could not read {}: {err}", dir.display())),
    }
}

fn rmdir_empty_tree(dir: &Path) {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                rmdir_empty_tree(&child);
            }
        }
    }
    let _ = fs::remove_dir(dir);
}

fn too_broad(path: &Path, home: &Path) -> bool {
    path == Path::new("/") || path == home || path.as_os_str().is_empty()
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn parents_are_real(path: &Path) -> bool {
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir.as_os_str().is_empty() || dir == Path::new("/") {
            break;
        }
        if let Ok(meta) = fs::symlink_metadata(dir)
            && meta.file_type().is_symlink()
        {
            return false;
        }
        current = dir.parent();
    }
    true
}

pub fn view<'a>(dialog: &'a UninstallDialog, busy: bool) -> Element<'a, Message> {
    let card = match dialog {
        UninstallDialog::Confirm {
            home,
            prefix,
            delete_data,
            data_dir,
            appimage,
            ..
        } => confirm_card(
            home,
            prefix,
            data_dir,
            appimage.as_deref(),
            *delete_data,
            busy,
        ),
        UninstallDialog::Manual {
            command,
            detail,
            paths,
        } => manual_card(detail, command.as_deref(), paths),
    };
    let mut backdrop = theme::BG;
    backdrop.a = 0.88;
    opaque(
        container(card)
            .padding(28)
            .center(Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(backdrop)),
                ..container::Style::default()
            }),
    )
}

fn confirm_card<'a>(
    home: &'a Path,
    prefix: &'a Path,
    data_dir: &'a Path,
    appimage: Option<&'a Path>,
    delete_data: bool,
    busy: bool,
) -> Element<'a, Message> {
    let mut body = column![
        text("Uninstall Scuffed Stat Tracker")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text("These files will be removed. The tracker is stopped first.")
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
    ]
    .spacing(8)
    .width(Fill);

    for group in preview(home, prefix, delete_data, data_dir, appimage) {
        body = body.push(group_block(&group));
    }

    let config = spec_path(home, prefix, PathClass::Config);
    body = body.push(
        checkbox(delete_data)
            .label("Also delete saved games and settings")
            .on_toggle(Message::ToggleUninstallData)
            .size(18)
            .text_size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .style(checkbox_style),
    );
    body = body.push(
        text(format!(
            "{}\nSaved games, debug images, and logs.\n{}\nSettings, including the sync token.",
            data_dir.display(),
            config.display()
        ))
        .size(SIZE_LABEL)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    );

    let cancel = button(
        text("Cancel")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([8, 16]))
    .style(theme::ghost_btn())
    .on_press(Message::DismissUninstall);
    let mut confirm = button(
        text(if busy { "Uninstalling..." } else { "Uninstall" })
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([8, 16]))
    .style(theme::danger_btn(true));
    if !busy {
        confirm = confirm.on_press(Message::ConfirmUninstall);
    }

    body = body.push(row![cancel, confirm].spacing(8).align_y(Alignment::Center));

    dialog_shell(scrollable(body).height(Fill).width(Fill).into())
}

fn manual_card<'a>(
    detail: &'a str,
    command: Option<&'a str>,
    paths: &'a [PathBuf],
) -> Element<'a, Message> {
    let mut body = column![
        text("Uninstall")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(detail).size(SIZE_BODY).font(FONT_MEDIUM).color(TEXT_2),
    ]
    .spacing(10)
    .width(Fill);

    for path in paths {
        body = body.push(
            text(path.display().to_string())
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        );
    }

    if let Some(command) = command {
        body = body.push(
            text(command)
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(theme::ACCENT),
        );
        body = body.push(
            button(text("Copy").size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
                .padding(Padding::from([8, 16]))
                .style(theme::ghost_btn())
                .on_press(Message::CopyUninstallCmd),
        );
    }

    body = body.push(
        button(
            text("Close")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([8, 16]))
        .style(theme::chip(true))
        .on_press(Message::DismissUninstall),
    );
    dialog_shell(body.into())
}

fn group_block(group: &PreviewGroup) -> Element<'static, Message> {
    let heading = group.heading;
    let note = group.note;
    let paths: Vec<String> = group
        .paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let mut col = column![text(heading).size(SIZE_TITLE).font(FONT_BOLD).color(TEXT)]
        .spacing(4)
        .width(Fill);
    if !note.is_empty() {
        col = col.push(text(note).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_3));
    }
    for path in paths {
        col = col.push(text(path).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_2));
    }
    col.into()
}

fn dialog_shell<'a>(body: Element<'a, Message>) -> Element<'a, Message> {
    container(body)
        .padding(24)
        .width(Length::Fixed(640.0))
        .max_height(560.0)
        .style(theme::surface_panel)
        .into()
}

fn checkbox_style(
    _theme: &iced::Theme,
    status: iced::widget::checkbox::Status,
) -> iced::widget::checkbox::Style {
    let selected = matches!(
        status,
        iced::widget::checkbox::Status::Active { is_checked: true }
            | iced::widget::checkbox::Status::Hovered { is_checked: true }
            | iced::widget::checkbox::Status::Disabled { is_checked: true }
    );
    iced::widget::checkbox::Style {
        background: iced::Background::Color(if selected { theme::ACCENT } else { theme::BG }),
        icon_color: TEXT,
        border: iced::Border {
            color: if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
            width: 1.0,
            radius: 4.0.into(),
        },
        text_color: Some(TEXT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn layout(home: &Path, prefix: &Path) {
        let mut recorded = Vec::new();
        for (spec, path) in expanded_specs(home, prefix) {
            if spec.when != When::Always || is_directory_class(spec.class) {
                continue;
            }
            if spec.class == PathClass::Autostart {
                let unit = spec_path(home, prefix, PathClass::Unit);
                write(&unit, "[Unit]\n");
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                std::os::unix::fs::symlink(&unit, &path).unwrap();
            } else {
                write(&path, "installed\n");
            }
            recorded.push(path);
        }
        let lib = spec_path(home, prefix, PathClass::LibDir);
        let bundled = lib.join("ocr/liblept.so.5");
        write(&bundled, "lib\n");
        recorded.push(bundled);
        let manifest = spec_path(home, prefix, PathClass::Manifest);
        recorded.push(manifest.clone());
        recorded.sort();
        recorded.dedup();
        write(
            &manifest,
            &recorded
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        write(
            &spec_path(home, prefix, PathClass::Data).join("stats.surrealkv/db"),
            "games\n",
        );
        write(
            &spec_path(home, prefix, PathClass::Data).join("debug/crop.png"),
            "crop\n",
        );
        write(
            &spec_path(home, prefix, PathClass::Data).join("shadow/digits.jsonl"),
            "shadow\n",
        );
        write(
            &spec_path(home, prefix, PathClass::Config).join("config.toml"),
            "sync_token = \"secret-token\"\n",
        );
    }

    fn request(
        home: &Path,
        prefix: &Path,
        delete_data: bool,
        origin: InstallOrigin,
    ) -> UninstallRequest {
        let systemctl = home.join("systemctl");
        let log = home.join("systemctl.log");
        let unit = spec_path(home, prefix, PathClass::Unit);
        write(
            &systemctl,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$2\" = disable ]; then\n  if [ ! -f '{}' ]; then echo UNIT_MISSING >> '{}'; exit 1; fi\nfi\nexit 0\n",
                log.display(),
                unit.display(),
                log.display()
            ),
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&systemctl).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&systemctl, perms).unwrap();
        }
        UninstallRequest {
            home: home.to_path_buf(),
            prefix: prefix.to_path_buf(),
            data_dir: spec_path(home, prefix, PathClass::Data),
            appimage: None,
            delete_data,
            origin,
            systemctl,
        }
    }

    #[test]
    fn path_list_names_what_bootstrap_installs() {
        let specs = parse_specs(INSTALL_PATHS_SH);
        let classes: Vec<_> = specs.iter().map(|spec| spec.class).collect();
        for class in [
            PathClass::Bin,
            PathClass::LibDir,
            PathClass::Desktop,
            PathClass::Unit,
            PathClass::Autostart,
            PathClass::Data,
            PathClass::Config,
        ] {
            assert!(classes.contains(&class), "missing {class:?}");
        }
        let home = Path::new("/home/player");
        let prefix = home.join(".local");
        let paths: Vec<_> = expanded_specs(home, &prefix)
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        assert!(paths.contains(&prefix.join("bin/scuffed-stat-tracker")));
        assert!(paths.contains(&prefix.join("bin/stat-tracker-gui")));
        assert!(
            paths.contains(&home.join(".local/share/applications/scuffed-stat-tracker.desktop"))
        );
        assert!(paths.contains(&home.join(".config/systemd/user/scuffed-stat-tracker.service")));
        assert!(paths.contains(&home.join(
            ".config/systemd/user/graphical-session.target.wants/scuffed-stat-tracker.service"
        )));
        assert!(paths.contains(&home.join(".local/share/scuffed-stat-tracker")));
        assert!(paths.contains(&home.join(".config/scuffed-stat-tracker")));
        assert!(
            specs
                .iter()
                .filter(|spec| spec.class == PathClass::Data || spec.class == PathClass::Config)
                .all(|spec| spec.when == When::Purge)
        );
    }

    #[test]
    fn package_commands_match_the_package_manager() {
        assert_eq!(
            parse_pacman_qo("/usr/bin/stat-tracker-gui is owned by scuffed-stat-tracker 1.0-1")
                .as_deref(),
            Some("scuffed-stat-tracker")
        );
        assert_eq!(
            parse_dpkg_s("scuffed-stat-tracker: /usr/bin/stat-tracker-gui").as_deref(),
            Some("scuffed-stat-tracker")
        );
        let pacman = PackageOwner {
            manager: PackageManager::Pacman,
            package: "scuffed-stat-tracker".into(),
        };
        let apt = PackageOwner {
            manager: PackageManager::Apt,
            package: "scuffed-stat-tracker".into(),
        };
        assert_eq!(
            pacman.remove_command(),
            "sudo pacman -R scuffed-stat-tracker"
        );
        assert_eq!(apt.remove_command(), "sudo apt remove scuffed-stat-tracker");
    }

    #[test]
    fn package_managed_install_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        write(&home.join("keep-me"), "keep");
        let exe = prefix.join("bin/stat-tracker-gui");
        let owner = PackageOwner {
            manager: PackageManager::Pacman,
            package: "scuffed-stat-tracker".into(),
        };
        let origin = decide(&exe, &home, Some(owner.clone()));
        assert_eq!(
            origin,
            InstallOrigin::Package {
                command: "sudo pacman -R scuffed-stat-tracker".into()
            }
        );
        let report = apply(&request(&home, &prefix, true, origin)).unwrap();
        assert!(report.skipped);
        assert_eq!(report.removed_files, 0);
        assert!(exe.is_file());
        assert!(home.join("keep-me").is_file());
        assert!(
            spec_path(&home, &prefix, PathClass::Data)
                .join("stats.surrealkv/db")
                .is_file()
        );
        assert!(!home.join("systemctl.log").is_file());
    }

    #[test]
    fn package_owner_of_a_usr_binary_deletes_nothing() {
        let home = Path::new("/home/player");
        let exe = Path::new("/usr/bin/stat-tracker-gui");
        let owner = PackageOwner {
            manager: PackageManager::Pacman,
            package: "scuffed-stat-tracker".into(),
        };
        assert_eq!(
            decide(exe, home, Some(owner)),
            InstallOrigin::Package {
                command: "sudo pacman -R scuffed-stat-tracker".into()
            }
        );
    }

    #[test]
    fn usr_path_without_a_package_owner_is_not_a_script_install() {
        let home = Path::new("/home/player");
        let exe = Path::new("/usr/bin/stat-tracker-gui");
        assert!(matches!(
            decide(exe, home, None),
            InstallOrigin::Outside { .. }
        ));
        let dialog = open_dialog(
            exe,
            home,
            None,
            &home.join(".local/share/scuffed-stat-tracker"),
        );
        match dialog {
            UninstallDialog::Manual {
                command,
                detail,
                paths,
            } => {
                assert!(command.is_none());
                assert_eq!(detail, "Remove it manually.");
                assert!(paths.is_empty() || paths.iter().all(|path| !path.starts_with("/home")));
            }
            other => panic!("expected a manual dialog, got {other:?}"),
        }
    }

    #[test]
    fn appimage_is_a_script_install() {
        let home = Path::new("/home/player");
        let exe = home.join("Downloads/ScuffedStatTracker.AppImage");
        match decide(&exe, home, None) {
            InstallOrigin::Bootstrap { prefix } => assert_eq!(prefix, home.join(".local")),
            other => panic!("expected a script install, got {other:?}"),
        }
        let mounted = Path::new("/tmp/.mount_ScuffedXXXX/usr/bin/stat-tracker-gui");
        match decide(mounted, home, None) {
            InstallOrigin::Bootstrap { prefix } => assert_eq!(prefix, home.join(".local")),
            other => panic!("expected a mounted AppImage to be a script install, got {other:?}"),
        }
        let cargo = Path::new("/workspace/target/debug/stat-tracker-gui");
        assert!(matches!(
            decide(cargo, home, None),
            InstallOrigin::Outside { .. }
        ));
    }

    #[test]
    fn units_include_the_timer() {
        let units = units_to_disable();
        assert!(
            units
                .iter()
                .any(|unit| unit == "scuffed-stat-tracker.service")
        );
        assert!(
            units
                .iter()
                .any(|unit| unit == "scuffed-stat-tracker.timer")
        );
        assert!(
            units
                .iter()
                .any(|unit| unit == "scuffed-stat-tracker-session.service")
        );
    }

    #[test]
    fn keep_data_leaves_the_data_dir_and_outside_paths() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        write(&home.join("keep-me"), "keep");
        write(&prefix.join("bin/other-tool"), "other");
        write(&prefix.join("lib/libother.so"), "other");
        write(
            &home.join(".local/share/applications/mimeinfo.cache"),
            "mime",
        );
        write(
            &home.join(".config/systemd/user/some-other.service"),
            "other",
        );
        write(
            &prefix.join("lib/scuffed-stat-tracker/keep-user.txt"),
            "user",
        );
        let neighbor = prefix.join("bin/neighbor-tool");
        write(&neighbor, "neighbor\n");

        let exe = prefix.join("bin/stat-tracker-gui");
        let origin = decide(&exe, &home, None);
        assert!(matches!(origin, InstallOrigin::Bootstrap { .. }));
        let dialog = open_dialog(
            &exe,
            &home,
            None,
            &spec_path(&home, &prefix, PathClass::Data),
        );
        match &dialog {
            UninstallDialog::Confirm { delete_data, .. } => assert!(!*delete_data),
            other => panic!("expected confirm, got {other:?}"),
        }

        let report = apply(&request(&home, &prefix, false, origin)).unwrap();
        assert!(!report.skipped);
        assert!(report.removed_files > 0);
        let log = fs::read_to_string(home.join("systemctl.log")).unwrap();
        assert!(log.contains("disable --now scuffed-stat-tracker.service"));
        assert!(log.contains("disable --now scuffed-stat-tracker.timer"));
        assert!(!log.contains("UNIT_MISSING"));
        assert!(neighbor.is_file(), "file next to the binary was removed");
        assert_eq!(fs::read_to_string(&neighbor).unwrap(), "neighbor\n");
        assert!(!exe.exists());
        assert!(!spec_path(&home, &prefix, PathClass::Desktop).exists());
        assert!(!spec_path(&home, &prefix, PathClass::Unit).exists());
        assert!(!spec_path(&home, &prefix, PathClass::Autostart).exists());
        let data = spec_path(&home, &prefix, PathClass::Data);
        assert!(data.join("stats.surrealkv/db").is_file(), "games database");
        assert!(data.join("debug/crop.png").is_file(), "debug crop");
        assert!(data.join("shadow/digits.jsonl").is_file(), "shadow log");
        let config = spec_path(&home, &prefix, PathClass::Config).join("config.toml");
        assert!(fs::read_to_string(config).unwrap().contains("secret-token"));
        assert_eq!(fs::read_to_string(home.join("keep-me")).unwrap(), "keep");
        assert!(prefix.join("bin/other-tool").is_file());
        assert!(neighbor.is_file());
        assert!(prefix.join("lib/libother.so").is_file());
        assert!(
            home.join(".local/share/applications/mimeinfo.cache")
                .is_file()
        );
        assert!(
            home.join(".config/systemd/user/some-other.service")
                .is_file()
        );
        assert!(
            prefix
                .join("lib/scuffed-stat-tracker/keep-user.txt")
                .is_file()
        );
    }

    #[test]
    fn missing_manifest_confirms_the_fixed_list_and_keeps_a_neighbor() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let manifest = spec_path(&home, &prefix, PathClass::Manifest);
        fs::remove_file(&manifest).unwrap();
        let neighbor = prefix.join("bin/neighbor-tool");
        write(&neighbor, "neighbor\n");
        let exe = prefix.join("bin/stat-tracker-gui");
        let dialog = open_dialog(
            &exe,
            &home,
            None,
            &spec_path(&home, &prefix, PathClass::Data),
        );
        match &dialog {
            UninstallDialog::Confirm {
                fallback,
                delete_data,
                ..
            } => {
                assert!(*fallback);
                assert!(!*delete_data);
            }
            other => panic!("expected confirm, got {other:?}"),
        }
        let shown: Vec<_> = preview(
            &home,
            &prefix,
            false,
            &spec_path(&home, &prefix, PathClass::Data),
            None,
        )
        .into_iter()
        .flat_map(|group| group.paths)
        .collect();
        assert!(shown.contains(&exe));
        assert!(!shown.contains(&neighbor));
        let origin = decide(&exe, &home, None);
        apply(&request(&home, &prefix, false, origin)).unwrap();
        assert!(!exe.exists());
        assert!(neighbor.is_file());
        assert_eq!(fs::read_to_string(&neighbor).unwrap(), "neighbor\n");
        let config = spec_path(&home, &prefix, PathClass::Config).join("config.toml");
        assert!(fs::read_to_string(config).unwrap().contains("secret-token"));
    }

    #[test]
    fn config_toml_recorded_in_the_manifest_stays_unless_data_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let config = spec_path(&home, &prefix, PathClass::Config).join("config.toml");
        let manifest = spec_path(&home, &prefix, PathClass::Manifest);
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push('\n');
        text.push_str(&config.display().to_string());
        text.push('\n');
        fs::write(&manifest, text).unwrap();
        let plan = removal_plan(&home, &prefix, None);
        assert!(!plan.fallback);
        assert!(!plan.files.iter().any(|path| path == &config));
        let origin = decide(&prefix.join("bin/stat-tracker-gui"), &home, None);
        apply(&request(&home, &prefix, false, origin)).unwrap();
        assert!(
            fs::read_to_string(&config)
                .unwrap()
                .contains("secret-token")
        );
    }

    #[test]
    fn purge_removes_data_and_config_only() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        write(&home.join("keep-me"), "keep");
        let origin = decide(&prefix.join("bin/stat-tracker-gui"), &home, None);
        apply(&request(&home, &prefix, true, origin)).unwrap();
        assert!(!spec_path(&home, &prefix, PathClass::Data).exists());
        assert!(!spec_path(&home, &prefix, PathClass::Config).exists());
        assert_eq!(fs::read_to_string(home.join("keep-me")).unwrap(), "keep");
    }

    #[test]
    fn data_dir_symlink_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let data = spec_path(&home, &prefix, PathClass::Data);
        fs::remove_dir_all(&data).unwrap();
        write(&elsewhere.join("secret"), "nope");
        std::os::unix::fs::symlink(&elsewhere, &data).unwrap();
        let origin = decide(&prefix.join("bin/stat-tracker-gui"), &home, None);
        let err = apply(&request(&home, &prefix, true, origin)).unwrap_err();
        assert!(err.contains("Still there"), "{err}");
        assert!(!err.contains("Os {"), "{err}");
        assert!(elsewhere.join("secret").is_file());
    }

    #[test]
    fn manifest_cannot_escape_home_or_use_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let outside = dir.path().join("outside-secret");
        write(&outside, "secret\n");
        let manifest = spec_path(&home, &prefix, PathClass::Manifest);
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str("\n/etc/x\n");
        text.push_str(&format!(
            "{}\n",
            prefix.join("bin/../../outside-secret").display()
        ));
        text.push_str(&format!("{}\n", outside.display()));
        fs::write(&manifest, text).unwrap();
        let plan = removal_plan(&home, &prefix, None);
        assert!(
            plan.files
                .iter()
                .all(|path| !path.ends_with("outside-secret"))
        );
        assert!(plan.files.iter().all(|path| path != Path::new("/etc/x")));
        let origin = decide(&prefix.join("bin/stat-tracker-gui"), &home, None);
        apply(&request(&home, &prefix, false, origin)).unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "secret\n");
        assert!(!prefix.join("bin/stat-tracker-gui").exists());
    }

    #[test]
    fn prefix_outside_home_lists_paths_and_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = dir.path().join("opt").join("scuffed");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let exe = prefix.join("bin/stat-tracker-gui");
        assert!(matches!(
            decide(&exe, &home, None),
            InstallOrigin::Outside { .. }
        ));
        let dialog = open_dialog(&exe, &home, None, &home.join("data"));
        match dialog {
            UninstallDialog::Manual { detail, paths, .. } => {
                assert_eq!(detail, "Remove it manually.");
                assert!(paths.contains(&exe));
            }
            other => panic!("expected a manual dialog, got {other:?}"),
        }
        let mut req = request(
            &home,
            &prefix,
            true,
            InstallOrigin::Bootstrap {
                prefix: prefix.clone(),
            },
        );
        req.prefix = prefix.clone();
        let report = apply(&req).unwrap();
        assert!(report.skipped);
        assert!(exe.is_file());
    }

    #[test]
    fn checkbox_deletes_the_configured_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let custom = home.join("saved-games");
        write(&custom.join("stats.surrealkv/db"), "custom\n");
        let default_data = spec_path(&home, &prefix, PathClass::Data);
        let shown: Vec<_> = preview(&home, &prefix, true, &custom, None)
            .into_iter()
            .flat_map(|group| group.paths)
            .collect();
        assert!(shown.contains(&custom));
        assert!(!shown.contains(&default_data) || custom == default_data);
        let mut req = request(
            &home,
            &prefix,
            true,
            decide(&prefix.join("bin/stat-tracker-gui"), &home, None),
        );
        req.data_dir = custom.clone();
        apply(&req).unwrap();
        assert!(!custom.exists());
        assert!(default_data.join("stats.surrealkv/db").is_file());

        let outside = dir.path().join("outside-data");
        write(&outside.join("db"), "nope\n");
        let saved = preview(&home, &prefix, true, &outside, None)
            .into_iter()
            .find(|group| group.heading == "Saved games")
            .unwrap();
        assert!(saved.paths.contains(&outside));
        assert!(saved.note.contains("left in place"));
        layout(&home, &prefix);
        let mut left = request(
            &home,
            &prefix,
            true,
            decide(&prefix.join("bin/stat-tracker-gui"), &home, None),
        );
        left.data_dir = outside.clone();
        apply(&left).unwrap();
        assert!(outside.join("db").is_file());
    }

    #[test]
    fn appimage_file_is_removed_and_missing_files_are_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join("Downloads")).unwrap();
        let image = home.join("Downloads/ScuffedStatTracker.AppImage");
        write(&image, "appimage\n");
        let prefix = home.join(".local");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        let missing = prefix.join("bin/stat-tracker-gui");
        match decide(&image, &home, None) {
            InstallOrigin::Bootstrap { prefix: got } => assert_eq!(got, prefix),
            other => panic!("expected a script install, got {other:?}"),
        }
        let shown: Vec<_> = preview(&home, &prefix, false, &home.join("data"), Some(&image))
            .into_iter()
            .flat_map(|group| group.paths)
            .collect();
        assert!(shown.contains(&image));
        assert!(!shown.contains(&missing));
        let mut req = request(
            &home,
            &prefix,
            false,
            InstallOrigin::Bootstrap {
                prefix: prefix.clone(),
            },
        );
        req.appimage = Some(image.clone());
        write(&req.systemctl, "#!/bin/sh\nexit 0\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&req.systemctl).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&req.systemctl, perms).unwrap();
        }
        apply(&req).unwrap();
        assert!(!image.exists());
    }

    #[test]
    fn probe_package_owner_reads_the_package_manager() {
        let dir = tempfile::tempdir().unwrap();
        let pacman = dir.path().join("pacman");
        write(
            &pacman,
            "#!/bin/sh\nif [ \"$1\" = -Qo ]; then echo \"$2 is owned by scuffed-stat-tracker 1.0-1\"; exit 0; fi\nexit 1\n",
        );
        let dpkg = dir.path().join("dpkg");
        write(&dpkg, "#!/bin/sh\nexit 1\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&pacman, &dpkg] {
                let mut perms = fs::metadata(path).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(path, perms).unwrap();
            }
        }
        let target = Path::new("/usr/bin/stat-tracker-gui");
        let owner = probe_package_owner_with(target, &pacman, &dpkg).unwrap();
        assert_eq!(
            owner.remove_command(),
            "sudo pacman -R scuffed-stat-tracker"
        );
        assert!(probe_package_owner(&dir.path().join("not-a-tracked-binary")).is_none());
    }

    #[test]
    fn systemctl_failure_removes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let req = request(
            &home,
            &prefix,
            false,
            decide(&prefix.join("bin/stat-tracker-gui"), &home, None),
        );
        write(&req.systemctl, "#!/bin/sh\necho active\nexit 0\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&req.systemctl).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&req.systemctl, perms).unwrap();
        }
        let err = apply(&req).unwrap_err();
        assert!(err.contains("still running"), "{err}");
        assert!(prefix.join("bin/stat-tracker-gui").is_file());
    }

    #[test]
    fn halfway_failure_says_what_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let prefix = home.join(".local");
        fs::create_dir_all(&home).unwrap();
        layout(&home, &prefix);
        let bin = prefix.join("bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o555);
            fs::set_permissions(&bin, perms).unwrap();
        }
        let origin = decide(&prefix.join("bin/stat-tracker-gui"), &home, None);
        let err = apply(&request(&home, &prefix, false, origin)).unwrap_err();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin, perms).unwrap();
        }
        assert!(err.contains("Uninstall stopped"), "{err}");
        assert!(err.contains("Still there"), "{err}");
        assert!(!err.contains("Os {"), "{err}");
        assert!(prefix.join("bin/stat-tracker-gui").is_file());
    }
}
