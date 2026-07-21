//! Native X11 screen capture (pure X11 sessions).
//!
//! Selected only after a real X connection/output probe succeeds. Connection
//! work runs inside `spawn_blocking` to match wayshot. MVP: connect-per-call,
//! no connection cache (decision R3-2).
//!
//! This module is complete-but-unwired at PR-2; Lane C adds the enum arm.

use std::cmp::Ordering;

use image::{DynamicImage, RgbaImage};
use x11rb::connection::Connection;
use x11rb::image::{Image, PixelLayout};
use x11rb::protocol::randr::{Connection as RandrConnection, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat, VisualClass, Visualtype, Window};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Normalized RandR output descriptor (active CRTC only).
#[derive(Debug, Clone, PartialEq, Eq)]
struct X11Output {
    name: String,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    primary: bool,
}

pub async fn probe() -> Result<(), BoxError> {
    tokio::task::spawn_blocking(probe_blocking).await?
}

pub async fn list_outputs() -> Result<Vec<String>, BoxError> {
    tokio::task::spawn_blocking(list_outputs_blocking).await?
}

pub async fn capture_with_output(output_name: Option<&str>) -> Result<DynamicImage, BoxError> {
    let target = output_name.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || capture_blocking(target.as_deref())).await?
}

fn probe_blocking() -> Result<(), BoxError> {
    let outs = enumerate_outputs()?;
    if outs.is_empty() {
        return Err("X11 probe: no active RandR outputs".into());
    }
    Ok(())
}

fn list_outputs_blocking() -> Result<Vec<String>, BoxError> {
    let outs = enumerate_outputs()?;
    if outs.is_empty() {
        return Err("no X11 outputs found".into());
    }
    Ok(outs.into_iter().map(|o| o.name).collect())
}

fn capture_blocking(output_name: Option<&str>) -> Result<DynamicImage, BoxError> {
    let (conn, screen_num) = connect_x()?;
    let setup = conn.setup().clone();
    let screen = setup
        .roots
        .get(screen_num)
        .ok_or("X11: missing screen in setup")?;
    let root = screen.root;

    // Ensure RandR is present before enumeration (clearer error than a raw BadRequest).
    let _ = conn.randr_query_version(1, 3)?.reply()?;

    let outputs = enumerate_outputs_on(&conn, root)?;
    if outputs.is_empty() {
        return Err("no X11 outputs found".into());
    }

    let selected = select_output(&outputs, output_name)?;
    tracing::debug!(
        output = %selected.name,
        x = selected.x,
        y = selected.y,
        width = selected.width,
        height = selected.height,
        "capturing via X11 GetImage"
    );

    // TrueColor-only policy at root visual; reject other classes/depths with detail.
    let (visual, depth) = root_visual_info(screen)?;
    if visual.class != VisualClass::TRUE_COLOR {
        return Err(format!(
            "X11 capture rejects non-TrueColor root visual (class={:?}, depth={depth}, \
             red_mask={:#x}, green_mask={:#x}, blue_mask={:#x})",
            visual.class, visual.red_mask, visual.green_mask, visual.blue_mask
        )
        .into());
    }
    if depth != 24 && depth != 32 {
        return Err(format!(
            "X11 capture supports root depth 24 or 32 only (got depth={depth}, \
             red_mask={:#x}, green_mask={:#x}, blue_mask={:#x})",
            visual.red_mask, visual.green_mask, visual.blue_mask
        )
        .into());
    }

    let layout = PixelLayout::from_visual_type(visual).map_err(|e| {
        format!(
            "X11: malformed TrueColor visual masks (depth={depth}, \
             red_mask={:#x}, green_mask={:#x}, blue_mask={:#x}): {e:?}",
            visual.red_mask, visual.green_mask, visual.blue_mask
        )
    })?;

    let reply = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            selected.x,
            selected.y,
            selected.width,
            selected.height,
            !0,
        )?
        .reply()?;

    // bpp / pad from setup formats for the reply depth — for diagnostics on failure.
    let format = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == reply.depth)
        .ok_or_else(|| {
            format!(
                "X11 GetImage: no pixmap format for depth={} (byte_order={:?})",
                reply.depth, setup.image_byte_order
            )
        })?;

    if reply.depth != 24 && reply.depth != 32 {
        return Err(format!(
            "X11 GetImage unsupported depth={} bits_per_pixel={} byte_order={:?} \
             red_mask={:#x} green_mask={:#x} blue_mask={:#x}",
            reply.depth,
            format.bits_per_pixel,
            setup.image_byte_order,
            visual.red_mask,
            visual.green_mask,
            visual.blue_mask
        )
        .into());
    }

    let img = Image::get_from_reply(&setup, selected.width, selected.height, reply).map_err(
        |e| {
            format!(
                "X11: failed to wrap GetImage reply (depth={} bpp={} pad={} byte_order={:?}): {e:?}",
                format.depth,
                format.bits_per_pixel,
                format.scanline_pad,
                setup.image_byte_order
            )
        },
    )?;

    let rgba = image_to_rgba8(&img, layout, &visual, format.bits_per_pixel)?;
    let w = u32::from(selected.width);
    let h = u32::from(selected.height);
    let buf = RgbaImage::from_raw(w, h, rgba).ok_or_else(|| {
        format!(
            "X11: RGBA buffer size mismatch for {w}x{h} (depth={}, bpp={})",
            format.depth, format.bits_per_pixel
        )
    })?;

    tracing::debug!(
        output = %selected.name,
        width = w,
        height = h,
        "screen captured via X11"
    );
    Ok(DynamicImage::ImageRgba8(buf))
}

