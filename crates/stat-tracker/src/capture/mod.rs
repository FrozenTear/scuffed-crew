pub mod portal;
pub mod wayshot;
/// Native X11 capture (pure X11 sessions). Wired into selection by Lane C.
pub mod x11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Wayshot,
    X11,
    Portal,
    None,
}

/// Pure backend-selection policy (R1-6). Every env read and live probe happens
/// in `detect_backend`; this takes their already-normalized results so the exact
/// production decision is unit-testable without touching process env or an X
/// server. `detect_backend` routes through this — the tests below exercise the
/// real logic, not a parallel copy.
///
/// `force` is the lowercased `STAT_TRACKER_CAPTURE` value (or `None`). A forced
/// backend never silently falls back (R1-3): unavailable/unknown force → `None`.
fn select_backend(
    force: Option<&str>,
    wayshot_ok: bool,
    x11_ok: bool,
    portal_ok: bool,
) -> CaptureBackend {
    match force {
        Some("wayshot") | Some("wayland") => {
            if wayshot_ok {
                CaptureBackend::Wayshot
            } else {
                CaptureBackend::None
            }
        }
        Some("x11") => {
            if x11_ok {
                CaptureBackend::X11
            } else {
                CaptureBackend::None
            }
        }
        Some("portal") => {
            if portal_ok {
                CaptureBackend::Portal
            } else {
                CaptureBackend::None
            }
        }
        // Unknown forced value — fail closed, no fallback.
        Some(_) => CaptureBackend::None,
        // Automatic priority: usable Wayshot → X11 → Portal → None.
        None => {
            if wayshot_ok {
                CaptureBackend::Wayshot
            } else if x11_ok {
                CaptureBackend::X11
            } else if portal_ok {
                CaptureBackend::Portal
            } else {
                CaptureBackend::None
            }
        }
    }
}

pub async fn detect_backend() -> CaptureBackend {
    // STAT_TRACKER_CAPTURE override (R1-3, fail-closed). Env read + live probes
    // live here; the decision is delegated to `select_backend` so tests hit
    // production logic. Probing is lazy per force so a forced backend never
    // warms an unrelated connection.
    let force = std::env::var("STAT_TRACKER_CAPTURE")
        .ok()
        .map(|s| s.to_ascii_lowercase());

    match force.as_deref() {
        Some("wayshot") | Some("wayland") => {
            let ok = wayshot::probe().await.is_ok();
            let backend = select_backend(force.as_deref(), ok, false, false);
            if backend == CaptureBackend::None {
                tracing::warn!("STAT_TRACKER_CAPTURE=wayshot is not usable");
            }
            backend
        }
        Some("x11") => {
            let ok = x11::probe().await.is_ok();
            let backend = select_backend(force.as_deref(), false, ok, false);
            if backend == CaptureBackend::None {
                tracing::warn!("STAT_TRACKER_CAPTURE=x11 is not usable");
            }
            backend
        }
        Some("portal") => {
            let ok = portal::is_available().await;
            let backend = select_backend(force.as_deref(), false, false, ok);
            if backend == CaptureBackend::None {
                tracing::warn!("STAT_TRACKER_CAPTURE=portal not available");
            }
            backend
        }
        Some(other) => {
            tracing::warn!(%other, "unknown STAT_TRACKER_CAPTURE value");
            CaptureBackend::None
        }
        // Automatic probe order (R1-2): real connect + ≥1 output per candidate;
        // a failed candidate falls through. Short-circuit — a winning probe
        // skips the rest.
        None => {
            let wayshot_ok = wayshot::probe().await.is_ok();
            let x11_ok = !wayshot_ok && x11::probe().await.is_ok();
            let portal_ok = !wayshot_ok && !x11_ok && portal::is_available().await;
            let backend = select_backend(None, wayshot_ok, x11_ok, portal_ok);
            if backend == CaptureBackend::None {
                tracing::warn!("no capture backend available");
            }
            backend
        }
    }
}

pub async fn capture_screen_output(
    backend: &CaptureBackend,
    output_name: Option<&str>,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Wayshot => wayshot::capture_with_output(output_name).await,
        CaptureBackend::X11 => x11::capture_with_output(output_name).await,
        CaptureBackend::Portal => portal::capture().await,
        CaptureBackend::None => Err("no capture backend available".into()),
    }
}

/// List capture targets for the backend already selected by detect_backend.
/// Connection work must stay off the async/UI thread.
pub async fn list_outputs(
    backend: CaptureBackend,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Wayshot => {
            let result =
                tokio::task::spawn_blocking(|| wayshot::list_outputs().map_err(|e| e.to_string()))
                    .await?;
            result.map_err(Into::into)
        }
        // x11::list_outputs is already async + Send (spawn_blocking internally).
        CaptureBackend::X11 => x11::list_outputs().await,
        CaptureBackend::Portal | CaptureBackend::None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure selection-policy tests (R1-6). No `std::env::set_var` — process-global
    // env mutation races parallel test threads; we feed `select_backend` explicit
    // normalized inputs, which is exactly what `detect_backend` routes through.

    #[test]
    fn auto_priority_wayshot_first() {
        // Wayshot wins whenever usable, regardless of the others.
        assert_eq!(
            select_backend(None, true, true, true),
            CaptureBackend::Wayshot
        );
        assert_eq!(
            select_backend(None, true, false, false),
            CaptureBackend::Wayshot
        );
    }

    #[test]
    fn auto_failed_wayshot_falls_to_x11() {
        assert_eq!(select_backend(None, false, true, true), CaptureBackend::X11);
        assert_eq!(
            select_backend(None, false, true, false),
            CaptureBackend::X11
        );
    }

    #[test]
    fn auto_failed_x11_falls_to_portal() {
        assert_eq!(
            select_backend(None, false, false, true),
            CaptureBackend::Portal
        );
    }

    #[test]
    fn auto_none_available() {
        assert_eq!(
            select_backend(None, false, false, false),
            CaptureBackend::None
        );
    }

    #[test]
    fn forced_wayshot_available() {
        assert_eq!(
            select_backend(Some("wayshot"), true, false, false),
            CaptureBackend::Wayshot
        );
        // `wayland` is an accepted alias for the Wayshot force.
        assert_eq!(
            select_backend(Some("wayland"), true, false, false),
            CaptureBackend::Wayshot
        );
    }

    #[test]
    fn forced_x11_available() {
        assert_eq!(
            select_backend(Some("x11"), false, true, false),
            CaptureBackend::X11
        );
    }

    #[test]
    fn forced_portal_available() {
        assert_eq!(
            select_backend(Some("portal"), false, false, true),
            CaptureBackend::Portal
        );
    }

    #[test]
    fn forced_unavailable_never_falls_back() {
        // Each of the four force values, unavailable, must return None even when
        // other backends are usable — force means force (R1-3).
        assert_eq!(
            select_backend(Some("wayshot"), false, true, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some("wayland"), false, true, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some("x11"), true, false, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some("portal"), true, true, false),
            CaptureBackend::None
        );
    }

    #[test]
    fn unknown_force_is_none() {
        // Unknown value → None even with every backend available.
        assert_eq!(
            select_backend(Some("garbage"), true, true, true),
            CaptureBackend::None
        );
        assert_eq!(
            select_backend(Some(""), true, true, true),
            CaptureBackend::None
        );
    }
}
