use std::cell::RefCell;

use image::DynamicImage;
use libwayshot::WayshotConnection;

pub fn is_available() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Real availability probe (R5-1, R1-2): a set `WAYLAND_DISPLAY` alone is intent,
/// not availability, so we cheap-check the env first, then connect and require
/// ≥1 output. Connection work runs on the blocking pool (Send-safe error), same
/// shape as `x11::probe`. R2-3: this warms one blocking-thread's thread-local
/// connection; other threads reconnect lazily — that is correct, leave it.
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

// Thread-local Wayshot connection. Capture runs on `spawn_blocking` pool
// threads; reusing the connection avoids compositor handshake + output
// enumeration on every 4s poll tick / Tab.
thread_local! {
    static WAYSHOT: RefCell<Option<WayshotConnection>> = const { RefCell::new(None) };
}

fn ensure_conn(
    slot: &mut Option<WayshotConnection>,
) -> Result<&WayshotConnection, Box<dyn std::error::Error + Send + Sync>> {
    if slot.is_none() {
        *slot = Some(
            WayshotConnection::new()
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?,
        );
    }
    Ok(slot.as_ref().expect("just inserted"))
}

fn with_wayshot<T>(
    f: impl FnOnce(&WayshotConnection) -> Result<T, Box<dyn std::error::Error + Send + Sync>>,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    WAYSHOT.with(|cell| {
        let mut slot = cell.borrow_mut();
        let first = {
            let conn = ensure_conn(&mut slot)?;
            f(conn)
        };
        match first {
            Ok(v) => Ok(v),
            Err(first_err) => {
                // Drop the stale connection; the next call reconnects lazily via
                // ensure_conn. Always surface the original capture/list error —
                // do not reconnect here (wasted work) or replace first_err with
                // a reconnect failure.
                tracing::warn!(error = %first_err, "wayshot call failed — dropping connection");
                *slot = None;
                Err(first_err)
            }
        }
    })
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
                        "output '{}' not found — available: {}",
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
