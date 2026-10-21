//! Which capture route a start uses, and the log lines for that choice.
//!
//! `STAT_TRACKER_CAPTURE`, when it is non-empty, wins over Settings.
//! Otherwise `capture_route` in config wins when it is not `auto`.
//! Otherwise the usual probe order runs, and a portal desktop uses the
//! screen stream (hidden cursor) rather than a fresh screenshot each time.
//!
//! `portal` and `screencast` are that stream. `portal-screenshot` is the
//! older screenshot portal. wlroots and X11 stay on their direct capture
//! unless the stream or the screenshot portal is forced.

use super::{CaptureBackend, select_backend};

/// Where the winning choice came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Environment,
    Settings,
    Automatic,
}

/// Portal image source. Only read when the backend is [`CaptureBackend::Portal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalPath {
    /// Long-lived ScreenCast stream, cursor hidden.
    Stream,
    /// One screenshot portal request per grab.
    Screenshot,
}

/// What a non-empty env value or a Settings choice asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceWord {
    Wayshot,
    X11,
    Stream,
    Screenshot,
    Unknown,
}

/// Env, else Settings, else automatic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    Auto,
    Forced(ForceWord),
}

/// One startup decision, including the lines `detect_backend` logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub backend: CaptureBackend,
    pub portal: PortalPath,
    pub source: RouteSource,
    pub forced: bool,
    /// Always logged at info.
    pub info: String,
    /// Logged at info when a method was forced.
    pub forced_line: Option<String>,
    /// Logged at warn when the force does not match this desktop.
    pub mismatch: Option<String>,
}

/// Keyword set shared with `force_target`. Exact, after ASCII case fold.
pub fn classify(raw: &str) -> ForceWord {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wayshot" | "wayland" => ForceWord::Wayshot,
        "x11" => ForceWord::X11,
        "portal" | "screencast" => ForceWord::Stream,
        "portal-screenshot" => ForceWord::Screenshot,
        _ => ForceWord::Unknown,
    }
}

/// Non-empty env wins entirely. Otherwise Settings, unless it is `auto`.
pub fn effective(env: Option<&str>, settings_word: Option<ForceWord>) -> Effective {
    match env.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Effective::Forced(classify(raw)),
        None => match settings_word {
            Some(word) => Effective::Forced(word),
            None => Effective::Auto,
        },
    }
}

/// Canonical keyword `select_backend` already understands.
fn keyword(word: ForceWord) -> &'static str {
    match word {
        ForceWord::Wayshot => "wayshot",
        ForceWord::X11 => "x11",
        ForceWord::Stream => "screencast",
        ForceWord::Screenshot => "portal-screenshot",
        ForceWord::Unknown => "unknown",
    }
}

fn method_name(backend: CaptureBackend, portal: PortalPath) -> &'static str {
    match backend {
        CaptureBackend::Wayshot => "wayshot",
        CaptureBackend::X11 => "x11",
        CaptureBackend::None => "none",
        CaptureBackend::Portal => match portal {
            PortalPath::Stream => "screen stream",
            PortalPath::Screenshot => "screenshot portal",
        },
    }
}

fn source_name(source: RouteSource) -> &'static str {
    match source {
        RouteSource::Environment => "environment",
        RouteSource::Settings => "settings",
        RouteSource::Automatic => "automatic",
    }
}

fn desktop_label(desktop: &str) -> &str {
    let trimmed = desktop.trim();
    if trimmed.is_empty() { "unset" } else { trimmed }
}

fn is_kde_or_gnome(desktop: &str) -> bool {
    desktop.split(':').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        token == "kde"
            || token == "gnome"
            || token == "plasma"
            || token.contains("kde")
            || token.contains("gnome")
    })
}

