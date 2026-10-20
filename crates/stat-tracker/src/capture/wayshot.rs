use image::DynamicImage;
use libwayshot::WayshotConnection;

pub fn is_available() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Real availability probe (R5-1, R1-2): a set `WAYLAND_DISPLAY` alone is intent,
/// not availability, so we cheap-check the env first, then connect and take one
/// real screenshot of the first output, which is discarded. Connection work runs
/// on the blocking pool (Send-safe error), same shape as `x11::probe`.
///
/// Requiring outputs alone is not enough: KDE Plasma exposes wl_outputs but no
/// wlroots screencopy (or ext image copy) protocol, so an output check passed
/// there and every real capture then failed with "Cannot find required wayland
/// protocol". A test screenshot fails on KDE instead, so auto-detect falls
/// through to the desktop portal. The probe runs once per detect, on its own
/// short-lived connection (see `with_wayshot`).
pub async fn probe() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !is_available() {
        return Err("WAYLAND_DISPLAY not set".into());
    }
    tokio::task::spawn_blocking(|| {
        with_wayshot(|wayshot| {
            let output = wayshot
                .get_all_outputs()
                .first()
                .ok_or("Wayland probe: no outputs")?;
            wayshot
                .screenshot_single_output(output, false)
                .map_err(describe)?;
            Ok(())
        })
    })
    .await?
}

// libwayshot's ProtocolNotFound prints only "Cannot find required wayland
// protocol" and drops which one, so add the name and what it means for the
// user (shown by the setup screen and "Capture now").
fn describe(err: libwayshot::Error) -> Box<dyn std::error::Error + Send + Sync> {
    match err {
        libwayshot::Error::ProtocolNotFound(name) => format!(
            "this desktop does not support wlroots screenshots (missing Wayland protocol {name}); \
             use the desktop portal instead (STAT_TRACKER_CAPTURE=portal)"
        )
        .into(),
        other => other.into(),
    }
}

pub fn list_outputs() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    with_wayshot(|wayshot| {
        let outputs = wayshot.get_all_outputs();
        Ok(outputs.iter().map(|o| o.name.clone()).collect())
    })
    .map_err(|e| -> Box<dyn std::error::Error> { e })
}

/// Output names and logical rectangles (xdg-output), for cropping portal
/// screenshots. KDE serves these even without screencopy.
pub fn output_rects()
-> Result<Vec<super::portal::OutputRect>, Box<dyn std::error::Error + Send + Sync>> {
    with_wayshot(|wayshot| {
        Ok(wayshot
            .get_all_outputs()
            .iter()
            .map(|o| {
                let r = o.logical_region.inner;
                let (real_width, real_height) = real_size(o);
                super::portal::OutputRect {
                    name: o.name.clone(),
                    x: r.position.x,
                    y: r.position.y,
                    width: r.size.width,
                    height: r.size.height,
                    real_width,
                    real_height,
                }
            })
            .collect())
    })
}

/// An output's real pixel size as it shows on screen: the mode size, with
/// width and height swapped for a rotated output. Falls back to the logical
/// size when the compositor sent no mode.
fn real_size(output: &libwayshot::output::OutputInfo) -> (u32, u32) {
    use libwayshot::reexport::Transform;
    let mode = output.physical_size;
    if mode.width == 0 || mode.height == 0 {
        let logical = output.logical_region.inner.size;
        return (logical.width, logical.height);
    }
    match output.transform {
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270 => {
            (mode.height, mode.width)
        }
        _ => (mode.width, mode.height),
    }
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
                // Auto: the external monitor, then the largest (see
                // `auto_output_index`). One output is that output, as before.
                let index = super::auto_output_index(outputs.iter().map(|o| {
                    let (w, h) = real_size(o);
                    (o.name.as_str(), u64::from(w) * u64::from(h))
                }))
                .ok_or_else(|| {
                    Box::<dyn std::error::Error + Send + Sync>::from("no Wayland outputs found")
                })?;
                &outputs[index]
            };

            tracing::debug!(output = %output.name, "capturing");
            let img = wayshot
                .screenshot_single_output(output, false)
                .map_err(describe)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_protocol_error_names_the_protocol_and_the_portal() {
        let err = describe(libwayshot::Error::ProtocolNotFound(
            "ZwlrScreencopyManagerV1".to_string(),
        ))
        .to_string();
        assert!(err.contains("ZwlrScreencopyManagerV1"), "{err}");
        assert!(err.contains("STAT_TRACKER_CAPTURE=portal"), "{err}");
    }

    #[test]
    fn other_errors_keep_their_libwayshot_text() {
        let err = describe(libwayshot::Error::NoOutputs).to_string();
        assert_eq!(err, "no outputs supplied");
    }
}
