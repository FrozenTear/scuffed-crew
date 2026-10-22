//! Which capture route a start uses, and the log lines for that choice.
//!
//! `STAT_TRACKER_CAPTURE`, when it is non-empty, wins over Settings.
//! Otherwise `capture_route` in config wins when it is not `auto`.
//! Otherwise the usual probe order runs, and a portal desktop uses the
//! screen stream (hidden cursor) rather than a fresh screenshot each time.
//!
//! `portal` and `screencast` are that stream. `portal-screenshot` is the
//! older screenshot portal. `kwin` asks KWin's own screenshot interface
//! (KDE direct). wlroots and X11 stay on their direct capture unless the
//! stream or the screenshot portal is forced.
//!
//! KDE direct is used only when its startup probe answered
//! ([`KwinProbe::Available`]). When it was asked for and the probe failed,
//! the route the user would have had on automatic is used instead, with
//! one warning naming the reason ([`Decision::fallback`]). This is the one
//! forced value that falls back: every other forced value fails closed
//! (R1-3). `kwin` falls back the same way whether it came from
//! `STAT_TRACKER_CAPTURE` or from Settings.

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
    /// KWin's ScreenShot2 interface, called directly (KDE direct).
    Kwin,
    Unknown,
}

/// Whether automatic picks KDE direct on KDE when its probe answers.
///
/// `false` for now: automatic stays the screen stream on KDE, as in rc.6.
/// Flip to `true` before the rc.7 tag only if the KDE tester's A/B shows
/// KDE direct reads the same boards at a lower game-fps cost. Nothing else
/// needs to change: the probe then runs on automatic too, and a failed
/// probe keeps the stream.
pub const KWIN_DIRECT_DEFAULT_ON_KDE: bool = false;

/// The startup probe of KDE direct. Only read when KDE direct was asked
/// for, or when [`KWIN_DIRECT_DEFAULT_ON_KDE`] lets automatic use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KwinProbe {
    /// The probe did not run.
    NotProbed,
    /// The interface answered a 1x1 capture with a picture.
    Available,
    /// The reason it cannot be used, for the log and the app.
    Unavailable(String),
    /// Not tested, and why: the 1x1 render would have run beside the
    /// tracker's screen share on a Plasma without the full KDE bug 524840
    /// fix. Not a verdict on KDE direct.
    NotChecked(String),
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
    /// KDE direct was asked for and its probe failed: the warning that
    /// names the reason and the route used instead. The app shows it too.
    pub fallback: Option<String>,
}

/// Keyword set shared with `force_target`. Exact, after ASCII case fold.
pub fn classify(raw: &str) -> ForceWord {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wayshot" | "wayland" => ForceWord::Wayshot,
        "x11" => ForceWord::X11,
        "portal" | "screencast" => ForceWord::Stream,
        "portal-screenshot" => ForceWord::Screenshot,
        "kwin" | "kde-direct" => ForceWord::Kwin,
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
        // Never passed on: `resolve` decides KDE direct itself.
        ForceWord::Kwin => "kwin",
        ForceWord::Unknown => "unknown",
    }
}

fn method_name(backend: CaptureBackend, portal: PortalPath) -> &'static str {
    match backend {
        CaptureBackend::Wayshot => "wayshot",
        CaptureBackend::X11 => "x11",
        CaptureBackend::Kwin => "KDE direct (KWin ScreenShot2)",
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

fn desktop_has(desktop: &str, names: &[&str]) -> bool {
    desktop.split(':').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        names.iter().any(|name| token.contains(name))
    })
}

/// KDE Plasma, from `XDG_CURRENT_DESKTOP`. The one KDE test: the screen
/// stream's KWin guards use it too.
pub fn is_kde(desktop: &str) -> bool {
    desktop_has(desktop, &["kde", "plasma", "kwin"])
}

/// GNOME, from `XDG_CURRENT_DESKTOP`.
pub fn is_gnome(desktop: &str) -> bool {
    desktop_has(desktop, &["gnome"])
}

/// Logged at warn when the older screenshot portal is forced on GNOME.
pub const GNOME_SCREENSHOT_WARNING: &str = "the older screenshot portal was forced on GNOME: GNOME flashes the screen and plays a sound on every capture";

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
        // xdg-desktop-portal-gnome asks gnome-shell for a flash and the
        // shutter sound on every screenshot, and answers after the flash.
        ForceWord::Screenshot if is_gnome(desktop) => Some(GNOME_SCREENSHOT_WARNING.to_string()),
        // The older screenshot portal on KDE is an intentional switch.
        _ => None,
    }
}

