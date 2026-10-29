//! Shared frame pixels that both readers (the cv reader and Tesseract), the
//! detectors and the privacy blanker use: board geometry (`geometry`), row
//! and team-size layout (`layout`) and the poll-tick stability gate
//! (`stability`). Nothing here reads text, calls Tesseract or touches packs:
//! production code here names only `frame` itself, the core modules
//! (`stat-tracker-core`) and outside crates (`tests_layering` checks this).
//!
//! By convention, declare any `#[cfg(test)] mod …;` at the bottom of this
//! file, after the production modules.

pub mod geometry;
pub mod layout;
pub mod stability;

use std::borrow::Cow;

use image::{DynamicImage, RgbImage};

/// The frame as RGB, borrowed when it already is RGB (every stream frame is:
/// `capture::stream::framebuf::to_dynamic`), converted otherwise.
/// `DynamicImage::to_rgb8` always copies, a full frame (11 to 15 MB) per
/// poll tick.
pub fn rgb_view(img: &DynamicImage) -> Cow<'_, RgbImage> {
    match img.as_rgb8() {
        Some(rgb) => Cow::Borrowed(rgb),
        None => Cow::Owned(img.to_rgb8()),
    }
}

#[cfg(test)]
mod tests_rgb_view;