fn image_to_rgba8(
    img: &Image<'_>,
    layout: PixelLayout,
    visual: &Visualtype,
    bits_per_pixel: u8,
) -> Result<Vec<u8>, BoxError> {
    // Reject exotic bpp that we did not design for (spec: depth 24/32 TrueColor MVP).
    if bits_per_pixel != 24 && bits_per_pixel != 32 {
        return Err(format!(
            "X11 capture unsupported bits_per_pixel={bits_per_pixel} \
             (depth visual masks red={:#x} green={:#x} blue={:#x})",
            visual.red_mask, visual.green_mask, visual.blue_mask
        )
        .into());
    }

    let w = img.width();
    let h = img.height();
    let mut out = Vec::with_capacity(usize::from(w) * usize::from(h) * 4);
    for y in 0..h {
        for x in 0..w {
            let pixel = img.get_pixel(x, y);
            // decode → each channel expanded to 16-bit; take high byte for 8-bit RGB.
            let (r16, g16, b16) = layout.decode(pixel);
            out.push((r16 >> 8) as u8);
            out.push((g16 >> 8) as u8);
            out.push((b16 >> 8) as u8);
            // image::reencode does NOT set alpha — force opaque (Task 0 API notes).
            out.push(0xFF);
        }
    }
    Ok(out)
}

fn connect_x() -> Result<(x11rb::rust_connection::RustConnection, usize), BoxError> {
    x11rb::connect(None).map_err(|e| -> BoxError {
        format!("X11 connection failed (is DISPLAY set and usable?): {e}").into()
    })
}

fn enumerate_outputs() -> Result<Vec<X11Output>, BoxError> {
    let (conn, screen_num) = connect_x()?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or("X11: missing screen in setup")?
        .root;
    let _ = conn.randr_query_version(1, 3)?.reply()?;
    enumerate_outputs_on(&conn, root)
}

fn enumerate_outputs_on(conn: &impl Connection, root: Window) -> Result<Vec<X11Output>, BoxError> {
    let res = conn.randr_get_screen_resources_current(root)?.reply()?;
    let primary = conn.randr_get_output_primary(root)?.reply()?.output;

    let mut outs = Vec::new();
    for &output_id in &res.outputs {
        let info = conn
            .randr_get_output_info(output_id, res.config_timestamp)?
            .reply()?;
        // Active = Connected + non-zero CRTC (claude Task0 API notes).
        if info.connection != RandrConnection::CONNECTED || info.crtc == 0 {
            continue;
        }
        let crtc = conn
            .randr_get_crtc_info(info.crtc, res.config_timestamp)?
            .reply()?;
        if crtc.width == 0 || crtc.height == 0 {
            continue;
        }
        let name = if info.name.is_empty() {
            // Runtime-stable only (R3-5) — XIDs change across server restarts.
            format!("x11-output-{output_id}")
        } else {
            String::from_utf8_lossy(&info.name).into_owned()
        };
        outs.push(X11Output {
            name,
            x: crtc.x,
            y: crtc.y,
            width: crtc.width,
            height: crtc.height,
            primary: output_id == primary,
        });
    }

    sort_outputs(&mut outs);
    Ok(outs)
}

