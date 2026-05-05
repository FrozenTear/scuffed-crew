use image::DynamicImage;

pub async fn is_available() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok()
}

pub async fn capture() -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let response = ashpd::desktop::screenshot::ScreenshotRequest::default()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .map_err(|e| format!("portal screenshot request failed: {e}"))?
        .response()
        .map_err(|e| format!("portal screenshot response failed: {e}"))?;

    let uri = response.uri().to_string();
    let path = uri
        .strip_prefix("file://")
        .ok_or_else(|| format!("unexpected screenshot URI scheme: {uri}"))?;

    let img = image::open(path)
        .map_err(|e| format!("failed to open portal screenshot at {path}: {e}"))?;

    // Clean up the temporary file
    let _ = std::fs::remove_file(path);

    tracing::debug!(
        width = img.width(),
        height = img.height(),
        "screen captured via XDG Desktop Portal"
    );

    Ok(img)
}
