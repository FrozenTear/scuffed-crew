//! Which xdg-desktop-portal backend is configured, and which one is running.
//!
//! Read from the portal files and `portals.conf`. The process name `comm`
//! is truncated to 15 bytes, so `xdg-desktop-portal-gnome` and
//! `xdg-desktop-portal-kde` look the same there. Callers pass executable
//! basenames from `/proc/<pid>/exe`, not `comm`.

use std::path::Path;

/// One `*.portal` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalFile {
    pub stem: String,
    pub dbus_name: String,
    pub interfaces: Vec<String>,
    pub use_in: Vec<String>,
}

/// `[preferred]` from `portals.conf`. Empty `default` means the file did not set it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preferred {
    pub default: Vec<String>,
    pub screencast: Option<Vec<String>>,
}

/// The backend a desktop will get, as far as the files say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendAnswer {
    pub stem: String,
    pub dbus_name: String,
}

/// `true` for an executable basename of a portal backend.
///
/// The 15-byte `comm` truncation `xdg-desktop-po` is not a basename and
/// does not count. Several backends share that `comm`.
pub fn is_portal_backend_exe(base: &str) -> bool {
    base.starts_with("xdg-desktop-portal") && base.len() > "xdg-desktop-po".len()
}

pub fn parse_portal(stem: &str, text: &str) -> PortalFile {
    let mut dbus_name = String::new();
    let mut interfaces = Vec::new();
    let mut use_in = Vec::new();
    let mut in_portal = false;
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section) = section_name(line) {
            in_portal = section.eq_ignore_ascii_case("portal");
            continue;
        }
        if !in_portal {
            continue;
        }
        let Some((key, value)) = split_kv(line) else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "dbusname" => dbus_name = value.to_string(),
            "interfaces" => interfaces = split_list(value, ';'),
            "usein" => use_in = split_list(value, ';'),
            _ => {}
        }
    }
    PortalFile {
        stem: stem.to_string(),
        dbus_name,
        interfaces,
        use_in,
    }
}

pub fn parse_preferred(text: &str) -> Preferred {
    let mut preferred = Preferred::default();
    let mut in_preferred = false;
    for line in text.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section) = section_name(line) {
            in_preferred = section.eq_ignore_ascii_case("preferred");
            continue;
        }
        if !in_preferred {
            continue;
        }
        let Some((key, value)) = split_kv(line) else {
            continue;
        };
        let list = split_list(value, ';');
        if key.eq_ignore_ascii_case("default") {
            preferred.default = list;
        } else if key.eq_ignore_ascii_case("org.freedesktop.impl.portal.ScreenCast") {
            preferred.screencast = Some(list);
        }
    }
    preferred
}

/// User config replaces keys it sets. A user file with no `default` keeps
/// the system default. A user ScreenCast key, even empty, replaces the system one.
pub fn merge_preferred(system: Preferred, user: Option<Preferred>) -> Preferred {
    let Some(user) = user else {
        return system;
    };
    Preferred {
        default: if user.default.is_empty() {
            system.default
        } else {
            user.default
        },
        screencast: user.screencast.or(system.screencast),
    }
}

/// Pick the ScreenCast backend for `desktop` (`XDG_CURRENT_DESKTOP`, `:` separated).
///
/// UseIn is matched case-insensitively. When nothing matches, the preferred
/// ScreenCast list, then the preferred default, then a running
/// `xdg-desktop-portal-gnome` executable, is the answer. niri often has no
/// UseIn line and is served by the gnome portal.
pub fn answer(
    desktop: &str,
    portals: &[PortalFile],
    preferred: &Preferred,
    running_exes: &[String],
) -> Option<BackendAnswer> {
    let tokens: Vec<String> = desktop
        .split(':')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let screencast: Vec<&PortalFile> = portals
        .iter()
        .filter(|p| {
            p.interfaces.iter().any(|iface| {
                iface.eq_ignore_ascii_case("org.freedesktop.impl.portal.ScreenCast")
                    || iface.to_ascii_lowercase().ends_with(".screencast")
            })
        })
        .collect();
    let used: Vec<&PortalFile> = screencast
        .iter()
        .copied()
        .filter(|p| {
            p.use_in.iter().any(|use_in| {
                let use_in = use_in.to_ascii_lowercase();
                tokens.iter().any(|token| token == &use_in)
            })
        })
        .collect();
    if let Some(found) = pick_preferred(&used, preferred.screencast.as_deref()) {
        return Some(found);
    }
    if let Some(found) = used.first() {
        return Some(to_answer(found));
    }
    let pool: Vec<&PortalFile> = if screencast.is_empty() {
        portals.iter().collect()
    } else {
        screencast
    };
    if let Some(list) = preferred.screencast.as_deref()
        && let Some(found) = pick_named(&pool, list)
    {
        return Some(found);
    }
    if let Some(found) = pick_named(&pool, &preferred.default) {
        return Some(found);
    }
    if running_exes
        .iter()
        .any(|exe| exe == "xdg-desktop-portal-gnome")
    {
        if let Some(found) = pool.iter().find(|p| p.stem.eq_ignore_ascii_case("gnome")) {
            return Some(to_answer(found));
        }
        return Some(BackendAnswer {
            stem: "gnome".to_string(),
            dbus_name: "org.freedesktop.impl.portal.desktop.gnome".to_string(),
        });
    }
    None
}

