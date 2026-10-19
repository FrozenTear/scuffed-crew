use image::DynamicImage;
use libwayshot::WayshotConnection;

pub fn is_available() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Real availability probe (R5-1, R1-2): a set `WAYLAND_DISPLAY` alone is intent,
/// not availability, so we cheap-check the env first, then connect and require
/// ≥1 output. Connection work runs on the blocking pool (Send-safe error), same
/// shape as `x11::probe`. R2-3: this warms one blocking-thread's thread-local
/// connection; other threads reconnect lazily, that is correct, leave it.
pub async fn probe() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !is_available() {
        return Err("WAYLAND_DISPLAY not set".into());
    }
    tokio::task::spawn_blocking(|| {
        with_wayshot(|wayshot| {
            if wayshot.get_all_outputs().is_empty() {
                return Err("Wayland probe: no outputs".into());
            }
            Ok(())
        })
    })
    .await?
}

pub fn list_outputs() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    with_wayshot(|wayshot| {
        let outputs = wayshot.get_all_outputs();
        Ok(outputs.iter().map(|o| o.name.clone()).collect())
    })
    .map_err(|e| -> Box<dyn std::error::Error> { e })
}

// One Wayshot connection per call, dropped when the call returns.
//
// libwayshot 0.7 creates a wl_shm pool and buffer (backed by a memfd) for
// every screenshot. On a long-lived connection the compositor kept those
// mappings alive: on niri a 2 hour session held about 8.7 GB of
// /memfd:libwayshot shmem across roughly 1,850 memfds owned by the tracker.
// Closing the connection makes the compositor free every object the client
// created, so a fresh connection per capture keeps compositor memory flat.
// The extra handshake costs a few milliseconds per capture.
fn with_wayshot<T>(
    f: impl FnOnce(&WayshotConnection) -> Result<T, Box<dyn std::error::Error + Send + Sync>>,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    let conn = WayshotConnection::new()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    let result = f(&conn);
    if let Err(ref err) = result {
        tracing::warn!(error = %err, "wayshot call failed");
    }
    drop(conn);
    result
}

pub async fn capture_with_output(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let target = output_name.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || {
        with_wayshot(|wayshot| {
            let outputs = wayshot.get_all_outputs();
            if outputs.is_empty() {
                return Err("no Wayland outputs found".into());
            }

            let output = if let Some(ref name) = target {
                outputs.iter().find(|o| o.name == *name).ok_or_else(|| {
                    let available: Vec<_> = outputs.iter().map(|o| o.name.as_str()).collect();
                    Box::<dyn std::error::Error + Send + Sync>::from(format!(
                        "output '{}' not found: available: {}",
                        name,
                        available.join(", ")
                    ))
                })?
            } else {
                outputs.first().ok_or_else(|| {
                    Box::<dyn std::error::Error + Send + Sync>::from("no Wayland outputs found")
                })?
            };

            tracing::debug!(output = %output.name, "capturing");
            // libwayshot returns the image from this call. The shm copy is
            // inside it, so there is no separate post-grab conversion to time.
            // X11 and the portal record that step as grab_convert.
            let img = wayshot.screenshot_single_output(output, false)?;
            tracing::debug!(
                output = %output.name,
                width = img.width(),
                height = img.height(),
                "screen captured via libwayshot"
            );
            Ok(img)
        })
    })
    .await?
}
