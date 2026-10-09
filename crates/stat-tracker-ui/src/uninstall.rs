//! Uninstall a bootstrap.sh user install from the desktop app.
//!
//! The path list is `crates/stat-tracker/dist/install-paths.sh`. install.sh,
//! uninstall.sh, and `bootstrap.sh --uninstall` read that same file. This
//! module parses it so the app cannot remove a path the installer does not
//! name.
//!
//! A pacman/AUR or apt/dpkg install, and any binary under `/usr`, is left
//! on disk. The dialog shows `sudo pacman -R <pkg>` or `sudo apt remove <pkg>`.

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

const DAEMON_UNIT: &str = "scuffed-stat-tracker.service";
const SESSION_UNIT: &str = "scuffed-stat-tracker-session.service";

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
    /// User-level bootstrap.sh / install.sh prefix.
    Bootstrap { prefix: PathBuf },
    /// Owned by pacman or dpkg. `command` is what the user should run.
    Package { command: String },
    /// Not a bootstrap install. Nothing is deleted.
    Outside { exe: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallDialog {
    Confirm {
        home: PathBuf,
        prefix: PathBuf,
        /// Unchecked until the user opts in. Local data stays.
        delete_data: bool,
    },
    /// Package manager, or a path this app must not delete.
    Manual {
        command: Option<String>,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallRequest {
    pub home: PathBuf,
    pub prefix: PathBuf,
    pub delete_data: bool,
    pub origin: InstallOrigin,
    pub systemctl: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    /// True when the origin was not a bootstrap install. No path was touched.
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

/// `/usr`, `/bin`, `/lib`, `/opt`, `/etc`, and anything under them.
pub fn is_system_install_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    const ROOTS: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64", "/opt", "/etc"];
    ROOTS
        .iter()
        .any(|root| text == *root || text.starts_with(&format!("{root}/")))
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
    let target = path.display().to_string();
    if let Some(text) = command_stdout("pacman", &["-Qo".to_string(), target.clone()])
        && let Some(package) = parse_pacman_qo(&text)
    {
        return Some(PackageOwner {
            manager: PackageManager::Pacman,
            package,
        });
    }
    if let Some(text) = command_stdout("dpkg", &["-S".to_string(), target])
        && let Some(package) = parse_dpkg_s(&text)
    {
        return Some(PackageOwner {
            manager: PackageManager::Apt,
            package,
        });
    }
    None
}

fn command_stdout(cmd: &str, args: &[String]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
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
    if let Some(owner) = owner {
        return InstallOrigin::Package {
            command: owner.remove_command(),
        };
    }
    if is_system_install_path(exe) {
        return InstallOrigin::Outside {
            exe: exe.to_path_buf(),
        };
    }
    if let Some(prefix) = prefix_from_gui(exe) {
        if is_system_install_path(&prefix) {
            return InstallOrigin::Outside {
                exe: exe.to_path_buf(),
            };
        }
        let manifest = spec_path(home, &prefix, PathClass::Manifest);
        let default_user = home.join(".local");
        if prefix == default_user || manifest.is_file() {
            return InstallOrigin::Bootstrap { prefix };
        }
    }
    InstallOrigin::Outside {
        exe: exe.to_path_buf(),
    }
}

fn spec_path(home: &Path, prefix: &Path, class: PathClass) -> PathBuf {
    expanded_specs(home, prefix)
        .into_iter()
        .find(|(spec, _)| spec.class == class)
        .map(|(_, path)| path)
        .unwrap_or_default()
}

pub fn open_dialog(exe: &Path, home: &Path, owner: Option<PackageOwner>) -> UninstallDialog {
    match decide(exe, home, owner) {
        InstallOrigin::Bootstrap { prefix } => UninstallDialog::Confirm {
            home: home.to_path_buf(),
            prefix,
            delete_data: false,
        },
        InstallOrigin::Package { command } => UninstallDialog::Manual {
            detail:
                "This copy was installed by a package manager. Nothing on disk will be removed."
                    .into(),
            command: Some(command),
        },
        InstallOrigin::Outside { exe } => UninstallDialog::Manual {
            detail: format!(
                "This copy is outside the bootstrap.sh install paths ({}). Nothing will be removed.",
                exe.display()
            ),
            command: None,
        },
    }
}

pub fn preview(home: &Path, prefix: &Path, delete_data: bool) -> Vec<PreviewGroup> {
    let specs = expanded_specs(home, prefix);
    let mut groups = vec![
        group(&specs, "Binaries", "", &[PathClass::Bin]),
        libraries_group(home, prefix, &specs),
        group(
            &specs,
            "Systemd user service",
            "Stopped and disabled first.",
            &[PathClass::Unit, PathClass::DropIn],
        ),
        group(
            &specs,
            "Desktop entry and icon",
            "The desktop file names the theme icon applications-games. No icon file is installed, so none is removed.",
            &[PathClass::Desktop],
        ),
        group(&specs, "Autostart entry", "", &[PathClass::Autostart]),
        group(&specs, "Install manifest", "", &[PathClass::Manifest]),
    ];
    if delete_data {
        groups.push(group(
            &specs,
            "Local data",
            "Games database, debug crops, and shadow logs.",
            &[PathClass::Data],
        ));
        groups.push(group(
            &specs,
            "Config",
            "Includes the sync token.",
            &[PathClass::Config],
        ));
    }
    groups
}

fn group(
    specs: &[(Spec, PathBuf)],
    heading: &'static str,
    note: &'static str,
    classes: &[PathClass],
) -> PreviewGroup {
    PreviewGroup {
        heading,
        note,
        paths: specs
            .iter()
            .filter(|(spec, _)| classes.contains(&spec.class))
            .map(|(_, path)| path.clone())
            .collect(),
    }
}

fn libraries_group(home: &Path, prefix: &Path, specs: &[(Spec, PathBuf)]) -> PreviewGroup {
    let mut paths: Vec<PathBuf> = specs
        .iter()
        .filter(|(spec, _)| {
            matches!(
                spec.class,
                PathClass::LibDir | PathClass::Helper | PathClass::List
            )
        })
        .map(|(_, path)| path.clone())
        .collect();
    let lib_dir = specs
        .iter()
        .find(|(spec, _)| spec.class == PathClass::LibDir)
        .map(|(_, path)| path.clone());
    if let Some(lib_dir) = lib_dir {
        for line in manifest_lines(home, prefix) {
            if is_under(&line, &lib_dir) && !paths.contains(&line) {
                paths.push(line);
            }
        }
    }
    PreviewGroup {
        heading: "Bundled libraries",
        note: "",
        paths,
    }
}

fn manifest_lines(home: &Path, prefix: &Path) -> Vec<PathBuf> {
    let path = spec_path(home, prefix, PathClass::Manifest);
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

pub fn apply(req: &UninstallRequest) -> Result<UninstallReport, String> {
    let InstallOrigin::Bootstrap { prefix } = &req.origin else {
        return Ok(UninstallReport {
            skipped: true,
            removed_files: 0,
        });
    };
    if prefix != &req.prefix
        || is_system_install_path(&req.prefix)
        || is_system_install_path(&req.prefix.join("bin/stat-tracker-gui"))
    {
        return Ok(UninstallReport {
            skipped: true,
            removed_files: 0,
        });
    }

    // Stop and disable before any listed file is unlinked.
    run_systemctl(&req.systemctl, &["--user", "disable", "--now", DAEMON_UNIT]);
    run_systemctl(&req.systemctl, &["--user", "stop", SESSION_UNIT]);

    let plan = files_to_remove(&req.home, &req.prefix);
    let mut removed_files = 0;
    for path in &plan {
        if remove_listed_file(path, &req.home, &req.prefix)? {
            removed_files += 1;
        }
    }
    if let Some(lib_dir) = lib_dir(&req.home, &req.prefix) {
        rmdir_empty_tree(&lib_dir);
    }
    run_systemctl(&req.systemctl, &["--user", "daemon-reload"]);

    if req.delete_data {
        for class in [PathClass::Data, PathClass::Config] {
            let dir = spec_path(&req.home, &req.prefix, class);
            remove_listed_tree(&dir, &req.home)?;
        }
    }

    Ok(UninstallReport {
        skipped: false,
        removed_files,
    })
}

fn run_systemctl(bin: &Path, args: &[&str]) {
    let _ = Command::new(bin).args(args).status();
}

fn lib_dir(home: &Path, prefix: &Path) -> Option<PathBuf> {
    let path = spec_path(home, prefix, PathClass::LibDir);
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Exact always-files, plus manifest lines that sit inside the library directory.
fn files_to_remove(home: &Path, prefix: &Path) -> Vec<PathBuf> {
    let specs = expanded_specs(home, prefix);
    let mut files: Vec<PathBuf> = specs
        .iter()
        .filter(|(spec, _)| spec.when == When::Always && !is_directory_class(spec.class))
        .map(|(_, path)| path.clone())
        .collect();
    let lib = specs
        .iter()
        .find(|(spec, _)| spec.class == PathClass::LibDir)
        .map(|(_, path)| path.clone());
    if let Some(lib) = lib {
        for line in manifest_lines(home, prefix) {
            if (is_under(&line, &lib) || files.iter().any(|file| file == &line))
                && !files.contains(&line)
            {
                files.push(line);
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn is_directory_class(class: PathClass) -> bool {
    matches!(
        class,
        PathClass::LibDir | PathClass::Data | PathClass::Config
    )
}

fn remove_listed_file(path: &Path, home: &Path, prefix: &Path) -> Result<bool, String> {
    if !is_allowed_file(path, home, prefix) || too_broad(path, home) {
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

fn is_allowed_file(path: &Path, home: &Path, prefix: &Path) -> bool {
    let specs = expanded_specs(home, prefix);
    if specs.iter().any(|(spec, listed)| {
        spec.when == When::Always && !is_directory_class(spec.class) && listed == path
    }) {
        return true;
    }
    specs.iter().any(|(spec, listed)| {
        spec.class == PathClass::LibDir && spec.when == When::Always && is_under(path, listed)
    })
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

fn is_under(child: &Path, parent: &Path) -> bool {
    let child = lexical(child);
    let parent = lexical(parent);
    child.starts_with(&parent) && child != parent
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
        } => confirm_card(home, prefix, *delete_data, busy),
        UninstallDialog::Manual { command, detail } => manual_card(detail, command.as_deref()),
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
    delete_data: bool,
    busy: bool,
) -> Element<'a, Message> {
    let mut body = column![
        text("Uninstall Scuffed Stat Tracker")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text("Only the paths below are removed. The systemd user service is stopped and disabled first.")
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_2),
    ]
    .spacing(8)
    .width(Fill);

    for group in preview(home, prefix, delete_data) {
        body = body.push(group_block(&group));
    }

    let data = spec_path(home, prefix, PathClass::Data);
    let config = spec_path(home, prefix, PathClass::Config);
    body = body.push(
        checkbox(delete_data)
            .label("Also delete local data")
            .on_toggle(Message::ToggleUninstallData)
            .size(18)
            .text_size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .style(checkbox_style),
    );
    body = body.push(
        text(format!(
            "{}\nGames database, debug crops, and shadow logs.\n{}\nConfig, including the sync token.",
            data.display(),
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

fn manual_card<'a>(detail: &'a str, command: Option<&'a str>) -> Element<'a, Message> {
    let mut body = column![
        text("Uninstall")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        text(detail).size(SIZE_BODY).font(FONT_MEDIUM).color(TEXT_2),
    ]
    .spacing(10)
    .width(Fill);

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
        }
        let lib = spec_path(home, prefix, PathClass::LibDir);
        write(&lib.join("ocr/liblept.so.5"), "lib\n");
        let manifest = spec_path(home, prefix, PathClass::Manifest);
        let mut lines = files_to_remove(home, prefix);
        lines.push(lib.join("ocr/liblept.so.5"));
        lines.sort();
        lines.dedup();
        write(
            &manifest,
            &lines
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
    fn paths_under_usr_are_not_a_bootstrap_install() {
        let home = Path::new("/home/player");
        let exe = Path::new("/usr/bin/stat-tracker-gui");
        assert!(matches!(
            decide(exe, home, None),
            InstallOrigin::Outside { .. }
        ));
        let dialog = open_dialog(exe, home, None);
        match dialog {
            UninstallDialog::Manual { command, detail } => {
                assert!(command.is_none());
                assert!(detail.contains("/usr/bin/stat-tracker-gui"));
            }
            other => panic!("expected a manual dialog, got {other:?}"),
        }
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
        let manifest = spec_path(&home, &prefix, PathClass::Manifest);
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push('\n');
        text.push_str(&home.join("keep-me").display().to_string());
        text.push('\n');
        fs::write(&manifest, text).unwrap();

        let exe = prefix.join("bin/stat-tracker-gui");
        let origin = decide(&exe, &home, None);
        assert!(matches!(origin, InstallOrigin::Bootstrap { .. }));
        let dialog = open_dialog(&exe, &home, None);
        match &dialog {
            UninstallDialog::Confirm { delete_data, .. } => assert!(!*delete_data),
            other => panic!("expected confirm, got {other:?}"),
        }

        let report = apply(&request(&home, &prefix, false, origin)).unwrap();
        assert!(!report.skipped);
        assert!(report.removed_files > 0);
        let log = fs::read_to_string(home.join("systemctl.log")).unwrap();
        assert!(log.contains("disable --now scuffed-stat-tracker.service"));
        assert!(!log.contains("UNIT_MISSING"));
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
        assert!(err.contains("symlink"), "{err}");
        assert!(elsewhere.join("secret").is_file());
    }
}
