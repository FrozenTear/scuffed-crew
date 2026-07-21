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

pub async fn detect_backend() -> CaptureBackend {
    // Task 5 adds the STAT_TRACKER_CAPTURE force block above this.
    // Automatic probe order (R1-2): each candidate must really connect + enumerate
    // ≥1 output; a failed candidate falls through. Short-circuit — a winning probe
    // skips the rest. XWayland → usable Wayshot still wins; pure X11 → X11.
    if wayshot::probe().await.is_ok() {
        return CaptureBackend::Wayshot;
    }
    if x11::probe().await.is_ok() {
        return CaptureBackend::X11;
    }
    if portal::is_available().await {
        return CaptureBackend::Portal;
    }
    tracing::warn!("no capture backend available");
    CaptureBackend::None
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
