//! The poll tick hands its detectors a borrowed RGB view of the frame
//! (memfix PR 3); `DynamicImage::to_rgb8` would copy the whole frame on
//! every tick. The frame work itself is measured in the integration test
//! `snapshot_memory::the_poll_tick_and_the_report_blanking_do_not_copy_the_frame`.

#[test]
fn the_poll_tick_borrows_the_rgb_view_of_the_frame() {
    let src = include_str!("event_loop.rs");
    let prod = src.split("\n#[cfg(test)]").next().unwrap();
    assert!(prod.contains("let rgb = stat_tracker::frame::rgb_view(&img);"));
    assert!(
        !prod.contains(".to_rgb8()"),
        "a full-frame copy in the event loop"
    );
}
