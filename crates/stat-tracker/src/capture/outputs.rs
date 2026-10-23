//! The Wayland output-geometry cache that the portal crop, KDE direct and
//! the stream fit read.

use super::geom::OutputRect;

/// Output names for the monitor picker. Empty when the geometry can't be
/// read, which leaves the picker on Auto and the full image in use. Always
/// read fresh, and the cache takes the answer.
pub async fn list_outputs() -> Vec<String> {
    invalidate_output_geometry();
    output_rects().await.into_iter().map(|o| o.name).collect()
}

/// Output rectangles, from the cache while it is fresh. Stream open uses
/// this; its CPU is not capture CPU.
pub(crate) async fn output_geometry() -> Vec<OutputRect> {
    output_rects().await
}

/// Output rectangles for an image of `img_width` x `img_height` that is one
/// output or the whole desktop, and the thread CPU of any read this made
/// (none when the cache answered), for a grab that counts it as capture
/// CPU. When the cached layout cannot explain that image, or does not have
/// `output_name`, the outputs changed: the cache is dropped and read again
/// once.
pub(crate) async fn output_geometry_for_timed(
    img_width: u32,
    img_height: u32,
    output_name: Option<&str>,
) -> (Vec<OutputRect>, std::time::Duration) {
    let (outputs, cpu) = output_rects_timed().await;
    if !geometry_looks_stale(&outputs, output_name, img_width, img_height) {
        return (outputs, cpu);
    }
    tracing::debug!(
        img_width,
        img_height,
        "output geometry does not match the captured image; reading it again"
    );
    invalidate_output_geometry();
    let (outputs, again) = output_rects_timed().await;
    (outputs, cpu.saturating_add(again))
}

/// [`output_geometry`] and the thread CPU of the read, for a stream grab,
/// which counts that CPU as capture CPU. Zero when the cache answered.
pub(crate) async fn output_geometry_timed() -> (Vec<OutputRect>, std::time::Duration) {
    output_rects_timed().await
}

/// How long read output geometry is reused. `output_rects` opens a Wayland
/// connection, which used to happen on every grab. A change of outputs
/// inside this window is caught by [`geometry_looks_stale`] or an error.
pub const GEOMETRY_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// An empty answer (no Wayland, or the read failed) is kept this long only.
pub const GEOMETRY_EMPTY_TTL: std::time::Duration = std::time::Duration::from_secs(5);

struct GeometryCache {
    outputs: Vec<OutputRect>,
    read_at: Option<std::time::Instant>,
}

static GEOMETRY: std::sync::Mutex<GeometryCache> = std::sync::Mutex::new(GeometryCache {
    outputs: Vec::new(),
    read_at: None,
});

/// True when outputs read at `read_at` may still be used at `now`.
pub fn geometry_fresh(
    read_at: Option<std::time::Instant>,
    empty: bool,
    now: std::time::Instant,
) -> bool {
    let ttl = if empty {
        GEOMETRY_EMPTY_TTL
    } else {
        GEOMETRY_TTL
    };
    read_at.is_some_and(|at| now.saturating_duration_since(at) < ttl)
}

/// Drop the cached geometry. A grab that failed in a way an output change
/// explains calls this, and so does the monitor picker.
pub(crate) fn invalidate_output_geometry() {
    GEOMETRY
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .read_at = None;
}

/// True when `outputs` cannot be the layout this image came from: a named
/// output is missing, or the image is neither one output's real size nor
/// the whole logical desktop at one scale (2% slack, as compositors round).
/// No outputs is nothing to check.
pub fn geometry_looks_stale(
    outputs: &[OutputRect],
    output_name: Option<&str>,
    img_width: u32,
    img_height: u32,
) -> bool {
    if outputs.is_empty() || img_width == 0 || img_height == 0 {
        return false;
    }
    if let Some(name) = output_name
        && !outputs.iter().any(|o| o.name == name)
    {
        return true;
    }
    let near = |a: u32, b: u32| a.abs_diff(b) <= 2;
    let one_output = outputs.iter().any(|o| {
        (near(img_width, o.real_width) && near(img_height, o.real_height))
            || (near(img_width, o.width) && near(img_height, o.height))
    });
    if one_output {
        return false;
    }
    let min_x = outputs.iter().map(|o| i64::from(o.x)).min().unwrap_or(0);
    let min_y = outputs.iter().map(|o| i64::from(o.y)).min().unwrap_or(0);
    let max_x = outputs
        .iter()
        .map(|o| i64::from(o.x) + i64::from(o.width))
        .max()
        .unwrap_or(0);
    let max_y = outputs
        .iter()
        .map(|o| i64::from(o.y) + i64::from(o.height))
        .max()
        .unwrap_or(0);
    let (w, h) = (max_x - min_x, max_y - min_y);
    if w <= 0 || h <= 0 {
        return false;
    }
    let sx = f64::from(img_width) / w as f64;
    let sy = f64::from(img_height) / h as f64;
    (sx - sy).abs() / sx.max(sy) > 0.02
}

async fn output_rects() -> Vec<OutputRect> {
    output_rects_timed().await.0
}

/// The output rectangles from the cache while it is fresh (no CPU), else
/// read, kept, and the thread CPU the read took on its blocking thread.
async fn output_rects_timed() -> (Vec<OutputRect>, std::time::Duration) {
    let now = std::time::Instant::now();
    {
        let cache = GEOMETRY.lock().unwrap_or_else(|err| err.into_inner());
        if geometry_fresh(cache.read_at, cache.outputs.is_empty(), now) {
            return (cache.outputs.clone(), std::time::Duration::ZERO);
        }
    }
    let (outputs, cpu) = read_output_rects_timed().await;
    let mut cache = GEOMETRY.lock().unwrap_or_else(|err| err.into_inner());
    cache.outputs = outputs.clone();
    cache.read_at = Some(std::time::Instant::now());
    (outputs, cpu)
}

/// Reads the output rectangles, and the thread CPU the read took on its
/// blocking thread.
async fn read_output_rects_timed() -> (Vec<OutputRect>, std::time::Duration) {
    if !super::wayshot::is_available() {
        return (Vec::new(), std::time::Duration::ZERO);
    }
    let read = tokio::task::spawn_blocking(|| {
        let cpu_started = crate::perf::thread_cpu();
        let outputs = super::wayshot::output_rects();
        (
            outputs,
            crate::perf::thread_cpu().saturating_sub(cpu_started),
        )
    })
    .await;
    match read {
        Ok((Ok(outputs), cpu)) => (outputs, cpu),
        Ok((Err(e), cpu)) => {
            tracing::debug!(error = %e, "could not read output geometry for the portal");
            (Vec::new(), cpu)
        }
        Err(e) => {
            tracing::debug!(error = %e, "output geometry task failed");
            (Vec::new(), std::time::Duration::ZERO)
        }
    }
}