fn mismatch_for(word: ForceWord, wayland: bool, desktop: &str) -> Option<String> {
    let kde_gnome = is_kde_or_gnome(desktop);
    match word {
        ForceWord::Wayshot if kde_gnome => Some(
            "capture method mismatch: wayshot was forced on a KDE or GNOME desktop, which does not serve wlroots screencopy"
                .to_string(),
        ),
        ForceWord::X11 if wayland && kde_gnome => Some(
            "capture method mismatch: x11 was forced on a Wayland KDE or GNOME desktop, so capture goes through XWayland"
                .to_string(),
        ),
        ForceWord::Stream if !wayland => Some(
            "capture method mismatch: the screen stream was forced but this is not a Wayland session"
                .to_string(),
        ),
        // The older screenshot portal on KDE is an intentional switch.
        _ => None,
    }
}

/// Pure decision. Probes are the caller's.
pub fn resolve(
    env: Option<&str>,
    settings_word: Option<ForceWord>,
    wayland: bool,
    wayshot_ok: bool,
    x11_ok: bool,
    portal_ok: bool,
    desktop: &str,
) -> Decision {
    let chosen = effective(env, settings_word);
    let (source, forced_word) = match chosen {
        Effective::Auto => (RouteSource::Automatic, None),
        Effective::Forced(word) => {
            let source = if env.map(str::trim).is_some_and(|s| !s.is_empty()) {
                RouteSource::Environment
            } else {
                RouteSource::Settings
            };
            (source, Some(word))
        }
    };
    let force_raw = forced_word.map(keyword);
    let backend = select_backend(force_raw, wayland, wayshot_ok, x11_ok, portal_ok);
    let portal = match forced_word {
        Some(ForceWord::Screenshot) => PortalPath::Screenshot,
        _ => PortalPath::Stream,
    };
    let desktop_shown = desktop_label(desktop);
    let name = method_name(backend, portal);
    let info = format!(
        "capture route resolved: {name}, source {}, desktop {desktop_shown}",
        source_name(source)
    );
    let forced_line = forced_word.map(|word| {
        format!(
            "capture method forced: {}; desktop detected: {desktop_shown}",
            method_name(
                backend,
                if word == ForceWord::Screenshot {
                    PortalPath::Screenshot
                } else {
                    portal
                }
            )
        )
    });
    // A forced name that could not be selected still names what was asked.
    let forced_line = match (forced_word, backend) {
        (Some(ForceWord::Unknown), _) => Some(format!(
            "capture method forced: unknown value; desktop detected: {desktop_shown}"
        )),
        (Some(word), CaptureBackend::None) => Some(format!(
            "capture method forced: {} is not usable; desktop detected: {desktop_shown}",
            match word {
                ForceWord::Wayshot => "wayshot",
                ForceWord::X11 => "x11",
                ForceWord::Stream => "screen stream",
                ForceWord::Screenshot => "screenshot portal",
                ForceWord::Unknown => "unknown value",
            }
        )),
        _ => forced_line,
    };
    let mismatch = forced_word.and_then(|word| mismatch_for(word, wayland, desktop));
    Decision {
        backend,
        portal,
        source,
        forced: forced_word.is_some(),
        info,
        forced_line,
        mismatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(
        env: Option<&str>,
        settings: Option<ForceWord>,
        wayland: bool,
        wayshot: bool,
        x11: bool,
        portal: bool,
        desktop: &str,
    ) -> Decision {
        resolve(env, settings, wayland, wayshot, x11, portal, desktop)
    }

    #[test]
    fn portal_env_is_the_stream_and_beats_settings() {
        let d = decide(
            Some("portal"),
            Some(ForceWord::Screenshot),
            true,
            true,
            true,
            true,
            "KDE",
        );
        assert_eq!(d.backend, CaptureBackend::Portal);
        assert_eq!(d.portal, PortalPath::Stream);
        assert_eq!(d.source, RouteSource::Environment);
        assert!(d.forced);
        assert!(d.info.contains("screen stream"));
        assert!(d.info.contains("source environment"));
        assert!(d.info.contains("desktop KDE"));
        assert!(
            d.mismatch.is_none(),
            "portal on KDE is the stream, not a mismatch"
        );
        assert!(d.forced_line.unwrap().contains("screen stream"));
    }

    #[test]
    fn screencast_env_forces_the_stream_even_when_wayshot_works() {
        let d = decide(Some("screencast"), None, true, true, true, true, "niri");
        assert_eq!(d.backend, CaptureBackend::Portal);
        assert_eq!(d.portal, PortalPath::Stream);
        assert_eq!(d.source, RouteSource::Environment);
        assert!(
            d.mismatch.is_none(),
            "niri is Wayland, so a forced stream is not a mismatch"
        );
    }

    #[test]
    fn portal_screenshot_is_the_old_route_and_not_a_kde_mismatch() {
        let d = decide(
            Some("portal-screenshot"),
            None,
            true,
            true,
            true,
            true,
            "KDE",
        );
        assert_eq!(d.backend, CaptureBackend::Portal);
        assert_eq!(d.portal, PortalPath::Screenshot);
        assert!(d.info.contains("screenshot portal"));
        assert!(d.mismatch.is_none());
    }

    #[test]
    fn settings_stream_is_used_only_when_env_is_empty() {
        let d = decide(
            None,
            Some(ForceWord::Stream),
            true,
            true,
            false,
            true,
            "niri",
        );
        assert_eq!(d.source, RouteSource::Settings);
        assert_eq!(d.portal, PortalPath::Stream);
        assert_eq!(d.backend, CaptureBackend::Portal);
        let blank = decide(
            Some("  "),
            Some(ForceWord::Screenshot),
            true,
            false,
            false,
            true,
            "GNOME",
        );
        assert_eq!(blank.source, RouteSource::Settings);
        assert_eq!(blank.portal, PortalPath::Screenshot);
    }

    #[test]
    fn automatic_keeps_wayshot_and_uses_the_stream_for_portal() {
        let wayshot = decide(None, None, true, true, true, true, "niri");
        assert_eq!(wayshot.backend, CaptureBackend::Wayshot);
        assert_eq!(wayshot.source, RouteSource::Automatic);
        assert!(!wayshot.forced);
        assert!(wayshot.forced_line.is_none());
        let portal = decide(None, None, true, false, true, true, "KDE");
        assert_eq!(portal.backend, CaptureBackend::Portal);
        assert_eq!(portal.portal, PortalPath::Stream);
        assert!(portal.info.contains("source automatic"));
    }

    #[test]
    fn mismatch_lines_name_the_desktop_case() {
        let wayshot = decide(Some("wayshot"), None, true, false, true, true, "KDE");
        let mismatch = wayshot.mismatch.unwrap();
        assert!(mismatch.contains("wayshot"));
        assert!(mismatch.contains("KDE or GNOME"));
        let x11 = decide(Some("x11"), None, true, false, true, true, "ubuntu:GNOME");
        assert!(x11.mismatch.unwrap().contains("XWayland"));
        let stream = decide(Some("screencast"), None, false, false, true, true, "");
        assert!(stream.mismatch.unwrap().contains("not a Wayland session"));
        assert!(stream.info.contains("desktop unset"));
        let shot = decide(
            Some("portal-screenshot"),
            None,
            true,
            false,
            true,
            true,
            "KDE",
        );
        assert!(shot.mismatch.is_none());
    }

    #[test]
    fn unknown_env_fails_closed_and_is_logged_as_forced() {
        let d = decide(
            Some("garbage"),
            Some(ForceWord::Stream),
            true,
            true,
            true,
            true,
            "KDE",
        );
        assert_eq!(d.backend, CaptureBackend::None);
        assert_eq!(d.source, RouteSource::Environment);
        assert!(d.forced_line.unwrap().contains("unknown"));
        assert!(d.info.contains("source environment"));
    }

    #[test]
    fn classify_is_case_insensitive() {
        assert_eq!(classify("Portal"), ForceWord::Stream);
        assert_eq!(classify("ScreenCast"), ForceWord::Stream);
        assert_eq!(classify("portal-screenshot"), ForceWord::Screenshot);
        assert_eq!(classify("WAYLAND"), ForceWord::Wayshot);
        assert_eq!(classify("nope"), ForceWord::Unknown);
    }
}
