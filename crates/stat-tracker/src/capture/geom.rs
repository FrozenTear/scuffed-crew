//! Output geometry: output and pixel rectangles, the portal crop, and
//! which output Auto captures.

/// The size to resize a portal cut-out to, or `None` to keep it. Only a
/// cut-out larger than the output's real mode is scaled down, and only when
/// it is more than 1px off in either direction. An unknown real size (0)
/// keeps the cut-out.
pub fn native_size(cut: PixelRect, real_width: u32, real_height: u32) -> Option<(u32, u32)> {
    if real_width == 0 || real_height == 0 {
        return None;
    }
    let off = |a: u32, b: u32| a.abs_diff(b) > 1;
    let larger = cut.width >= real_width && cut.height >= real_height;
    (larger && (off(cut.width, real_width) || off(cut.height, real_height)))
        .then_some((real_width, real_height))
}

/// One output's logical rectangle on the desktop, as xdg-output reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRect {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Real pixel size (the mode, rotation applied). 0 when unknown.
    pub real_width: u32,
    pub real_height: u32,
}

impl OutputRect {
    /// Pixel area for the Auto choice: the real size, else the logical one.
    pub(crate) fn area(&self) -> u64 {
        if self.real_width > 0 && self.real_height > 0 {
            u64::from(self.real_width) * u64::from(self.real_height)
        } else {
            u64::from(self.width) * u64::from(self.height)
        }
    }
}

/// A crop rectangle in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where in a whole-desktop portal image the chosen output sits. Logical
/// coordinates map to image pixels by the ratio of image size to the
/// logical desktop's bounding box, so a desktop rendered at scale 2 maps
/// each logical pixel to 2 image pixels. Returns the chosen output's index
/// in `outputs` and its rectangle. Auto (`None`) uses `auto_output_index`.
/// `Ok(None)` keeps the full image
/// (one output, no geometry, or a degenerate layout). A named output that
/// isn't there is an error, as it is for wayshot.
pub fn crop_rect(
    outputs: &[OutputRect],
    output_name: Option<&str>,
    img_width: u32,
    img_height: u32,
) -> Result<Option<(usize, PixelRect)>, Box<dyn std::error::Error + Send + Sync>> {
    if outputs.len() < 2 || img_width == 0 || img_height == 0 {
        return Ok(None);
    }
    let index = match output_name {
        Some(name) => outputs.iter().position(|o| o.name == name).ok_or_else(|| {
            let available: Vec<_> = outputs.iter().map(|o| o.name.as_str()).collect();
            format!(
                "output '{}' not found: available: {}",
                name,
                available.join(", ")
            )
        })?,
        None => auto_output_index(outputs.iter().map(|o| (o.name.as_str(), o.area()))).unwrap_or(0),
    };
    let output = &outputs[index];

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
    let total_w = max_x - min_x;
    let total_h = max_y - min_y;
    if total_w <= 0 || total_h <= 0 || output.width == 0 || output.height == 0 {
        return Ok(None);
    }

    let sx = f64::from(img_width) / total_w as f64;
    let sy = f64::from(img_height) / total_h as f64;
    let to_px = |v: i64, scale: f64, max: u32| -> u32 {
        ((v as f64 * scale).round().max(0.0) as u32).min(max)
    };
    let left = to_px(i64::from(output.x) - min_x, sx, img_width);
    let top = to_px(i64::from(output.y) - min_y, sy, img_height);
    let right = to_px(
        i64::from(output.x) + i64::from(output.width) - min_x,
        sx,
        img_width,
    );
    let bottom = to_px(
        i64::from(output.y) + i64::from(output.height) - min_y,
        sy,
        img_height,
    );
    if right <= left || bottom <= top {
        return Ok(None);
    }
    Ok(Some((
        index,
        PixelRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        },
    )))
}

/// Built-in panels (laptop screens). On Auto these lose to an external
/// monitor, since a player with a monitor plugged in plays on it.
fn is_builtin_panel(name: &str) -> bool {
    ["eDP", "LVDS", "DSI"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Which output Auto (no `capture_output`) captures, given each output's
/// name and real pixel area, in compositor order. One output is always that
/// output. With several, external monitors win over built-in panels (eDP,
/// LVDS, DSI), then the largest area wins, and a tie keeps list order.
/// `None` only for an empty list.
pub fn auto_output_index<'a>(outputs: impl IntoIterator<Item = (&'a str, u64)>) -> Option<usize> {
    let outputs: Vec<(&str, u64)> = outputs.into_iter().collect();
    if outputs.len() <= 1 {
        return if outputs.is_empty() { None } else { Some(0) };
    }
    let any_external = outputs.iter().any(|(name, _)| !is_builtin_panel(name));
    let mut best: Option<(usize, u64)> = None;
    for (i, (name, area)) in outputs.iter().enumerate() {
        if any_external && is_builtin_panel(name) {
            continue;
        }
        if best.is_none_or(|(_, best_area)| *area > best_area) {
            best = Some((i, *area));
        }
    }
    best.map(|(i, _)| i)
}
