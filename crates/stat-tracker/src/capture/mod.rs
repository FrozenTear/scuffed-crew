pub mod portal;
pub mod wayshot;

#[derive(Debug)]
pub enum CaptureBackend {
    Wayshot,
    Portal,
    None,
}

pub async fn detect_backend() -> CaptureBackend {
    if wayshot::is_available() {
        CaptureBackend::Wayshot
    } else if portal::is_available().await {
        CaptureBackend::Portal
    } else {
        tracing::warn!("no capture backend available");
        CaptureBackend::None
    }
}

pub async fn capture_screen_output(
    backend: &CaptureBackend,
    output_name: Option<&str>,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Wayshot => wayshot::capture_with_output(output_name).await,
        CaptureBackend::Portal => portal::capture().await,
        CaptureBackend::None => Err("no capture backend available".into()),
    }
}