fn pick_preferred(matched: &[&PortalFile], preferred: Option<&[String]>) -> Option<BackendAnswer> {
    let list = preferred?;
    pick_named(matched, list)
}

fn pick_named(portals: &[&PortalFile], names: &[String]) -> Option<BackendAnswer> {
    for name in names {
        if let Some(found) = portals.iter().find(|p| p.stem.eq_ignore_ascii_case(name)) {
            return Some(to_answer(found));
        }
    }
    None
}

fn to_answer(portal: &PortalFile) -> BackendAnswer {
    BackendAnswer {
        stem: portal.stem.clone(),
        dbus_name: portal.dbus_name.clone(),
    }
}

pub fn configured_line(answer: Option<&BackendAnswer>) -> String {
    match answer {
        Some(answer) => format!(
            "portal backend configured: {} ({})",
            answer.stem, answer.dbus_name
        ),
        None => "portal backend configured: unknown".to_string(),
    }
}

pub fn answered_line(answer: Option<&BackendAnswer>, running: &[String]) -> String {
    let running = if running.is_empty() {
        "none".to_string()
    } else {
        running.join(",")
    };
    match answer {
        Some(answer) => format!(
            "portal backend answered: {} ({}) ; running {running}",
            answer.stem, answer.dbus_name
        ),
        None => format!("portal backend answered: unknown ; running {running}"),
    }
}

/// Executable basenames only. Skips anything that is not a portal backend binary.
#[cfg_attr(not(test), allow(dead_code))]
pub fn portal_exes(basenames: &[String]) -> Vec<String> {
    let mut out: Vec<String> = basenames
        .iter()
        .filter(|name| is_portal_backend_exe(name))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

fn section_name(line: &str) -> Option<&str> {
    let line = line.trim();
    line.starts_with('[')
        .then(|| line.trim_start_matches('[').trim_end_matches(']').trim())
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

fn split_list(value: &str, sep: char) -> Vec<String> {
    value
        .split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Read the host's portal files. Missing directories yield an empty answer.
pub fn configured_from_host(desktop: &str) -> Option<BackendAnswer> {
    let mut portals = Vec::new();
    for dir in [
        "/usr/share/xdg-desktop-portal/portals",
        "/usr/local/share/xdg-desktop-portal/portals",
    ] {
        read_portal_dir(Path::new(dir), &mut portals);
    }
    let system = read_preferred_file(Path::new("/usr/share/xdg-desktop-portal/portals.conf"));
    let user = dirs::config_dir()
        .and_then(|dir| read_preferred_file(&dir.join("xdg-desktop-portal/portals.conf")));
    let preferred = merge_preferred(system.unwrap_or_default(), user);
    answer(desktop, &portals, &preferred, &[])
}

fn read_portal_dir(dir: &Path, out: &mut Vec<PortalFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("portal") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.push(parse_portal(stem, &text));
    }
}

fn read_preferred_file(path: &Path) -> Option<Preferred> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_preferred(&text))
}

/// Basenames of running `xdg-desktop-portal*` executables. `comm` is not used.
pub fn running_portal_exes() -> Vec<String> {
    let mut names = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(_) => return names,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str() else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        if let Some(base) = exe.file_name().and_then(|s| s.to_str())
            && is_portal_backend_exe(base)
        {
            names.push(base.to_string());
        }
    }
    names.sort();
    names.dedup();
    names
}

