use image::DynamicImage;
use libwayshot::WayshotConnection;

pub fn is_available() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

pub async fn capture() -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    tokio::task::spawn_blocking(|| {
        let wayshot = WayshotConnection::new()?;
        let outputs = wayshot.get_all_outputs();
        if outputs.is_empty() {
            return Err("no Wayland outputs found".into());
        }
        let img = wayshot.screenshot_single_output(&outputs[0], false)?;
        tracing::debug!(
            width = img.width(),
            height = img.height(),
            "screen captured via libwayshot"
        );
        Ok(img)
    })
    .await?
}