fn sort_outputs(outs: &mut [X11Output]) {
    // Stable order: primary first, then name, then geometry.
    outs.sort_by(|a, b| match (a.primary, b.primary) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a
            .name
            .cmp(&b.name)
            .then(a.x.cmp(&b.x))
            .then(a.y.cmp(&b.y))
            .then(a.width.cmp(&b.width))
            .then(a.height.cmp(&b.height)),
    });
}

/// Pure selection helper — unit-tested without an X server (Task 4 Step 2).
fn select_output<'a>(
    outputs: &'a [X11Output],
    configured: Option<&str>,
) -> Result<&'a X11Output, BoxError> {
    if outputs.is_empty() {
        return Err("no X11 outputs found".into());
    }
    if let Some(name) = configured {
        return outputs.iter().find(|o| o.name == name).ok_or_else(|| {
            let available: Vec<_> = outputs.iter().map(|o| o.name.as_str()).collect();
            format!(
                "output '{name}' not found — available: {}",
                available.join(", ")
            )
            .into()
        });
    }
    // Unconfigured → primary active; else first in stable order.
    if let Some(primary) = outputs.iter().find(|o| o.primary) {
        return Ok(primary);
    }
    outputs
        .first()
        .ok_or_else(|| -> BoxError { "no X11 outputs found".into() })
}

fn root_visual_info(
    screen: &x11rb::protocol::xproto::Screen,
) -> Result<(Visualtype, u8), BoxError> {
    let id = screen.root_visual;
    for depth in &screen.allowed_depths {
        if let Some(visual) = depth.visuals.iter().find(|v| v.visual_id == id) {
            return Ok((*visual, depth.depth));
        }
    }
    Err(format!("X11: root visual {id} not found in allowed_depths").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(name: &str, primary: bool, x: i16) -> X11Output {
        X11Output {
            name: name.into(),
            x,
            y: 0,
            width: 1920,
            height: 1080,
            primary,
        }
    }

    #[test]
    fn select_missing_configured_lists_available() {
        let outs = vec![out("DP-1", true, 0), out("HDMI-1", false, 1920)];
        let err = select_output(&outs, Some("eDP-1")).unwrap_err().to_string();
        assert!(err.contains("eDP-1"), "{err}");
        assert!(err.contains("DP-1"), "{err}");
        assert!(err.contains("HDMI-1"), "{err}");
    }

    #[test]
    fn select_unconfigured_prefers_primary() {
        let outs = vec![out("HDMI-1", false, 0), out("DP-1", true, 1920)];
        // Note: real enumerate sorts primary first; selection still finds primary.
        let sel = select_output(&outs, None).unwrap();
        assert_eq!(sel.name, "DP-1");
    }

    #[test]
    fn select_unconfigured_no_primary_uses_first_stable() {
        let mut outs = vec![out("HDMI-1", false, 100), out("DP-1", false, 0)];
        sort_outputs(&mut outs);
        // After sort by name: DP-1 then HDMI-1
        assert_eq!(outs[0].name, "DP-1");
        let sel = select_output(&outs, None).unwrap();
        assert_eq!(sel.name, "DP-1");
    }

    #[test]
    fn select_exact_match() {
        let outs = vec![out("DP-1", true, 0), out("HDMI-1", false, 1920)];
        let sel = select_output(&outs, Some("HDMI-1")).unwrap();
        assert_eq!(sel.name, "HDMI-1");
    }

    #[test]
    fn sort_primary_first_then_name() {
        let mut outs = vec![
            out("HDMI-1", false, 0),
            out("DP-1", true, 0),
            out("eDP-1", false, 0),
        ];
        sort_outputs(&mut outs);
        assert_eq!(outs[0].name, "DP-1");
        assert!(outs[0].primary);
        assert_eq!(outs[1].name, "HDMI-1");
        assert_eq!(outs[2].name, "eDP-1");
    }

    #[test]
    fn empty_name_fallback_format() {
        // Pure unit: the format used when RandR name is empty.
        let xid: u32 = 0x42;
        let label = format!("x11-output-{xid}");
        assert_eq!(label, "x11-output-66");
    }

    #[test]
    fn select_empty_errors() {
        let outs: Vec<X11Output> = vec![];
        assert!(select_output(&outs, None).is_err());
        assert!(select_output(&outs, Some("x")).is_err());
    }
}
