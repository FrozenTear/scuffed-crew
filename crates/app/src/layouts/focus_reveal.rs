//! Keep a focused control clear of fixed and sticky bars.
//!
//! The page scroller's `scroll-padding` tells the browser where the bars
//! are, but the two engines use it differently when focus moves:
//!
//! - Chromium scrolls any focused element that is not fully visible to the
//!   nearest padded edge.
//! - Firefox scrolls a newly focused element only when none of it is inside
//!   the padded view. An element that straddles a bar's edge counts as
//!   visible and stays where it is, partly under the bar.
//!
//! So after a focus move, a caller measures the clear band between its bars
//! and calls [`reveal_focus_between`]. When the focused box reaches past the
//! band, `scrollIntoView({block: "nearest"})` moves it the least distance,
//! and the scroller's own `scroll-padding` puts it just inside the band.
//! The shell runs this for every focus move in the page (top bar and phone
//! tab bar); a page with its own bar (the settings Save bar) can call the
//! same helper with that bar's edge.

/// Sub-pixel layout can leave a box a fraction of a pixel past an edge it
/// was scrolled to. That is not a cover, and must not scroll again.
const EDGE_SLACK_PX: f64 = 0.5;

/// Whether a focused box (`top`..`bottom`, viewport px) needs a scroll to sit
/// inside the clear band `clear_top`..`clear_bottom`. A box taller than the
/// band can never fit (a whole tab panel, `main` itself), so it is left
/// where it is rather than jumped to one end.
pub(crate) fn focus_needs_reveal(top: f64, bottom: f64, clear_top: f64, clear_bottom: f64) -> bool {
    if bottom - top > clear_bottom - clear_top {
        return false;
    }
    top < clear_top - EDGE_SLACK_PX || bottom > clear_bottom + EDGE_SLACK_PX
}

/// How far the page must scroll (viewport px, positive is down) so that a
/// focused box (`top`..`bottom`) sits inside the band
/// `clear_top`..`clear_bottom`, or `None` when it already does or can never
/// fit. The box then ends exactly on the edge it crossed. Whole pixels,
/// rounded away from the edge, so sub-pixel layout cannot leave it a
/// fraction under a bar.
pub(crate) fn reveal_scroll_delta(
    top: f64,
    bottom: f64,
    clear_top: f64,
    clear_bottom: f64,
) -> Option<f64> {
    if !focus_needs_reveal(top, bottom, clear_top, clear_bottom) {
        return None;
    }
    if bottom > clear_bottom + EDGE_SLACK_PX {
        Some((bottom - clear_bottom).ceil())
    } else {
        Some((top - clear_top).floor())
    }
}

/// Scroll the page by [`reveal_scroll_delta`], measured against the bars'
/// live boxes rather than the scroller's `scroll-padding`: a bar that grows
/// (the settings Save bar with its error line) is cleared at once, even
/// before any reserved space has caught up. Returns whether it scrolled.
#[cfg(target_arch = "wasm32")]
pub(crate) fn scroll_focus_into_band(
    focused: &web_sys::Element,
    clear_top: f64,
    clear_bottom: f64,
) -> bool {
    let rect = focused.get_bounding_client_rect();
    let Some(delta) = reveal_scroll_delta(rect.top(), rect.bottom(), clear_top, clear_bottom)
    else {
        return false;
    };
    match web_sys::window() {
        Some(window) => {
            window.scroll_by_with_x_and_y(0.0, delta);
            true
        }
        None => false,
    }
}

/// Scroll `focused` the least distance (`block: "nearest"`) when it is not
/// fully inside the band `clear_top`..`clear_bottom` (viewport px). Returns
/// whether it asked for a scroll.
#[cfg(target_arch = "wasm32")]
pub(crate) fn reveal_focus_between(
    focused: &web_sys::Element,
    clear_top: f64,
    clear_bottom: f64,
) -> bool {
    use wasm_bindgen::JsCast;

    let rect = focused.get_bounding_client_rect();
    if !focus_needs_reveal(rect.top(), rect.bottom(), clear_top, clear_bottom) {
        return false;
    }
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"block".into(), &"nearest".into());
    let _ = js_sys::Reflect::set(&options, &"inline".into(), &"nearest".into());
    let scroll = js_sys::Reflect::get(focused, &"scrollIntoView".into()).ok();
    match scroll
        .as_ref()
        .and_then(|f| f.dyn_ref::<js_sys::Function>())
    {
        Some(scroll) => scroll.call1(focused, &options).is_ok(),
        None => false,
    }
}

/// The shell's guard, run on every document `focusin`: a keyboard focus
/// stop in the page (`main` or the footer) is kept between the bottom of the
/// sticky top bar and the top of the phone tab bar (or the bottom of the
/// view, where the tab bar is not shown).
///
/// Only keyboard focus (`:focus-visible`, which also covers focus a script
/// moves after a key press): a mouse press focuses on `mousedown`, and a
/// scroll then would move the target out from under the pointer before the
/// click lands. The bars, the drawer and the open panels are outside `main`
/// and the footer, so they are never scrolled.
#[cfg(target_arch = "wasm32")]
pub(crate) fn keep_focus_clear_of_bars(evt: &web_sys::Event) {
    use wasm_bindgen::JsCast;

    let Some(focused) = evt
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    if !focused.matches(":focus-visible").unwrap_or(false) {
        return;
    }
    if focused
        .closest(PAGE_CONTENT_SELECTOR)
        .ok()
        .flatten()
        .is_none()
    {
        return;
    }
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(root) = document.document_element() else {
        return;
    };
    let view_bottom = f64::from(root.client_height());
    let bar_rect = |selector: &str| {
        document
            .query_selector(selector)
            .ok()
            .flatten()
            .map(|bar| bar.get_bounding_client_rect())
            // `display: none` (the tab bar off phone) has an empty box.
            .filter(|rect| rect.height() > 0.0)
    };
    let clear_top = bar_rect(TOP_BAR_SELECTOR).map_or(0.0, |rect| rect.bottom().max(0.0));
    let clear_bottom =
        bar_rect(TAB_BAR_SELECTOR).map_or(view_bottom, |rect| rect.top().min(view_bottom));
    reveal_focus_between(&focused, clear_top, clear_bottom);
}

