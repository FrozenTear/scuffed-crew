use image::DynamicImage;
use libwayshot::WayshotConnection;

pub fn is_available() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

pub fn list_outputs() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let wayshot = WayshotConnection::new()?;
    let outputs = wayshot.get_all_outputs();
    Ok(outputs.iter().map(|o| o.name.clone()).collect())
}

pub async fn capture_with_output(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let target = output_name.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || {
        let wayshot = WayshotConnection::new()?;
        let outputs = wayshot.get_all_outputs();
        if outputs.is_empty() {
            return Err("no Wayland outputs found".into());
        }

        let output = if let Some(ref name) = target {
            outputs
                .iter()
                .find(|o| o.name == *name)
                .ok_or_else(|| {
                    let available: Vec<_> = outputs.iter().map(|o| o.name.as_str()).collect();
                    format!(
                        "output '{}' not found — available: {}",
                        name,
                        available.join(", ")
                    )
                })?
        } else {
            &outputs[0]
        };

        tracing::debug!(output = %output.name, "capturing");
        let img = wayshot.screenshot_single_output(output, false)?;
        tracing::debug!(
            output = %output.name,
            width = img.width(),
            height = img.height(),
            "screen captured via libwayshot"
        );
        Ok(img)
    })
    .await?
}