pub fn log_configured(desktop: &str) {
    let answer = configured_from_host(desktop);
    tracing::info!("{}", configured_line(answer.as_ref()));
}

pub fn log_answered(desktop: &str) {
    let running = running_portal_exes();
    let answer = configured_from_host(desktop).or_else(|| {
        if running.iter().any(|exe| exe == "xdg-desktop-portal-gnome") {
            Some(BackendAnswer {
                stem: "gnome".to_string(),
                dbus_name: "org.freedesktop.impl.portal.desktop.gnome".to_string(),
            })
        } else {
            None
        }
    });
    tracing::info!("{}", answered_line(answer.as_ref(), &running));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kde() -> PortalFile {
        parse_portal(
            "kde",
            "\
[portal]
DBusName=org.freedesktop.impl.portal.desktop.kde
Interfaces=org.freedesktop.impl.portal.ScreenCast;org.freedesktop.impl.portal.Screenshot;
UseIn=KDE;
",
        )
    }

    fn gnome() -> PortalFile {
        parse_portal(
            "gnome",
            "\
[portal]
DBusName=org.freedesktop.impl.portal.desktop.gnome
Interfaces=org.freedesktop.impl.portal.ScreenCast;
UseIn=GNOME;
",
        )
    }

    #[test]
    fn use_in_matches_a_desktop_token_and_preferred_order_wins() {
        let portals = [kde(), gnome()];
        let preferred = parse_preferred(
            "\
[preferred]
default=gtk
org.freedesktop.impl.portal.ScreenCast=gnome;kde
",
        );
        let got = answer("ubuntu:GNOME", &portals, &preferred, &[]).unwrap();
        assert_eq!(got.stem, "gnome");
        assert_eq!(got.dbus_name, "org.freedesktop.impl.portal.desktop.gnome");
        let kde_desktop = answer("KDE", &portals, &preferred, &[]).unwrap();
        assert_eq!(kde_desktop.stem, "kde");
    }

    #[test]
    fn user_preferred_replaces_the_system_screencast_line() {
        let system = parse_preferred("default=kde\n[preferred]\ndefault=kde\n");
        let user = parse_preferred("[preferred]\norg.freedesktop.impl.portal.ScreenCast=gnome\n");
        let merged = merge_preferred(system, Some(user));
        assert_eq!(merged.default, vec!["kde".to_string()]);
        assert_eq!(merged.screencast.unwrap(), vec!["gnome".to_string()]);
    }

    #[test]
    fn niri_with_no_use_in_uses_the_gnome_portal_or_the_running_exe() {
        let portals = [gnome()];
        let preferred = parse_preferred("[preferred]\ndefault=gnome\n");
        let got = answer("niri", &portals, &preferred, &[]).unwrap();
        assert_eq!(got.stem, "gnome");
        let no_pref = Preferred::default();
        let running = vec!["xdg-desktop-portal-gnome".to_string()];
        let got = answer("niri", &portals, &no_pref, &running).unwrap();
        assert_eq!(got.stem, "gnome");
        let line = answered_line(Some(&got), &running);
        assert!(line.contains("portal backend answered: gnome"));
        assert!(line.contains("running xdg-desktop-portal-gnome"));
        assert!(!line.contains("home"));
    }

    #[test]
    fn comm_truncation_is_not_an_executable_name() {
        assert!(!is_portal_backend_exe("xdg-desktop-po"));
        assert!(is_portal_backend_exe("xdg-desktop-portal"));
        assert!(is_portal_backend_exe("xdg-desktop-portal-gnome"));
        assert!(is_portal_backend_exe("xdg-desktop-portal-kde"));
        let names = portal_exes(&[
            "xdg-desktop-po".to_string(),
            "xdg-desktop-portal-kde".to_string(),
            "firefox".to_string(),
        ]);
        assert_eq!(names, vec!["xdg-desktop-portal-kde".to_string()]);
    }

    #[test]
    fn configured_line_names_the_backend_without_a_path() {
        let line = configured_line(Some(&BackendAnswer {
            stem: "kde".to_string(),
            dbus_name: "org.freedesktop.impl.portal.desktop.kde".to_string(),
        }));
        assert_eq!(
            line,
            "portal backend configured: kde (org.freedesktop.impl.portal.desktop.kde)"
        );
        assert_eq!(configured_line(None), "portal backend configured: unknown");
    }
}