/// Where a focus stop is part of the scrolling page, not of the chrome.
const PAGE_CONTENT_SELECTOR: &str = "main#main, footer.site-footer";
/// The sticky top bar.
pub(crate) const TOP_BAR_SELECTOR: &str = "header.d1-top";
/// The fixed tab bar, shown on phone only.
const TAB_BAR_SELECTOR: &str = "nav.site-tab-bar";

#[cfg(test)]
mod tests {
    use super::*;

    /// QA 696 nit 1, with the numbers QA measured: a footer link at 390x844
    /// that ends 3 to 13px under the phone tab bar (top at 799), and a patch
    /// note row head with 10px under the 64px top bar.
    #[test]
    fn a_box_that_straddles_a_bar_edge_is_revealed() {
        // Footer link 44px tall, bottom 13px under the tab bar.
        assert!(focus_needs_reveal(768.0, 812.0, 64.0, 799.0));
        // Only 3px under.
        assert!(focus_needs_reveal(758.0, 802.0, 64.0, 799.0));
        // Row head with its top 10px under the top bar.
        assert!(focus_needs_reveal(54.0, 133.0, 64.0, 799.0));
        // Past the fold at 1440x900 with no tab bar (the /blog card title).
        assert!(focus_needs_reveal(900.0, 919.0, 64.0, 900.0));
    }

    #[test]
    fn a_box_inside_the_band_is_left_alone() {
        assert!(!focus_needs_reveal(400.0, 444.0, 64.0, 799.0));
        // Exactly on both edges, and sub-pixel past them.
        assert!(!focus_needs_reveal(64.0, 799.0, 64.0, 799.0));
        assert!(!focus_needs_reveal(63.7, 755.0, 64.0, 799.0));
        assert!(!focus_needs_reveal(700.0, 799.4, 64.0, 799.0));
    }

    #[test]
    fn a_box_taller_than_the_band_is_never_jumped() {
        // A whole tab panel or `main`: it cannot fit, so no scroll.
        assert!(!focus_needs_reveal(300.0, 1500.0, 64.0, 799.0));
        assert!(!focus_needs_reveal(-200.0, 900.0, 64.0, 799.0));
    }

    /// QA 706 fix4, with QA's numbers: after a failed save the Save bar's
    /// top is at 699 at 390x844 (101px bar over the 45px tab bar), and the
    /// "About / ethos" box was left at 712-730. It moves up by exactly the
    /// part under the bar, so its bottom lands on the bar's top.
    #[test]
    fn the_scroll_delta_lands_the_box_on_the_edge_it_crossed() {
        assert_eq!(reveal_scroll_delta(712.0, 730.0, 64.0, 699.0), Some(31.0));
        // Sub-pixel: rounded away from the bar, never short of it.
        assert_eq!(reveal_scroll_delta(712.0, 729.6, 64.0, 699.0), Some(31.0));
        // Fully hidden under the bar (Firefox leaves it there).
        assert_eq!(reveal_scroll_delta(730.0, 748.0, 64.0, 699.0), Some(49.0));
        // Under the top bar: up, so its top lands on the bar's bottom.
        assert_eq!(reveal_scroll_delta(50.5, 94.0, 64.0, 699.0), Some(-14.0));
        // Inside the band, or too tall to fit: no scroll.
        assert_eq!(reveal_scroll_delta(400.0, 699.0, 64.0, 699.0), None);
        assert_eq!(reveal_scroll_delta(0.0, 900.0, 64.0, 699.0), None);
    }

    /// The guard measures the real bars by the classes they render with,
    /// acts only on keyboard focus, and only inside the page content.
    #[test]
    fn the_shell_guard_reads_the_bars_the_shell_renders() {
        let shell = include_str!("shell.rs");
        assert!(shell.contains("header {\n                class: \"d1-top\","));
        assert!(shell.contains("nav { class: \"site-tab-bar\","));
        assert!(shell.contains("footer { class: \"site-footer\","));
        assert!(shell.contains(
            "main {\n                class: \"site-main\",\n                id: MAIN_ID,"
        ));
        assert!(shell.contains("pub(crate) const MAIN_ID: &str = \"main\";"));

        let src = include_str!("focus_reveal.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let guard = prod
            .split("pub(crate) fn keep_focus_clear_of_bars(")
            .nth(1)
            .expect("guard fn");
        assert!(guard.contains("matches(\":focus-visible\")"));
        assert!(guard.contains(".closest(PAGE_CONTENT_SELECTOR)"));
        assert!(guard.contains("reveal_focus_between(&focused, clear_top, clear_bottom)"));
        let reveal = prod
            .split("pub(crate) fn reveal_focus_between(")
            .nth(1)
            .expect("reveal fn");
        assert!(reveal.contains("\"nearest\""));
        assert!(reveal.contains("focus_needs_reveal("));
    }
}
