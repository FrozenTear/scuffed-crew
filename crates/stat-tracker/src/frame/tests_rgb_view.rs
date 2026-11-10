//! [`super::rgb_view`]: no copy for an RGB frame.

use image::{DynamicImage, RgbImage, RgbaImage};

#[test]
fn an_rgb_frame_is_borrowed_not_copied() {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 32, image::Rgb([1, 2, 3])));
    let view = super::rgb_view(&img);
    assert!(matches!(view, std::borrow::Cow::Borrowed(_)));
    assert!(std::ptr::eq(
        view.as_raw().as_ptr(),
        img.as_rgb8().unwrap().as_raw().as_ptr()
    ));
}

#[test]
fn another_format_is_converted() {
    let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 2, image::Rgba([9, 8, 7, 255])));
    let view = super::rgb_view(&img);
    assert!(matches!(view, std::borrow::Cow::Owned(_)));
    assert_eq!(view.get_pixel(0, 0).0, [9, 8, 7]);
    assert_eq!(*view, img.to_rgb8());
}
