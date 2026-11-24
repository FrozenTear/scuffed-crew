//! Fitting a stream frame to the chosen output, and the debug compare
//! pair.

use std::path::Path;

use image::DynamicImage;

use crate::capture::geom::{self, OutputRect};
use crate::capture::{outputs, portal};

pub(super) async fn fit_stream(
    img: DynamicImage,
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    // The geometry read's CPU is measured on its blocking thread (zero
    // when the cache answered). A layout that cannot explain this frame is
    // read again.
    let (outputs, geometry_cpu) =
        outputs::output_geometry_for_timed(img.width(), img.height(), output_name).await;
    let cpu_started = crate::perf::thread_cpu();
    let fitted = fit_stream_image(img, &outputs, output_name);
    if fitted.is_ok() {
        crate::perf::record_cpu(
            crate::perf::Stage::Capture,
            geometry_cpu.saturating_add(crate::perf::thread_cpu().saturating_sub(cpu_started)),
        );
    }
    fitted
}

pub fn fit_stream_image(
    img: DynamicImage,
    outputs: &[OutputRect],
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    if outputs.is_empty() {
        return Ok(img);
    }
    let index = match output_name {
        Some(name) => outputs
            .iter()
            .position(|output| output.name == name)
            .ok_or_else(|| format!("output '{name}' not found"))?,
        None => geom::auto_output_index(
            outputs
                .iter()
                .map(|output| (output.name.as_str(), output.area())),
        )
        .unwrap_or(0),
    };
    let chosen = &outputs[index];
    if stream_should_crop(img.width(), img.height(), chosen.area(), outputs.len()) {
        let Some((_, rect)) = geom::crop_rect(outputs, output_name, img.width(), img.height())?
        else {
            return Ok(img);
        };
        return Ok(img.crop_imm(rect.x, rect.y, rect.width, rect.height));
    }
    Ok(img)
}

/// Crop only when the image is clearly the whole desktop: at least two
/// outputs, and the image area is more than 1.2 times the chosen output.
pub fn stream_should_crop(img_w: u32, img_h: u32, chosen_area: u64, outputs: usize) -> bool {
    outputs >= 2
        && chosen_area > 0
        && u64::from(img_w).saturating_mul(u64::from(img_h)) > chosen_area.saturating_mul(6) / 5
}

pub(super) async fn save_compare(
    data_dir: &Path,
    index: u32,
    stream: &DynamicImage,
    output_name: Option<&str>,
) {
    let dir = data_dir.join("debug").join("portal-compare");
    if std::fs::create_dir_all(&dir).is_err() {
        tracing::warn!("could not create debug/portal-compare");
        return;
    }
    crate::fs_mode::tighten_private_dir(&dir);
    let n = index + 1;
    let stream_path = dir.join(format!("{n:04}-stream.png"));
    let shot_path = dir.join(format!("{n:04}-screenshot.png"));
    if stream.save(&stream_path).is_err() {
        tracing::warn!("could not save a screencast debug frame");
        return;
    }
    crate::fs_mode::tighten_private_file(&stream_path);
    // KDE direct first: it is tens of milliseconds, so it is nearer the
    // stream frame's moment than the portal screenshot after it.
    // Boxed so the probe and KWin grab do not sit inline in every stream
    // grab's future (debug-only path, large state).
    Box::pin(crate::capture::kwin::save_compare_half(
        &dir,
        n,
        output_name,
    ))
    .await;
    match portal::capture(output_name).await {
        Ok(shot) => {
            if shot.save(&shot_path).is_ok() {
                crate::fs_mode::tighten_private_file(&shot_path);
                tracing::info!(
                    pair = n,
                    "saved a portal compare pair under debug/portal-compare"
                );
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "screenshot half of the portal compare pair failed");
        }
    }
}