/// Pure decision. Probes are the caller's. `kwin` is the KDE direct probe
/// and `kwin_default_on` is [`KWIN_DIRECT_DEFAULT_ON_KDE`] (a parameter so
/// both settings are tested).
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    env: Option<&str>,
    settings_word: Option<ForceWord>,
    wayland: bool,
    wayshot_ok: bool,
    x11_ok: bool,
    portal_ok: bool,
    desktop: &str,
    kwin: &KwinProbe,
    kwin_default_on: bool,
) -> Decision {
    let chosen = effective(env, settings_word);
    let desktop_shown = desktop_label(desktop);
    if let Effective::Forced(ForceWord::Kwin) = chosen {
        let source = if env.map(str::trim).is_some_and(|s| !s.is_empty()) {
            RouteSource::Environment
        } else {
            RouteSource::Settings
        };
        if *kwin == KwinProbe::Available {
            let name = method_name(CaptureBackend::Kwin, PortalPath::Stream);
            return Decision {
                backend: CaptureBackend::Kwin,
                portal: PortalPath::Stream,
                source,
                forced: true,
                info: format!(
                    "capture route resolved: {name}, source {}, desktop {desktop_shown}",
                    source_name(source)
                ),
                forced_line: Some(format!(
                    "capture method forced: {name}; desktop detected: {desktop_shown}"
                )),
                mismatch: None,
                fallback: None,
            };
        }
        // Asked for and not there: the automatic route, with the reason.
        let mut decision = resolve(
            None,
            None,
            wayland,
            wayshot_ok,
            x11_ok,
            portal_ok,
            desktop,
            &KwinProbe::NotProbed,
            false,
        );
        let reason = match kwin {
            KwinProbe::Unavailable(reason) | KwinProbe::NotChecked(reason) => reason.as_str(),
            _ => "it was not probed",
        };
        decision.fallback = Some(format!(
            "KDE direct capture is not available ({reason}); using {} instead",
            method_name(decision.backend, decision.portal)
        ));
        return decision;
    }
    if chosen == Effective::Auto
        && kwin_default_on
        && *kwin == KwinProbe::Available
        && is_kde(desktop)
        && select_backend(None, wayland, wayshot_ok, x11_ok, portal_ok) == CaptureBackend::Portal
    {
        let name = method_name(CaptureBackend::Kwin, PortalPath::Stream);
        return Decision {
            backend: CaptureBackend::Kwin,
            portal: PortalPath::Stream,
            source: RouteSource::Automatic,
            forced: false,
            info: format!(
                "capture route resolved: {name}, source automatic, desktop {desktop_shown}"
            ),
            forced_line: None,
            mismatch: None,
            fallback: None,
        };
    }
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
                ForceWord::Kwin => "KDE direct",
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
        fallback: None,
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
        resolve(
            env,
            settings,
            wayland,
            wayshot,
            x11,
            portal,
            desktop,
            &KwinProbe::NotProbed,
            KWIN_DIRECT_DEFAULT_ON_KDE,
        )
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

    fn decide_kwin(
        env: Option<&str>,
        settings: Option<ForceWord>,
        wayshot: bool,
        desktop: &str,
        kwin: KwinProbe,
        default_on: bool,
    ) -> Decision {
        resolve(
            env, settings, true, wayshot, true, true, desktop, &kwin, default_on,
        )
    }

    #[test]
    fn kde_direct_words() {
        assert_eq!(classify("kwin"), ForceWord::Kwin);
        assert_eq!(classify(" KWin "), ForceWord::Kwin);
        assert_eq!(classify("kde-direct"), ForceWord::Kwin);
        assert_eq!(classify("kde"), ForceWord::Unknown);
        for kde in ["KDE", "plasma", "KWin", "ubuntu:KDE"] {
            assert!(is_kde(kde), "{kde}");
        }
        assert!(!is_kde("GNOME"));
        assert!(!is_kde("niri"));
    }

    #[test]
    fn kde_direct_is_used_when_asked_for_and_the_probe_answered() {
        let d = decide_kwin(
            None,
            Some(ForceWord::Kwin),
            false,
            "KDE",
            KwinProbe::Available,
            false,
        );
        assert_eq!(d.backend, CaptureBackend::Kwin);
        assert_eq!(d.source, RouteSource::Settings);
        assert!(d.forced);
        assert!(d.fallback.is_none());
        assert!(d.mismatch.is_none());
        assert!(d.info.contains("KDE direct"), "{}", d.info);
        assert!(d.forced_line.unwrap().contains("KDE direct"));
        let env = decide_kwin(
            Some("kwin"),
            Some(ForceWord::Stream),
            false,
            "KDE",
            KwinProbe::Available,
            false,
        );
        assert_eq!(env.backend, CaptureBackend::Kwin);
        assert_eq!(env.source, RouteSource::Environment);
    }

    #[test]
    fn kde_direct_that_failed_its_probe_falls_back_to_the_automatic_route() {
        // Plasma 6.7: KWin refuses callers without a .desktop grant.
        let d = decide_kwin(
            None,
            Some(ForceWord::Kwin),
            false,
            "KDE",
            KwinProbe::Unavailable("KWin refused the call".into()),
            false,
        );
        assert_eq!(d.backend, CaptureBackend::Portal);
        assert_eq!(d.portal, PortalPath::Stream);
        assert_eq!(d.source, RouteSource::Automatic);
        assert!(!d.forced, "the stream was not asked for");
        let line = d.fallback.unwrap();
        assert!(line.contains("KWin refused the call"), "{line}");
        assert!(line.contains("screen stream"), "{line}");
        // Not KDE at all: what automatic gives there.
        let niri = decide_kwin(
            Some("kwin"),
            None,
            true,
            "niri",
            KwinProbe::Unavailable("not KDE Plasma".into()),
            false,
        );
        assert_eq!(niri.backend, CaptureBackend::Wayshot);
        assert!(niri.fallback.unwrap().contains("using wayshot"));
        // A probe that never ran still says why.
        let unprobed = decide_kwin(
            None,
            Some(ForceWord::Kwin),
            false,
            "KDE",
            KwinProbe::NotProbed,
            false,
        );
        assert!(unprobed.fallback.unwrap().contains("not probed"));
        let skipped = decide_kwin(
            None,
            Some(ForceWord::Kwin),
            false,
            "KDE",
            KwinProbe::NotChecked("needs 6.8.1".into()),
            false,
        );
        assert_eq!(skipped.backend, CaptureBackend::Portal);
        assert!(skipped.fallback.unwrap().contains("needs 6.8.1"));
    }

    #[test]
    fn automatic_keeps_the_stream_on_kde_until_the_default_constant_flips() {
        let off = decide_kwin(None, None, false, "KDE", KwinProbe::Available, false);
        assert_eq!(off.backend, CaptureBackend::Portal);
        assert_eq!(off.portal, PortalPath::Stream);
        let on = decide_kwin(None, None, false, "KDE", KwinProbe::Available, true);
        assert_eq!(on.backend, CaptureBackend::Kwin);
        assert_eq!(on.source, RouteSource::Automatic);
        assert!(!on.forced);
        // Default on, probe failed: the stream, and no warning (nobody asked).
        let failed = decide_kwin(
            None,
            None,
            false,
            "KDE",
            KwinProbe::Unavailable("x".into()),
            true,
        );
        assert_eq!(failed.backend, CaptureBackend::Portal);
        assert!(failed.fallback.is_none());
        // Default on never takes over GNOME or a working wlroots capture.
        let gnome = decide_kwin(None, None, false, "GNOME", KwinProbe::Available, true);
        assert_eq!(gnome.backend, CaptureBackend::Portal);
        let wlr = decide_kwin(None, None, true, "KDE", KwinProbe::Available, true);
        assert_eq!(wlr.backend, CaptureBackend::Wayshot);
        // A forced stream stays the stream.
        let stream = decide_kwin(
            None,
            Some(ForceWord::Stream),
            false,
            "KDE",
            KwinProbe::Available,
            true,
        );
        assert_eq!(stream.backend, CaptureBackend::Portal);
    }

    #[test]
    fn older_screenshot_on_gnome_warns_about_the_flash_and_sound() {
        for (env, settings) in [
            (Some("portal-screenshot"), None),
            (None, Some(ForceWord::Screenshot)),
        ] {
            let d = decide(env, settings, true, false, false, true, "ubuntu:GNOME");
            assert_eq!(d.portal, PortalPath::Screenshot);
            let warn = d.mismatch.unwrap();
            assert!(warn.contains("flashes the screen"), "{warn}");
            assert!(warn.contains("sound"), "{warn}");
        }
        let kde = decide(
            None,
            Some(ForceWord::Screenshot),
            true,
            false,
            false,
            true,
            "KDE",
        );
        assert!(kde.mismatch.is_none());
        let gnome_stream = decide(
            None,
            Some(ForceWord::Stream),
            true,
            false,
            false,
            true,
            "GNOME",
        );
        assert!(gnome_stream.mismatch.is_none());
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
