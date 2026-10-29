//! The poll tick hands its detectors a borrowed RGB view of the frame
//! (memfix PR 3); `DynamicImage::to_rgb8` would copy the whole frame on
//! every tick. The frame work itself is measured in the integration tests
//! `snapshot_memory::the_poll_tick_and_the_report_blanking_do_not_copy_the_frame`
//! and `snapshot_memory::the_poll_tick_on_a_words_pack_does_not_copy_the_frame`.

#[test]
fn the_poll_tick_borrows_the_rgb_view_of_the_frame() {
    let src = include_str!("event_loop.rs");
    // Everything before a test module; a single `#[cfg(test)]` helper in
    // the middle of the file does not end the production code.
    let prod = src.split("\n#[cfg(test)]\nmod ").next().unwrap();
    assert!(prod.contains("pub(crate) async fn finish_within_shutdown"));
    assert!(prod.contains("let rgb = stat_tracker::frame::rgb_view(&img);"));
    assert!(
        !prod.contains(".to_rgb8()"),
        "a full-frame copy in the event loop"
    );
}
