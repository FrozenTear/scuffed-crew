//! The one site shell (Direction 1): skip link, grouped top bar with a mega
//! menu, account menu, phone tab bar and drawer, main landmark and footer.
//!
//! The public, admin and strategy layouts all render through [`SiteShell`],
//! so every page keeps the same chrome.

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::SiteSettings;

use super::nav::{
    NavContext, NavEntry, NavGroupId, Section, account_entries, aria_current, changelog_entry,
    drawer_admin_entries, guest_entries, is_current, is_tool_route, nav_groups,
    section_aria_current, section_of, shows_admin_entry,
};
#[cfg(target_arch = "wasm32")]
use super::use_dom_listener;
use super::{focus_element, use_document_keydown};
use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use crate::state::{loaded_site_settings, use_site_settings};
use crate::theme::ThemeToggle;

/// The main landmark. The skip link targets it.
pub(crate) const MAIN_ID: &str = "main";
const ACCOUNT_TOGGLE_ID: &str = "nav-account-toggle";
const ACCOUNT_PANEL_ID: &str = "nav-account-panel";
const DRAWER_TOGGLE_ID: &str = "site-drawer-toggle";
const DRAWER_ID: &str = "site-drawer";
const DRAWER_CLOSE_ID: &str = "site-drawer-close";

/// The one thing in the shell that can be open. Opening one closes the others.
///
/// These are disclosures (a button with `aria-expanded` and `aria-controls`
/// plus a panel of links), not ARIA menus: the panels hold plain links that
/// Tab walks through, so they carry no `role="menu"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disclosure {
    Group(NavGroupId),
    Account,
    Drawer,
}

impl Disclosure {
    /// The button that opened it. Focus goes back here on close.
    fn toggle_id(self) -> &'static str {
        match self {
            Disclosure::Group(group) => group.toggle_id(),
            Disclosure::Account => ACCOUNT_TOGGLE_ID,
            Disclosure::Drawer => DRAWER_TOGGLE_ID,
        }
    }

    fn panel_id(self) -> &'static str {
        match self {
            Disclosure::Group(group) => group.panel_id(),
            Disclosure::Account => ACCOUNT_PANEL_ID,
            Disclosure::Drawer => DRAWER_ID,
        }
    }
}

/// Pressing a toggle: open it, or close it when it is the open one.
fn toggled(current: Option<Disclosure>, target: Disclosure) -> Option<Disclosure> {
    if current == Some(target) {
        None
    } else {
        Some(target)
    }
}

/// `aria-expanded` for the toggle of `target`.
fn expanded(current: Option<Disclosure>, target: Disclosure) -> &'static str {
    if current == Some(target) {
        "true"
    } else {
        "false"
    }
}

/// The page behind the open drawer is `inert`, so Tab, clicks and screen
/// readers stay inside the dialog.
///
/// `Option<&str>`, never a bool: Dioxus 0.7.9 web writes `inert: false` as the
/// string `"false"`, and `inert` is not in the interpreter's boolean attribute
/// list, so `inert="false"` would stay on the element and block the page.
/// `None` removes the attribute.
fn background_inert(current: Option<Disclosure>) -> Option<&'static str> {
    (current == Some(Disclosure::Drawer)).then_some("true")
}

/// Set by a page while it shows a modal of its own (the strategy editor's
/// map picker). The modal lives inside `main`, so `main` stays live and the
/// page marks its own content inert; the shell makes its chrome inert (skip
/// link, top bar, sub-nav, footer, tab bar), so Tab, clicks and screen
/// readers stay in the modal. See [`use_page_modal`].
#[derive(Clone, Copy)]
pub(crate) struct PageModal(Signal<bool>);

/// The main landmark's mounted handle. Focus through it works on web and in
/// the desktop webview alike (`focus_element` is `web_sys`, wasm only).
#[derive(Clone, Copy)]
struct MainHandle(Signal<Option<std::rc::Rc<MountedData>>>);

/// A callback that moves focus to `main#main` (the editor's Esc out of the
/// map canvas). Call it from an event handler: it spawns the focus call.
pub(crate) fn use_focus_main() -> impl FnMut() + Copy {
    let handle = try_use_context::<MainHandle>();
    move || {
        let Some(MainHandle(mounted)) = handle else {
            return;
        };
        if let Some(main) = mounted.peek().clone() {
            spawn(async move {
                let _ = main.set_focus(true).await;
            });
        }
    }
}

/// `inert` for the shell chrome: behind the drawer, or behind a page modal.
/// Same `Option<&str>` rule as [`background_inert`].
fn chrome_inert(current: Option<Disclosure>, page_modal: bool) -> Option<&'static str> {
    (current == Some(Disclosure::Drawer) || page_modal).then_some("true")
}

/// Tell the shell whether this page's modal is open. Cleared when the page
/// unmounts, so a modal never leaves the chrome inert after navigation.
pub(crate) fn use_page_modal(open: bool) {
    let modal = try_use_context::<PageModal>();
    use_effect(use_reactive((&open,), move |(open,)| {
        if let Some(PageModal(mut flag)) = modal
            && *flag.peek() != open
        {
            flag.set(open);
        }
    }));
    use_drop(move || {
        if let Some(PageModal(mut flag)) = modal
            && let Ok(mut flag) = flag.try_write()
        {
            *flag = false;
        }
    });
}

/// Where Tab goes when it would leave the drawer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrapEdge {
    First,
    Last,
}

/// The drawer's focus trap. `None` lets the browser move focus as usual.
fn tab_trap_target(shift: bool, inside: bool, at_first: bool, at_last: bool) -> Option<TrapEdge> {
    if shift {
        (!inside || at_first).then_some(TrapEdge::Last)
    } else {
        (!inside || at_last).then_some(TrapEdge::First)
    }
}

fn trap_tab_in_drawer(evt: &web_sys::KeyboardEvent) {
    trap_tab_within(DRAWER_ID, evt);
}

/// Keep Tab and Shift+Tab inside the element `container_id` (a modal): from
/// the last stop Tab wraps to the first, from the first Shift+Tab wraps to
/// the last, and from anywhere outside focus is pulled back in. The drawer
/// and the editor's map picker share it.
#[cfg(target_arch = "wasm32")]
pub(crate) fn trap_tab_within(container_id: &str, evt: &web_sys::KeyboardEvent) {
    use wasm_bindgen::JsCast;

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(drawer) = document.get_element_by_id(container_id) else {
        return;
    };
    let Ok(stops) = drawer.query_selector_all("a[href], button:not([disabled])") else {
        return;
    };
    let count = stops.length();
    if count == 0 {
        return;
    }
    let stop = |index: u32| {
        stops
            .item(index)
            .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
    };
    let (first, last) = (stop(0), stop(count - 1));
    let active = document.active_element();
    let is_active = |el: &Option<web_sys::HtmlElement>| match (el, &active) {
        (Some(el), Some(active)) => el.is_same_node(Some(active)),
        _ => false,
    };
    let inside = active
        .as_ref()
        .is_some_and(|active| drawer.contains(Some(active)));
    let target = match tab_trap_target(evt.shift_key(), inside, is_active(&first), is_active(&last))
    {
        Some(TrapEdge::First) => first,
        Some(TrapEdge::Last) => last,
        None => return,
    };
    if let Some(target) = target {
        evt.prevent_default();
        let _ = target.focus();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn trap_tab_within(_container_id: &str, _evt: &web_sys::KeyboardEvent) {}

/// The phone layout's media query, the same one `SHELL_CSS` uses. The drawer
/// exists only while it matches.
#[cfg(any(test, target_arch = "wasm32"))]
const PHONE_QUERY: &str = "(max-width: 820px)";

/// Each disclosure belongs to one layout. The drawer is a phone control:
/// when the viewport grows past the phone breakpoint (a rotated tablet, a
/// resized window) it closes, so it never sits over the desktop layout with
/// the page inert and its toggle hidden. A group panel or the account menu
/// is a desktop control: entering the phone layout closes it, so its
/// full-screen dismiss layer does not stay behind a hidden button.
#[cfg(any(test, target_arch = "wasm32"))]
fn closes_on_resize(current: Option<Disclosure>, phone_layout: bool) -> bool {
    match current {
        Some(Disclosure::Drawer) => !phone_layout,
        Some(Disclosure::Group(_) | Disclosure::Account) => phone_layout,
        None => false,
    }
}

/// A group panel or the account menu closes when focus moves to something
/// outside it (Tab past its last link, Shift+Tab before its button). The
/// drawer traps focus instead, so it is not closed this way.
#[cfg(any(test, target_arch = "wasm32"))]
fn closes_on_focus_move(current: Option<Disclosure>, focus_inside: bool) -> bool {
    matches!(current, Some(Disclosure::Group(_) | Disclosure::Account)) && !focus_inside
}

/// The phone media query, as an event target for its `change` event.
#[cfg(target_arch = "wasm32")]
fn phone_query_target() -> Option<web_sys::EventTarget> {
    let query = web_sys::window()?.match_media(PHONE_QUERY).ok()??;
    Some(query.into())
}

#[cfg(target_arch = "wasm32")]
fn document_target() -> Option<web_sys::EventTarget> {
    web_sys::window()?.document().map(Into::into)
}

/// True when the node that took focus sits in the open disclosure: its
/// toggle and its panel share one wrapper element.
#[cfg(target_arch = "wasm32")]
fn focus_is_inside(current: Disclosure, evt: &web_sys::Event) -> bool {
    use wasm_bindgen::JsCast;

    let wrapper = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(current.toggle_id()))
        .and_then(|toggle| toggle.parent_element());
    let focused = evt
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Node>().ok());
    match (wrapper, focused) {
        (Some(wrapper), Some(focused)) => wrapper.contains(Some(&focused)),
        // Cannot tell: leave the panel open.
        _ => true,
    }
}

// `use_signal` runs its initializer once. Tests set `STARTS_OPEN` so the real
// shell mounts with one disclosure open; production always starts closed.
#[cfg(test)]
thread_local! {
    static STARTS_OPEN: std::cell::Cell<Option<Disclosure>> = const { std::cell::Cell::new(None) };
}

fn initial_disclosure() -> Option<Disclosure> {
    #[cfg(test)]
    {
        STARTS_OPEN.with(|slot| slot.get())
    }
    #[cfg(not(test))]
    {
        None
    }
}

#[derive(Clone, Copy)]
struct ShellState {
    open: Signal<Option<Disclosure>>,
    /// Element to focus after the next render. Closing the drawer has to
    /// wait for that render: until then its toggle sits in the inert page.
    refocus: Signal<Option<&'static str>>,
}

impl ShellState {
    fn toggle(self, target: Disclosure) {
        let mut open = self.open;
        let next = toggled(open(), target);
        open.set(next);
    }

    /// Close whatever is open. `return_focus` sends focus back to its toggle
    /// (Escape, the Close button, a click outside). A followed link passes
    /// `false`: focus belongs to the new page.
    fn close(self, return_focus: bool) {
        let mut open = self.open;
        let mut refocus = self.refocus;
        // `peek`: an effect that closes must not start following `open`.
        let Some(current) = *open.peek() else {
            return;
        };
        open.set(None);
        if return_focus {
            refocus.set(Some(current.toggle_id()));
        }
    }
}

impl ShellState {
    /// Close whatever is open and, after the next render (once nothing is
    /// inert), focus `id`.
    #[cfg(any(test, target_arch = "wasm32"))]
    fn close_to(self, id: &'static str) {
        let mut refocus = self.refocus;
        if self.open.peek().is_none() {
            return;
        }
        self.close(false);
        refocus.set(Some(id));
    }
}

fn use_shell_state() -> ShellState {
    let open = use_signal(initial_disclosure);
    let mut refocus = use_signal(|| Option::<&'static str>::None);
    // Effects run after the DOM has the new render, so the target is no
    // longer inert (drawer toggle) and the drawer is on screen (Close).
    use_effect(move || {
        if let Some(id) = refocus() {
            focus_element(id);
            refocus.set(None);
        }
    });
    use_effect(move || {
        if open() == Some(Disclosure::Drawer) {
            focus_element(DRAWER_CLOSE_ID);
        }
    });
    let state = ShellState { open, refocus };

    // Off wasm these two listeners are not installed (see `use_dom_listener`).
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;

        // `change` fires when the viewport crosses the phone breakpoint.
        use_dom_listener(phone_query_target, "change", move |evt| {
            let phone_layout = evt
                .target()
                .and_then(|target| target.dyn_into::<web_sys::MediaQueryList>().ok())
                .is_none_or(|query| query.matches());
            let current = *state.open.peek();
            if closes_on_resize(current, phone_layout) {
                // The toggle is hidden in the new layout, so focus goes to
                // the main landmark instead of falling to the body.
                state.close_to(MAIN_ID);
            }
        });
        use_dom_listener(document_target, "focusin", move |evt| {
            let Some(current) = *state.open.peek() else {
                return;
            };
            if closes_on_focus_move(Some(current), focus_is_inside(current, &evt)) {
                state.close(false);
            }
        });
    }
    state
}

fn sign_out(mut auth: Signal<AuthState>) {
    spawn(async move {
        let _ = ApiClient::web().logout().await;
        auth.set(AuthState {
            user: None,
            loading: false,
        });
    });
}

fn strategies_enabled_or_default(settings: Option<&SiteSettings>) -> bool {
    settings.map(|s| s.strategies_enabled).unwrap_or(true)
}

/// Two characters for the avatar when there is no picture.
fn user_initials(username: &str) -> String {
    let initials: String = username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if initials.is_empty() {
        "?".to_string()
    } else {
        initials
    }
}

/// Only same-site paths and https pictures reach the `src` attribute.
fn safe_avatar_url(url: Option<&str>) -> Option<String> {
    let url = url?.trim();
    (url.starts_with('/') || url.starts_with("https://")).then(|| url.to_string())
}

/// `aria-current` for a `Link` that stands for a section (a phone tab, the
/// Admin entry): `page` on its own page, `true` elsewhere in the section.
/// Always passed, `page` included: see [`aria_current`] for why `None`
/// there would remove `Link`'s own value in the browser.
fn section_link_current(to: &Route, current: &Route, in_section: bool) -> Option<&'static str> {
    if to == current {
        Some("page")
    } else {
        section_aria_current(in_section)
    }
}

fn entry_class(entry: &NavEntry, current: &Route) -> &'static str {
    if is_current(&entry.route, current) {
        "is-active"
    } else {
        ""
    }
}

const SHELL_CSS: &str = r#"
    /* WCAG 2.4.11: Tab, the skip link and in-page jumps scroll the page, so
       the page keeps a focused control clear of the sticky top bar (and of
       the phone tab bar below). Fixed on the page scroller once, for every page. */
    html {
        scroll-padding-top: calc(var(--nav-height-used) + var(--space-2));
    }
    .skip-link {
        position: fixed;
        top: var(--space-2);
        left: var(--space-3);
        z-index: 200;
        transform: translateY(-300%);
        display: inline-flex;
        align-items: center;
        height: var(--hit-target-min);
        padding: 0 var(--space-3);
        background: var(--skip-link-bg);
        color: var(--skip-link-fg);
        border-radius: var(--radius-control);
        font-family: var(--font-head);
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
        text-decoration: none;
    }
    .skip-link:focus {
        transform: none;
        outline: var(--focus-ring-width) solid var(--focus-ring);
        outline-offset: var(--focus-ring-offset);
    }
    .site-app {
        /* What the shell takes from the viewport height: the top bar, plus
           the tab bar on phone. Full-height tools subtract it. */
        --shell-chrome-height: var(--nav-height-used);
        min-height: 100vh;
        display: flex;
        flex-direction: column;
    }
    .site-main {
        min-width: 0;
        flex: 1 0 auto;
    }
    .site-main:focus {
        outline: none;
    }

    /* Top bar */
    .d1-top {
        position: sticky;
        top: 0;
        z-index: 100;
        /* Exactly the token, border included: pages subtract it. */
        box-sizing: border-box;
        height: var(--nav-height-used);
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: 0 var(--space-page);
        background: var(--bg);
        border-bottom: 1px solid var(--nav-border);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        color: var(--nav-fg);
    }
    .d1-home-link {
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        min-height: var(--hit-target-min);
        color: var(--nav-fg);
        text-decoration: none;
        flex: 0 0 auto;
    }
    .d1-mark {
        width: var(--control-size-sm);
        height: var(--control-size-sm);
        border-radius: var(--nav-mark-radius);
        background: var(--nav-mark-bg);
        color: var(--nav-mark-fg);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-xs);
        letter-spacing: var(--font-head-tracking);
        flex: 0 0 auto;
    }
    .d1-mark.is-pending {
        background: var(--surface-sunken);
    }
    .d1-brand {
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        font-size: var(--nav-font-size);
        color: var(--nav-fg);
        white-space: nowrap;
    }
    .d1-groups {
        display: flex;
        align-items: stretch;
        align-self: stretch;
        min-width: 0;
        margin-left: var(--space-2);
    }
    .d1-group {
        position: relative;
        display: flex;
    }
    .d1-group-btn {
        min-height: var(--hit-target-min);
        padding: 0 var(--space-3);
        background: transparent;
        border: 0;
        border-bottom: var(--nav-active-underline-width) solid transparent;
        color: var(--nav-muted);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        display: inline-flex;
        align-items: center;
        gap: var(--space-1);
        cursor: pointer;
    }
    .d1-group-btn:hover {
        color: var(--nav-fg);
    }
    .d1-group.is-current .d1-group-btn,
    .d1-group.is-open .d1-group-btn {
        color: var(--accent-secondary-text);
        border-bottom-color: var(--accent-secondary-text);
    }
    .d1-mega {
        display: none;
        position: absolute;
        top: 100%;
        left: 0;
        min-width: calc(var(--space-8) * 8);
        max-height: calc(100vh - var(--nav-height-used) - var(--space-4));
        overflow-y: auto;
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-card);
        box-shadow: var(--shadow-overlay);
        padding: var(--space-2);
        z-index: 110;
    }
    .d1-group.is-open .d1-mega {
        display: block;
    }
    .d1-mega a {
        display: flex;
        align-items: center;
        min-height: var(--hit-target-min);
        padding: 0 var(--space-3);
        border-radius: var(--nav-radius);
        color: var(--nav-fg);
        font-size: var(--nav-font-size);
        text-decoration: none;
    }
    .d1-mega a.is-active,
    .d1-mega a:hover {
        background: var(--accent-soft);
        color: var(--accent-secondary-text);
    }
    .d1-actions {
        margin-left: auto;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        flex: 0 0 auto;
    }
    .d1-btn {
        height: var(--button-height);
        min-height: var(--hit-target-min);
        min-width: var(--hit-target-min);
        padding: 0 var(--button-pad-x);
        border-radius: var(--button-radius);
        border: 1px solid var(--border-strong);
        background: transparent;
        color: var(--text);
        font-family: var(--button-font);
        font-size: var(--button-font-size);
        font-weight: var(--button-font-weight);
        letter-spacing: var(--font-head-tracking);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        text-decoration: none;
        white-space: nowrap;
    }
    .d1-btn.is-primary {
        background: var(--button-primary-bg);
        color: var(--button-primary-fg);
        border-color: var(--button-primary-border);
    }
    .d1-session-pending {
        color: var(--nav-muted);
    }
    .theme-toggle {
        width: var(--control-size-sm);
        height: var(--control-size-sm);
        min-width: var(--control-size-sm);
        min-height: var(--control-size-sm);
        padding: 0;
        border-radius: var(--radius-control);
        border: 1px solid var(--text-3);
        background: var(--surface);
        color: var(--text);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
        cursor: pointer;
    }
    .theme-toggle:hover {
        background: var(--surface-sunken);
    }

    /* Signed in: Admin entry and the account menu */
    .btn-admin {
        height: var(--control-size-sm);
        min-height: var(--control-size-sm);
        padding: 0 var(--space-3);
        border-radius: var(--radius-control);
        border: 1px solid var(--border-strong);
        background: var(--surface);
        color: var(--text);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        display: inline-flex;
        align-items: center;
        text-decoration: none;
    }
    .btn-admin.is-current {
        color: var(--accent-secondary-text);
        border-color: var(--accent-text);
    }
    .account-wrap {
        position: relative;
    }
    .account-btn {
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        height: var(--control-size-sm);
        min-height: var(--control-size-sm);
        padding: 0 var(--space-2) 0 0;
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-control);
        background: var(--surface);
        color: var(--text);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        cursor: pointer;
    }
    .account-btn.is-open {
        border-color: var(--accent-text);
        color: var(--accent-secondary-text);
    }
    .account-name {
        max-width: calc(var(--space-8) * 4);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        padding-right: var(--space-2);
    }
    .account-avatar {
        width: var(--control-size-sm);
        height: var(--control-size-sm);
        border-radius: 50%;
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border-strong);
        display: inline-flex;
        align-items: center;
        justify-content: center;
        overflow: hidden;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        font-size: var(--text-xs);
        flex: 0 0 auto;
    }
    /* The circle shares the button's left edge instead of sitting inside it. */
    .account-btn .account-avatar {
        margin: -1px 0 -1px -1px;
    }
    .account-avatar img {
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
    }
    .account-menu {
        display: none;
        position: absolute;
        top: calc(100% + var(--space-2));
        right: 0;
        min-width: calc(var(--space-8) * 8);
        max-height: calc(100vh - var(--nav-height-used) - var(--space-4));
        overflow-y: auto;
        background: var(--surface);
        color: var(--text);
        border: 1px solid var(--border-strong);
        border-radius: var(--radius-card);
        box-shadow: var(--shadow-overlay);
        padding: var(--space-2);
        z-index: 110;
    }
    .account-wrap.is-open .account-menu {
        display: block;
    }
    .account-menu a,
    .account-menu button.account-item {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
        box-sizing: border-box;
        width: 100%;
        min-height: var(--hit-target-min);
        padding: 0 var(--space-3);
        border: 0;
        border-radius: var(--nav-radius);
        background: transparent;
        color: var(--nav-fg);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        text-align: left;
        text-decoration: none;
        cursor: pointer;
    }
    .account-menu a:hover,
    .account-menu a.is-active,
    .account-menu button.account-item:hover {
        background: var(--accent-soft);
        color: var(--accent-secondary-text);
    }
    /* Catches a click outside an open group or account menu. It sits under
       the top bar, so the bar's own controls still take their clicks. */
    .nav-dismiss-layer {
        position: fixed;
        inset: 0;
        z-index: 90;
    }

    /* Admin sub-nav, under the top bar on admin pages */
    .admin-subnav {
        display: flex;
        gap: var(--space-1);
        overflow-x: auto;
        scrollbar-width: thin;
        scrollbar-color: var(--border-strong) transparent;
        background: var(--surface);
        border-bottom: 1px solid var(--nav-border);
        padding: 0 var(--space-page);
        min-height: var(--hit-target-min);
    }
    .admin-subnav a {
        display: inline-flex;
        align-items: center;
        flex: 0 0 auto;
        min-height: var(--hit-target-min);
        padding: 0 var(--space-3);
        color: var(--nav-muted);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        white-space: nowrap;
        text-decoration: none;
        border-bottom: var(--nav-active-underline-width) solid transparent;
    }
    .admin-subnav a:hover {
        color: var(--nav-fg);
    }
    .admin-subnav a.is-active {
        color: var(--accent-secondary-text);
        border-bottom-color: var(--accent-secondary-text);
    }
    .admin-subnav a:focus-visible {
        outline-offset: calc(var(--focus-ring-width) * -1);
    }

    /* Phone drawer: a solid --bg panel, never see-through */
    .site-drawer {
        display: none;
    }
    .site-drawer.is-open {
        display: flex;
        flex-direction: column;
        position: fixed;
        inset: 0;
        z-index: 120;
        background: var(--bg);
        color: var(--text);
        padding: var(--space-page);
        padding-bottom: calc(var(--space-page) + env(safe-area-inset-bottom, 0px));
        overflow-y: auto;
        overscroll-behavior: contain;
    }
    .drawer-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-3);
        min-height: var(--hit-target-min);
        margin-bottom: var(--space-4);
    }
    .drawer-close {
        min-width: var(--hit-target-min);
        min-height: var(--hit-target-min);
        padding: 0 var(--space-2);
        border-radius: var(--radius-control);
        border: 1px solid var(--border-strong);
        background: var(--surface);
        color: var(--text);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        cursor: pointer;
    }
    .site-drawer h2 {
        margin: var(--space-4) 0 var(--space-2);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        color: var(--text-subtle);
    }
    .site-drawer a,
    .site-drawer .drawer-link {
        display: flex;
        align-items: center;
        flex: 0 0 auto;
        width: 100%;
        min-height: var(--hit-target-min);
        padding: 0;
        background: transparent;
        border: 0;
        border-bottom: 1px solid var(--border);
        color: var(--text);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        text-align: left;
        text-decoration: none;
        cursor: pointer;
    }
    .site-drawer a.is-active {
        color: var(--accent-secondary-text);
    }
    .drawer-theme {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        margin-top: var(--space-4);
        color: var(--text-3);
        font-size: var(--text-sm);
    }
    .site-drawer .theme-toggle {
        width: var(--hit-target-min);
        height: var(--hit-target-min);
    }

    /* Phone tab bar */
    .site-tab-bar {
        display: none;
    }
    .site-tab-bar a,
    .site-tab-bar button {
        min-height: var(--hit-target-min);
        min-width: var(--hit-target-min);
        background: transparent;
        border: 0;
        color: var(--nav-muted);
        font-family: var(--nav-font);
        font-size: var(--nav-font-size);
        font-weight: var(--nav-font-weight);
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-1);
        padding: var(--space-1);
        text-decoration: none;
        cursor: pointer;
    }
    .site-tab-bar a.is-active,
    .site-tab-bar button.is-active {
        color: var(--accent-secondary-text);
    }
    .site-tab-bar a:focus-visible,
    .site-tab-bar button:focus-visible {
        outline-offset: calc(var(--focus-ring-width) * -1);
    }

    /* Footer */
    .site-footer {
        border-top: 1px solid var(--nav-border);
        background: var(--surface);
        color: var(--text-3);
        padding: var(--space-section) var(--space-page);
    }
    .footer-inner {
        max-width: var(--content-max-default);
        margin: 0 auto;
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: var(--space-section);
    }
    .footer-col h2 {
        margin: 0 0 var(--space-3);
        font-family: var(--font-head);
        font-size: var(--text-xs);
        font-weight: var(--font-label-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        color: var(--text);
    }
    .footer-col a,
    .footer-col .footer-link {
        display: flex;
        align-items: center;
        width: 100%;
        min-height: var(--hit-target-min);
        padding: 0;
        background: transparent;
        border: 0;
        color: var(--text-3);
        font-family: inherit;
        font-size: var(--text-sm);
        text-align: left;
        text-decoration: none;
        cursor: pointer;
    }
    .footer-col a:hover,
    .footer-col .footer-link:hover,
    .footer-col a.is-active {
        color: var(--link);
    }
    .footer-note {
        max-width: var(--content-max-default);
        margin: var(--space-section) auto 0;
        font-size: var(--text-sm);
        color: var(--text-subtle);
    }

    /* Optical alignment for UI labels: trim the font's own top and bottom
       space so a label with no descenders sits centred in its control. */
    .d1-btn,
    .d1-group-btn,
    .d1-mark,
    .d1-brand,
    .d1-mega a,
    .btn-admin,
    .account-btn,
    .account-menu a,
    .account-menu button.account-item,
    .admin-subnav a,
    .drawer-close,
    .site-drawer a,
    .site-drawer .drawer-link,
    .site-tab-bar a,
    .site-tab-bar button,
    .footer-col a,
    .footer-col .footer-link {
        line-height: var(--leading-ui);
        text-box-trim: trim-both;
        text-box-edge: cap alphabetic;
    }
    @supports not (text-box-trim: trim-both) {
        .d1-btn,
        .d1-group-btn,
        .d1-mark,
        .btn-admin,
        .drawer-close,
        .site-tab-bar a,
        .site-tab-bar button {
            padding-top: 0.08em;
        }
    }

    @media (max-width: 820px) {
        .d1-top {
            padding-right: var(--space-2);
        }
        .d1-groups,
        .d1-top .d1-brand,
        .d1-btn,
        .btn-admin,
        .account-name {
            display: none;
        }
        .d1-mark,
        .d1-top .theme-toggle {
            width: var(--hit-target-min);
            height: var(--hit-target-min);
            min-width: var(--hit-target-min);
            min-height: var(--hit-target-min);
        }
        .account-btn {
            width: var(--hit-target-min);
            height: var(--hit-target-min);
            min-width: var(--hit-target-min);
            min-height: var(--hit-target-min);
            padding: 0;
            justify-content: center;
            border: 0;
            border-radius: var(--radius-pill);
            background: transparent;
        }
        .account-btn.is-open {
            outline: var(--focus-ring-width) solid var(--focus-ring);
            outline-offset: var(--focus-ring-offset);
        }
        .account-btn .account-avatar {
            margin: 0;
            width: var(--hit-target-min);
            height: var(--hit-target-min);
            font-size: var(--text-sm);
        }
        .account-menu {
            position: fixed;
            top: calc(var(--nav-height-used) + var(--space-2));
            left: var(--space-page);
            right: var(--space-page);
            min-width: 0;
        }
        .admin-subnav {
            padding: 0 var(--space-2);
        }
        html {
            /* The tab bar is one hit target plus its 1px top border. Set on
               the root so the page scroller and the shell read one value. */
            --tab-bar-height: calc(var(--hit-target-min) + 1px + env(safe-area-inset-bottom, 0px));
            scroll-padding-bottom: calc(var(--tab-bar-height) + var(--space-2));
        }
        .site-app {
            --shell-chrome-height: calc(var(--nav-height-used) + var(--tab-bar-height));
            padding-bottom: var(--tab-bar-height);
        }
        .site-tab-bar {
            display: grid;
            grid-template-columns: repeat(5, minmax(0, 1fr));
            position: fixed;
            left: 0;
            right: 0;
            bottom: 0;
            z-index: 100;
            background: var(--bg);
            border-top: 1px solid var(--border-strong);
            min-height: var(--hit-target-min);
            padding-bottom: env(safe-area-inset-bottom, 0px);
        }
        .footer-inner {
            grid-template-columns: 1fr 1fr;
            gap: var(--space-4);
        }
    }
    /* Narrow phones: five tabs in 360px leave 72px each, and "Community" at
       the nav size is wider than that in Firefox. */
    @media (max-width: 380px) {
        .site-tab-bar a,
        .site-tab-bar button {
            font-size: var(--text-xs);
            padding-left: 0;
            padding-right: 0;
        }
    }
"#;

/// The site chrome around a routed page.
///
/// `subnav` is rendered between the top bar and the main landmark (the admin
/// sub-nav). `children` is the page, inside `<main id="main">`.
#[component]
pub(crate) fn SiteShell(subnav: Option<Element>, children: Element) -> Element {
    let state = use_shell_state();
    let page_modal = use_context_provider(|| PageModal(Signal::new(false)));
    let MainHandle(mut main_mounted) = use_context_provider(|| MainHandle(Signal::new(None)));
    let auth = use_auth();
    let route = use_route::<Route>();

    // Any route change closes what is open: a followed link, and also the
    // browser's Back and Forward, which no click handler sees. Focus is left
    // to the new page. `peek`, so opening something does not re-run this.
    use_effect(use_reactive((&route,), move |(_route,)| {
        if state.open.peek().is_some() {
            state.close(false);
        }
    }));

    // Off wasm the listener is not installed (see `use_document_keydown`).
    use_document_keydown(move |evt| {
        let Some(current) = (state.open)() else {
            return;
        };
        match evt.key().as_str() {
            "Escape" => {
                evt.prevent_default();
                state.close(true);
            }
            "Tab" if current == Disclosure::Drawer => trap_tab_in_drawer(&evt),
            _ => {}
        }
    });

    let site_settings = use_site_settings();
    let relay = crate::state::provide_relay_availability();
    let resolved = site_settings.resolved.read();
    let loaded_settings = loaded_site_settings(resolved.as_ref());

    let session = auth();
    let loading = session.loading;
    let ctx = NavContext {
        signed_in: session.is_logged_in(),
        is_member: session.is_org_member(),
        is_recruit: session.is_recruit(),
        admin_access: session.is_officer_or_above(),
        is_admin: session.is_admin(),
        strategies_enabled: strategies_enabled_or_default(loaded_settings),
        relay_on: relay.current().shows_chat_links(),
    };
    let groups = nav_groups(ctx);
    let account = account_entries(ctx);
    let guest = guest_entries();
    let changelog = changelog_entry();
    let drawer_admin = drawer_admin_entries(ctx);
    let show_admin = shows_admin_entry(ctx);
    let section = section_of(&route);
    let show_footer = !is_tool_route(&route);

    let username = session
        .user
        .as_ref()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let avatar_url = safe_avatar_url(session.user.as_ref().and_then(|u| u.avatar_url.as_deref()));
    let initials = user_initials(&username);

    let org_name = loaded_settings.as_ref().map(|s| s.org_name.clone());
    let site_description = loaded_settings
        .as_ref()
        .map(|s| s.site_description.trim().to_string())
        .filter(|d| !d.is_empty());
    let footer_note = org_name.as_ref().map(|name| match &site_description {
        Some(desc) => format!("© {name} · {desc}"),
        None => format!("© {name}"),
    });
    let brand = org_name.clone().unwrap_or_default();
    let home_label = if brand.is_empty() {
        "Home".to_string()
    } else {
        format!("{brand}, home")
    };
    let mark = org_name
        .as_deref()
        .map(scuffed_types::org_initials)
        .unwrap_or_default();
    let mark_class = if mark.is_empty() {
        "d1-mark is-pending"
    } else {
        "d1-mark"
    };

    let open = (state.open)();
    // `main` holds a page modal, so it only goes inert behind the drawer.
    let inert = background_inert(open);
    let chrome = chrome_inert(open, (page_modal.0)());
    let drawer_open = open == Some(Disclosure::Drawer);
    let drawer_class = if drawer_open {
        "site-drawer is-open"
    } else {
        "site-drawer"
    };
    let account_open = open == Some(Disclosure::Account);
    let account_wrap_class = if account_open {
        "account-wrap is-open"
    } else {
        "account-wrap"
    };
    let account_btn_class = if account_open {
        "account-btn is-open"
    } else {
        "account-btn"
    };
    let admin_class = if section == Section::Admin {
        "btn-admin is-current"
    } else {
        "btn-admin"
    };
    let home_tab_class = if section == Section::Home && !drawer_open {
        "is-active"
    } else {
        ""
    };
    let menu_tab_class = if drawer_open { "is-active" } else { "" };
    let admin_current =
        section_link_current(&Route::AdminDashboard {}, &route, section == Section::Admin);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-layouts-shell", css: SHELL_CSS }
        a {
            class: "skip-link",
            href: "#{MAIN_ID}",
            inert: chrome,
            onclick: move |evt| {
                evt.prevent_default();
                focus_element(MAIN_ID);
            },
            "Skip to main content"
        }
        div { class: "site-app",
            header {
                class: "d1-top",
                inert: chrome,
                // A click on the bar itself closes an open panel. The group
                // and account wrappers stop their own clicks from getting here.
                onclick: move |_| state.close(false),
                Link {
                    to: Route::Home {},
                    class: "d1-home-link",
                    aria_label: "{home_label}",
                    onclick: move |_| state.close(false),
                    span { class: "{mark_class}", aria_hidden: "true", "{mark}" }
                    span { class: "d1-brand",
                        if brand.is_empty() {
                            span { class: "brand-pending", aria_hidden: "true" }
                        } else {
                            "{brand}"
                        }
                    }
                }
                nav { class: "d1-groups", aria_label: "Sections",
                    for group in groups.iter() {
                        div {
                            key: "{group.id.toggle_id()}",
                            class: group_class(open, section, group.id),
                            onclick: move |evt| evt.stop_propagation(),
                            button {
                                id: group.id.toggle_id(),
                                class: "d1-group-btn",
                                r#type: "button",
                                aria_expanded: expanded(open, Disclosure::Group(group.id)),
                                "aria-current": section_aria_current(section == Section::Group(group.id)),
                                aria_controls: Disclosure::Group(group.id).panel_id(),
                                onclick: {
                                    let target = Disclosure::Group(group.id);
                                    move |_| state.toggle(target)
                                },
                                "{group.id.label()}"
                            }
                            div { id: group.id.panel_id(), class: "d1-mega",
                                for entry in group.entries.iter() {
                                    Link {
                                        key: "mega-{entry.id}",
                                        to: entry.route.clone(),
                                        class: entry_class(entry, &route),
                                        "aria-current": aria_current(&entry.route, &route),
                                        onclick: move |_| state.close(false),
                                        "{entry.label}"
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "d1-actions",
                    if loading {
                        span { class: "d1-session-pending", "Checking session" }
                    } else if ctx.signed_in {
                        if show_admin {
                            Link {
                                to: Route::AdminDashboard {},
                                class: admin_class,
                                "aria-current": admin_current,
                                onclick: move |_| state.close(false),
                                "Admin"
                            }
                        }
                        div {
                            class: account_wrap_class,
                            onclick: move |evt| evt.stop_propagation(),
                            button {
                                id: ACCOUNT_TOGGLE_ID,
                                class: account_btn_class,
                                r#type: "button",
                                aria_label: "Account menu for {username}",
                                aria_expanded: expanded(open, Disclosure::Account),
                                aria_controls: ACCOUNT_PANEL_ID,
                                onclick: move |_| state.toggle(Disclosure::Account),
                                span { class: "account-avatar", aria_hidden: "true",
                                    if let Some(src) = avatar_url.clone() {
                                        img { src: "{src}", alt: "" }
                                    } else {
                                        "{initials}"
                                    }
                                }
                                span { class: "account-name", "{username}" }
                            }
                            div { id: ACCOUNT_PANEL_ID, class: "account-menu",
                                for entry in account.iter() {
                                    Link {
                                        key: "account-{entry.id}",
                                        to: entry.route.clone(),
                                        class: entry_class(entry, &route),
                                        "aria-current": aria_current(&entry.route, &route),
                                        onclick: move |_| state.close(false),
                                        "{entry.label}"
                                    }
                                }
                                if show_admin {
                                    Link {
                                        to: Route::AdminDashboard {},
                                        class: if section == Section::Admin { "is-active" } else { "" },
                                        "aria-current": admin_current,
                                        onclick: move |_| state.close(false),
                                        "Admin"
                                    }
                                }
                                button {
                                    class: "account-item",
                                    r#type: "button",
                                    onclick: move |_| {
                                        state.close(false);
                                        sign_out(auth);
                                    },
                                    "Sign out"
                                }
                            }
                        }
                    } else {
                        Link { to: Route::Apply {}, class: "d1-btn is-primary", "Apply" }
                        Link { to: Route::Login {}, class: "d1-btn", "Log in" }
                    }
                    ThemeToggle {}
                }
            }

            if matches!(open, Some(Disclosure::Group(_) | Disclosure::Account)) {
                div {
                    class: "nav-dismiss-layer",
                    aria_hidden: "true",
                    onclick: move |_| state.close(true),
                }
            }

            div {
                id: DRAWER_ID,
                class: drawer_class,
                role: "dialog",
                aria_modal: "true",
                aria_label: "Menu",
                div { class: "drawer-head",
                    span { class: "d1-brand", "{brand}" }
                    button {
                        id: DRAWER_CLOSE_ID,
                        class: "drawer-close",
                        r#type: "button",
                        aria_label: "Close menu",
                        onclick: move |_| state.close(true),
                        "Close"
                    }
                }
                Link {
                    to: Route::Home {},
                    class: if section == Section::Home { "is-active" } else { "" },
                    onclick: move |_| state.close(false),
                    "Home"
                }
                for group in groups.iter() {
                    h2 { key: "drawer-head-{group.id.toggle_id()}", "{group.id.label()}" }
                    for entry in group.entries.iter() {
                        Link {
                            key: "drawer-{entry.id}",
                            to: entry.route.clone(),
                            class: entry_class(entry, &route),
                            "aria-current": aria_current(&entry.route, &route),
                            onclick: move |_| state.close(false),
                            "{entry.label}"
                        }
                    }
                }
                if !drawer_admin.is_empty() {
                    h2 { "Admin" }
                    for entry in drawer_admin.iter() {
                        Link {
                            key: "drawer-{entry.id}",
                            to: entry.route.clone(),
                            class: entry_class(entry, &route),
                            "aria-current": aria_current(&entry.route, &route),
                            onclick: move |_| state.close(false),
                            "{entry.label}"
                        }
                    }
                }
                if !loading {
                    h2 { "Account" }
                    if ctx.signed_in {
                        for entry in account.iter() {
                            Link {
                                key: "drawer-{entry.id}",
                                to: entry.route.clone(),
                                class: entry_class(entry, &route),
                                "aria-current": aria_current(&entry.route, &route),
                                onclick: move |_| state.close(false),
                                "{entry.label}"
                            }
                        }
                        button {
                            class: "drawer-link",
                            r#type: "button",
                            onclick: move |_| {
                                state.close(true);
                                sign_out(auth);
                            },
                            "Sign out"
                        }
                    } else {
                        for entry in guest.iter() {
                            Link {
                                key: "drawer-{entry.id}",
                                to: entry.route.clone(),
                                class: entry_class(entry, &route),
                                "aria-current": aria_current(&entry.route, &route),
                                onclick: move |_| state.close(false),
                                "{entry.label}"
                            }
                        }
                    }
                    Link {
                        to: changelog.route.clone(),
                        class: entry_class(&changelog, &route),
                        "aria-current": aria_current(&changelog.route, &route),
                        onclick: move |_| state.close(false),
                        "{changelog.label}"
                    }
                }
                div { class: "drawer-theme",
                    ThemeToggle {}
                    span { "Theme" }
                }
            }

            if let Some(subnav) = subnav {
                div { class: "site-subnav", inert: chrome, {subnav} }
            }

            main {
                class: "site-main",
                id: MAIN_ID,
                tabindex: "-1",
                inert,
                onmounted: move |evt| main_mounted.set(Some(evt.data())),
                {children}
            }

            if show_footer {
                footer { class: "site-footer", inert: chrome,
                    nav { class: "footer-inner", aria_label: "Footer",
                        for group in groups.iter() {
                            div {
                                key: "footer-col-{group.id.toggle_id()}",
                                class: "footer-col",
                                h2 { "{group.id.label()}" }
                                for entry in group.entries.iter() {
                                    Link {
                                        key: "footer-{entry.id}",
                                        to: entry.route.clone(),
                                        class: entry_class(entry, &route),
                                        "aria-current": aria_current(&entry.route, &route),
                                        "{entry.label}"
                                    }
                                }
                            }
                        }
                        div { class: "footer-col",
                            h2 { "Account" }
                            if ctx.signed_in {
                                for entry in account.iter() {
                                    Link {
                                        key: "footer-{entry.id}",
                                        to: entry.route.clone(),
                                        class: entry_class(entry, &route),
                                        "aria-current": aria_current(&entry.route, &route),
                                        "{entry.label}"
                                    }
                                }
                                button {
                                    class: "footer-link",
                                    r#type: "button",
                                    onclick: move |_| sign_out(auth),
                                    "Sign out"
                                }
                            } else if !loading {
                                for entry in guest.iter() {
                                    Link {
                                        key: "footer-{entry.id}",
                                        to: entry.route.clone(),
                                        class: entry_class(entry, &route),
                                        "aria-current": aria_current(&entry.route, &route),
                                        "{entry.label}"
                                    }
                                }
                            }
                            Link {
                                to: changelog.route.clone(),
                                class: entry_class(&changelog, &route),
                                "aria-current": aria_current(&changelog.route, &route),
                                "{changelog.label}"
                            }
                            Link { to: Route::Home {}, "Home" }
                        }
                    }
                    p { class: "footer-note",
                        if let Some(note) = footer_note {
                            "{note}"
                        } else {
                            span { class: "brand-pending", aria_hidden: "true" }
                        }
                    }
                }
            }

            nav { class: "site-tab-bar", aria_label: "Primary", inert: chrome,
                Link {
                    to: Route::Home {},
                    class: home_tab_class,
                    onclick: move |_| state.close(false),
                    "Home"
                }
                for group in groups.iter() {
                    if let Some(landing) = group.landing() {
                        Link {
                            key: "tab-{group.id.toggle_id()}",
                            to: landing.clone(),
                            class: tab_class(open, section, group.id),
                            "aria-current": section_link_current(
                                &landing,
                                &route,
                                section == Section::Group(group.id),
                            ),
                            onclick: move |_| state.close(false),
                            "{group.id.label()}"
                        }
                    }
                }
                button {
                    id: DRAWER_TOGGLE_ID,
                    class: menu_tab_class,
                    r#type: "button",
                    aria_expanded: expanded(open, Disclosure::Drawer),
                    aria_controls: DRAWER_ID,
                    onclick: move |_| state.toggle(Disclosure::Drawer),
                    "Menu"
                }
            }
        }
    }
}

fn group_class(open: Option<Disclosure>, section: Section, group: NavGroupId) -> &'static str {
    match (
        open == Some(Disclosure::Group(group)),
        section == Section::Group(group),
    ) {
        (true, true) => "d1-group is-open is-current",
        (true, false) => "d1-group is-open",
        (false, true) => "d1-group is-current",
        (false, false) => "d1-group",
    }
}

/// The Menu tab takes the highlight while the drawer is open.
fn tab_class(open: Option<Disclosure>, section: Section, group: NavGroupId) -> &'static str {
    if open != Some(Disclosure::Drawer) && section == Section::Group(group) {
        "is-active"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::{OrgRole, UserInfo};

    use crate::state::{RelayAvailability, set_test_embedded_json, set_test_relay_availability};

    const COMMUNITY: Disclosure = Disclosure::Group(NavGroupId::Community);
    const COMPETE: Disclosure = Disclosure::Group(NavGroupId::Compete);

    #[derive(Clone, Copy, PartialEq)]
    enum Who {
        Guest,
        /// Signed in, no member row.
        Applicant,
        Member,
        Officer,
        Admin,
    }

    thread_local! {
        static WHO: std::cell::Cell<Who> = const { std::cell::Cell::new(Who::Guest) };
        static PATH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }

    /// Resets the per-thread test knobs when a test ends, pass or fail.
    struct Scene;

    impl Scene {
        fn new(who: Who, path: &str, open: Option<Disclosure>) -> Self {
            WHO.with(|slot| slot.set(who));
            PATH.with(|slot| *slot.borrow_mut() = path.to_string());
            STARTS_OPEN.with(|slot| slot.set(open));
            Self
        }
    }

    impl Drop for Scene {
        fn drop(&mut self) {
            WHO.with(|slot| slot.set(Who::Guest));
            PATH.with(|slot| slot.borrow_mut().clear());
            STARTS_OPEN.with(|slot| slot.set(None));
            set_test_relay_availability(None);
            set_test_embedded_json(None);
        }
    }

    fn app() -> Element {
        let path = PATH.with(|slot| slot.borrow().clone());
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(path)));
        let who = WHO.with(|slot| slot.get());
        let role = match who {
            Who::Guest | Who::Applicant => None,
            Who::Member => Some(OrgRole::Member),
            Who::Officer => Some(OrgRole::Officer),
            Who::Admin => Some(OrgRole::Admin),
        };
        let auth = use_signal(|| AuthState {
            user: (who != Who::Guest).then(|| UserInfo {
                id: "member-1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role,
            }),
            loading: false,
        });
        use_context_provider(|| auth);
        crate::state::provide_site_settings();
        rsx! {
            crate::theme::ThemeProvider {
                crate::components::ToastProvider {
                    dioxus::router::components::HistoryProvider {
                        history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                        Router::<crate::routes::Route> {}
                    }
                }
            }
        }
    }

    fn render(who: Who, path: &str, open: Option<Disclosure>) -> String {
        let _scene = Scene::new(who, path, open);
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn tag_with_id(html: &str, id: &str) -> String {
        let needle = format!("id=\"{id}\"");
        let id_at = html
            .find(&needle)
            .unwrap_or_else(|| panic!("missing {needle} in {html}"));
        let start = html[..id_at].rfind('<').expect("tag start");
        let end = html[id_at..]
            .find('>')
            .map(|rel| id_at + rel)
            .expect("tag end");
        html[start..=end].to_string()
    }

    /// The first opening tag that carries `class="…"` exactly.
    fn tag_with_class(html: &str, class: &str) -> String {
        let needle = format!("class=\"{class}\"");
        let at = html
            .find(&needle)
            .unwrap_or_else(|| panic!("missing {needle} in {html}"));
        let start = html[..at].rfind('<').expect("tag start");
        let end = html[at..].find('>').map(|rel| at + rel).expect("tag end");
        html[start..=end].to_string()
    }

    /// The html of the element with `id`, up to the next element named `until`.
    fn section<'a>(html: &'a str, from: &str, until: &str) -> &'a str {
        let start = html
            .find(from)
            .unwrap_or_else(|| panic!("missing {from} in {html}"));
        let rest = &html[start..];
        let end = rest[from.len()..]
            .find(until)
            .map(|rel| rel + from.len())
            .unwrap_or(rest.len());
        &rest[..end]
    }

    fn top_bar(html: &str) -> &str {
        section(html, "<header", "</header>")
    }

    fn drawer(html: &str) -> &str {
        section(html, "id=\"site-drawer\"", "<main")
    }

    fn css_rule(css: &str, marker: &str) -> String {
        let start = css
            .find(marker)
            .unwrap_or_else(|| panic!("missing {marker}"));
        css[start + marker.len()..]
            .split('}')
            .next()
            .unwrap_or_else(|| panic!("unclosed {marker}"))
            .to_string()
    }

    fn has_decl(block: &str, decl: &str) -> bool {
        block
            .split(';')
            .any(|part| part.lines().any(|line| line.trim() == decl))
    }

    /// QA 696 nit 2 (WCAG 2.4.11): Tab and the skip link scroll the page,
    /// so the page scroller keeps focus clear of the sticky top bar at every
    /// width and of the fixed tab bar on phone, from the shell's own height
    /// tokens, never a fixed length.
    #[test]
    fn focus_scrolling_clears_the_top_bar_and_the_phone_tab_bar() {
        let (wide, phone) = SHELL_CSS
            .split_once("@media (max-width: 820px) {")
            .expect("phone block");
        let root = css_rule(wide, "\n    html {");
        assert!(
            has_decl(
                &root,
                "scroll-padding-top: calc(var(--nav-height-used) + var(--space-2))"
            ),
            "{root}"
        );
        let phone_root = css_rule(phone, "\n        html {");
        assert!(
            has_decl(
                &phone_root,
                "--tab-bar-height: calc(var(--hit-target-min) + 1px + env(safe-area-inset-bottom, 0px))"
            ),
            "{phone_root}"
        );
        assert!(
            has_decl(
                &phone_root,
                "scroll-padding-bottom: calc(var(--tab-bar-height) + var(--space-2))"
            ),
            "{phone_root}"
        );
        // The shell's padding and chrome height read the same root value.
        let app = css_rule(phone, ".site-app {");
        assert!(
            has_decl(&app, "padding-bottom: var(--tab-bar-height)"),
            "{app}"
        );
        // The tab bar is exactly that tall: one hit target plus its border.
        let bar = css_rule(phone, ".site-tab-bar {");
        assert!(has_decl(&bar, "min-height: var(--hit-target-min)"), "{bar}");
        assert!(
            has_decl(&bar, "border-top: 1px solid var(--border-strong)"),
            "{bar}"
        );
    }

    #[test]
    fn a_toggle_opens_its_panel_and_closes_it_again() {
        assert_eq!(toggled(None, COMMUNITY), Some(COMMUNITY));
        assert_eq!(toggled(Some(COMMUNITY), COMMUNITY), None);
        // Opening one closes whatever else was open.
        assert_eq!(toggled(Some(COMMUNITY), COMPETE), Some(COMPETE));
        assert_eq!(
            toggled(Some(COMPETE), Disclosure::Account),
            Some(Disclosure::Account)
        );
        assert_eq!(
            toggled(Some(Disclosure::Account), Disclosure::Drawer),
            Some(Disclosure::Drawer)
        );
        assert_eq!(expanded(None, COMMUNITY), "false");
        assert_eq!(expanded(Some(COMMUNITY), COMMUNITY), "true");
        assert_eq!(expanded(Some(COMPETE), COMMUNITY), "false");
    }

    #[test]
    fn closing_returns_focus_to_the_button_that_opened_it() {
        for group in NavGroupId::ALL {
            assert_eq!(Disclosure::Group(group).toggle_id(), group.toggle_id());
            assert_eq!(Disclosure::Group(group).panel_id(), group.panel_id());
            assert_ne!(group.toggle_id(), group.panel_id());
        }
        assert_eq!(Disclosure::Account.toggle_id(), ACCOUNT_TOGGLE_ID);
        assert_eq!(Disclosure::Drawer.toggle_id(), DRAWER_TOGGLE_ID);

        // Every toggle id is a real element in the rendered shell.
        let html = render(Who::Member, "/", None);
        for target in [
            COMMUNITY,
            COMPETE,
            Disclosure::Group(NavGroupId::Strategy),
            Disclosure::Account,
            Disclosure::Drawer,
        ] {
            let toggle = tag_with_id(&html, target.toggle_id());
            assert!(toggle.starts_with("<button"), "{toggle}");
            assert!(
                toggle.contains(&format!("aria-controls=\"{}\"", target.panel_id())),
                "{toggle}"
            );
            // The controlled panel exists, once.
            let panel = format!("id=\"{}\"", target.panel_id());
            assert_eq!(html.matches(&panel).count(), 1, "{panel}");
        }

        let src = include_str!("shell.rs");
        let prod = src.split("mod tests").next().expect("production half");
        assert!(
            prod.contains("\"Escape\" => {\n                evt.prevent_default();\n                state.close(true);"),
            "Escape closes the open disclosure and returns focus"
        );
    }

    #[test]
    fn closed_groups_are_disclosures_with_aria_expanded_false() {
        let html = render(Who::Guest, "/", None);
        for group in NavGroupId::ALL {
            let toggle = tag_with_id(&html, group.toggle_id());
            assert!(toggle.contains("type=\"button\""), "{toggle}");
            assert!(toggle.contains("aria-expanded=\"false\""), "{toggle}");
            assert!(
                toggle.contains(&format!("aria-controls=\"{}\"", group.panel_id())),
                "{toggle}"
            );
        }
        assert!(!html.contains("is-open"), "nothing starts open: {html}");
        assert!(!html.contains("nav-dismiss-layer"), "{html}");
    }

    #[test]
    fn an_open_group_says_so_and_only_that_group() {
        let html = render(Who::Guest, "/", Some(COMMUNITY));
        let community = tag_with_id(&html, NavGroupId::Community.toggle_id());
        assert!(community.contains("aria-expanded=\"true\""), "{community}");
        for other in [NavGroupId::Compete, NavGroupId::Strategy] {
            let toggle = tag_with_id(&html, other.toggle_id());
            assert!(toggle.contains("aria-expanded=\"false\""), "{toggle}");
        }
        assert_eq!(html.matches("d1-group is-open").count(), 1, "{html}");
        assert!(
            html.contains("nav-dismiss-layer"),
            "an open panel has a click-outside layer: {html}"
        );
        let drawer_tag = tag_with_id(&html, DRAWER_ID);
        assert!(!drawer_tag.contains("is-open"), "{drawer_tag}");
    }

    #[test]
    fn the_mega_menu_is_not_an_aria_menu() {
        for who in [Who::Guest, Who::Admin] {
            for open in [None, Some(COMMUNITY), Some(Disclosure::Account)] {
                let html = render(who, "/", open);
                assert!(!html.contains("role=\"menu"), "{html}");
                assert!(!html.contains("aria-haspopup"), "{html}");
            }
        }
    }

    #[test]
    fn the_current_group_is_marked_in_the_bar_and_the_tab_bar() {
        let html = render(Who::Guest, "/members", None);
        assert_eq!(html.matches("d1-group is-current").count(), 1, "{html}");
        let bar = section(&html, "class=\"site-tab-bar\"", "</nav>");
        assert_eq!(bar.matches("is-active").count(), 1, "{bar}");
        let active_at = bar.find("is-active").expect("active tab");
        assert!(
            bar[active_at..].starts_with("is-active\" aria-current=\"page\">Community")
                || bar[active_at..].contains(">Community<"),
            "{bar}"
        );
        let members = section(top_bar(&html), "href=\"/members\"", "</a>");
        assert!(members.contains("is-active"), "{members}");
        assert!(members.contains("aria-current=\"page\""), "{members}");
    }

    #[test]
    fn guests_get_apply_log_in_and_the_theme_toggle() {
        let html = render(Who::Guest, "/", None);
        let bar = top_bar(&html);
        assert!(bar.contains("href=\"/apply\""), "{bar}");
        assert!(bar.contains("href=\"/login\""), "{bar}");
        assert!(bar.contains(">Log in<"), "{bar}");
        assert!(bar.contains("class=\"theme-toggle\""), "{bar}");
        assert!(!bar.contains(ACCOUNT_TOGGLE_ID), "{bar}");
        assert!(!html.contains("href=\"/admin"), "{html}");
        assert!(!html.contains("href=\"/chat\""), "{html}");
        assert!(!html.contains("href=\"/dm\""), "{html}");
    }

    #[test]
    fn members_lose_apply_and_get_the_account_control() {
        let html = render(Who::Member, "/", None);
        assert!(
            !html.contains("href=\"/apply\""),
            "Apply leaves the signed-in chrome: {html}"
        );
        assert!(!html.contains("href=\"/login\""), "{html}");
        let bar = top_bar(&html);
        let toggle = tag_with_id(bar, ACCOUNT_TOGGLE_ID);
        assert!(toggle.contains("aria-expanded=\"false\""), "{toggle}");
        assert!(
            toggle.contains("aria-label=\"Account menu for Player01\""),
            "{toggle}"
        );
        assert!(bar.contains("class=\"theme-toggle\""), "{bar}");
        // Initials when there is no picture, hidden from the name.
        assert!(bar.contains("aria-hidden=\"true\">PL<"), "{bar}");

        let open = render(Who::Member, "/", Some(Disclosure::Account));
        let toggle = tag_with_id(&open, ACCOUNT_TOGGLE_ID);
        assert!(toggle.contains("aria-expanded=\"true\""), "{toggle}");
        assert!(toggle.contains("account-btn is-open"), "{toggle}");
        assert!(open.contains("account-wrap is-open"), "{open}");
    }

    #[test]
    fn direct_messages_sit_in_the_account_menu_and_chat_under_community() {
        let html = render(Who::Member, "/", None);
        let account = section(&html, "id=\"nav-account-panel\"", "</header>");
        assert!(account.contains("href=\"/dm\""), "{account}");
        assert!(account.contains(">Direct messages<"), "{account}");
        assert!(!account.contains("href=\"/chat\""), "{account}");
        assert!(account.contains(">Sign out<"), "{account}");

        let community = section(&html, "id=\"nav-group-community-panel\"", "</div>");
        assert!(community.contains("href=\"/chat\""), "{community}");
        assert!(community.contains("href=\"/wiki\""), "{community}");
        assert!(!community.contains("href=\"/dm\""), "{community}");
    }

    #[test]
    fn admin_entry_is_shown_to_officers_and_admins_only() {
        for who in [Who::Guest, Who::Member] {
            let html = render(who, "/", None);
            assert!(
                !html.contains("href=\"/admin"),
                "no Admin entry anywhere for this visitor: {html}"
            );
            assert!(!html.contains(">Admin<"), "{html}");
        }
        for who in [Who::Officer, Who::Admin] {
            let html = render(who, "/", None);
            let bar = top_bar(&html);
            assert!(bar.contains("class=\"btn-admin\""), "{bar}");
            let account = section(&html, "id=\"nav-account-panel\"", "</header>");
            assert!(account.contains("href=\"/admin\""), "{account}");
            assert!(account.contains(">Admin<"), "{account}");
            let menu = drawer(&html);
            assert!(menu.contains("<h2>Admin</h2>"), "{menu}");
            assert!(menu.contains("href=\"/admin\""), "{menu}");
        }
        let officer = render(Who::Officer, "/", None);
        assert!(!officer.contains("href=\"/admin/settings\""), "{officer}");
        let admin = render(Who::Admin, "/", None);
        assert!(
            drawer(&admin).contains("href=\"/admin/settings\""),
            "{admin}"
        );
    }

    #[test]
    fn admin_pages_keep_the_top_bar_and_add_the_sub_nav() {
        let html = render(Who::Officer, "/admin", None);
        assert!(html.contains("class=\"d1-top\""), "{html}");
        assert!(html.contains("btn-admin is-current"), "{html}");
        let subnav = section(&html, "class=\"admin-subnav\"", "</nav>");
        assert!(subnav.contains("aria-label=\"Admin\""), "{subnav}");
        let dashboard = section(subnav, "href=\"/admin\"", "</a>");
        assert!(dashboard.contains("is-active"), "{dashboard}");
        assert!(dashboard.contains("aria-current=\"page\""), "{dashboard}");
        assert_eq!(subnav.matches("is-active").count(), 1, "{subnav}");
        assert!(subnav.contains("href=\"/admin/moderation\""), "{subnav}");
        assert!(!subnav.contains("href=\"/admin/settings\""), "{subnav}");
        assert!(html.find("admin-subnav") < html.find("<main"), "{html}");

        let admin = render(Who::Admin, "/admin/members/m1/matches", None);
        let subnav = section(&admin, "class=\"admin-subnav\"", "</nav>");
        assert!(subnav.contains("href=\"/admin/settings\""), "{subnav}");
        let members = section(subnav, "href=\"/admin/members\"", "</a>");
        assert!(members.contains("is-active"), "{members}");
    }

    #[test]
    fn a_member_on_an_admin_page_keeps_the_nav_and_is_not_told_to_sign_in() {
        let html = render(Who::Member, "/admin", None);
        assert!(html.contains("class=\"d1-top\""), "{html}");
        assert!(!html.contains("admin-subnav\""), "{html}");
        assert!(html.contains("You do not have access"), "{html}");
        assert!(html.contains("signed in as Player01"), "{html}");
        assert!(!html.contains("href=\"/login\""), "{html}");
        assert!(html.contains(">Return home<"), "{html}");
        assert_eq!(html.matches("<h1").count(), 1, "{html}");

        let guest = render(Who::Guest, "/admin", None);
        let wall = section(&guest, "<main", "</main>");
        assert!(wall.contains(">Log in<"), "{wall}");
        assert!(!guest.contains("admin-subnav\""), "{guest}");
    }

    #[test]
    fn strategy_pages_use_the_site_top_bar_with_no_second_bar() {
        let html = render(Who::Guest, "/strategy/heroes", None);
        assert!(html.contains("class=\"d1-top\""), "{html}");
        assert!(!html.contains("strategy-nav"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert_eq!(html.matches("d1-group is-current").count(), 1, "{html}");
        let strategy = section(&html, "id=\"nav-group-strategy-panel\"", "</div>");
        let heroes = section(strategy, "href=\"/strategy/heroes\"", "</a>");
        assert!(heroes.contains("is-active"), "{heroes}");
        assert!(strategy.contains("href=\"/patch-notes\""), "{strategy}");
    }

    #[test]
    fn skip_link_comes_first_and_targets_the_main_landmark() {
        for (who, path) in [
            (Who::Guest, "/"),
            (Who::Admin, "/admin"),
            (Who::Guest, "/strategy"),
        ] {
            let html = render(who, path, None);
            let skip = html.find("class=\"skip-link\"").expect("skip link");
            let header = html.find("<header").expect("header");
            assert!(skip < header, "skip link must be the first focusable thing");
            assert!(html.contains("href=\"#main\""), "{html}");
            assert!(html.contains("Skip to main content"), "{html}");
            assert_eq!(html.matches("<main").count(), 1, "{path}: {html}");
            let main_at = html.find("<main").expect("main");
            let main_tag = &html[main_at..main_at + html[main_at..].find('>').unwrap()];
            assert!(main_tag.contains("id=\"main\""), "{main_tag}");
            assert!(main_tag.contains("tabindex=\"-1\""), "{main_tag}");
        }
    }

    #[test]
    fn tab_bar_has_five_items_and_menu_is_a_disclosure_button() {
        let html = render(Who::Guest, "/", None);
        let bar = section(&html, "class=\"site-tab-bar\"", "</nav>");
        assert!(bar.contains("aria-label=\"Primary\""), "{bar}");
        assert_eq!(bar.matches("<a ").count(), 4, "{bar}");
        assert_eq!(bar.matches("<button").count(), 1, "{bar}");
        let order: Vec<usize> = ["Home", "Community", "Compete", "Strategy", "Menu"]
            .iter()
            .map(|label| {
                bar.find(&format!(">{label}<"))
                    .unwrap_or_else(|| panic!("missing tab {label}: {bar}"))
            })
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{bar}");
        for href in ["/", "/members", "/tournaments", "/strategy"] {
            assert!(bar.contains(&format!("href=\"{href}\"")), "{href}: {bar}");
        }
        let menu = tag_with_id(&html, DRAWER_TOGGLE_ID);
        assert!(menu.contains("aria-expanded=\"false\""), "{menu}");
        assert!(menu.contains("aria-controls=\"site-drawer\""), "{menu}");
    }

    #[test]
    fn open_drawer_is_a_modal_dialog_over_an_inert_page() {
        let closed = render(Who::Guest, "/", None);
        let drawer_tag = tag_with_id(&closed, DRAWER_ID);
        assert!(drawer_tag.contains("class=\"site-drawer\""), "{drawer_tag}");
        assert!(
            !closed.contains("inert"),
            "nothing is inert while the drawer is closed: {closed}"
        );

        let open = render(Who::Guest, "/", Some(Disclosure::Drawer));
        let drawer_tag = tag_with_id(&open, DRAWER_ID);
        assert!(drawer_tag.contains("site-drawer is-open"), "{drawer_tag}");
        assert!(drawer_tag.contains("role=\"dialog\""), "{drawer_tag}");
        assert!(drawer_tag.contains("aria-modal=\"true\""), "{drawer_tag}");
        assert!(drawer_tag.contains("aria-label=\"Menu\""), "{drawer_tag}");
        assert!(!drawer_tag.contains("inert"), "{drawer_tag}");
        assert!(!drawer_tag.contains("aria-hidden"), "{drawer_tag}");

        for background in [
            tag_with_class(&open, "d1-top"),
            tag_with_id(&open, MAIN_ID),
            tag_with_class(&open, "site-footer"),
            tag_with_class(&open, "site-tab-bar"),
        ] {
            assert!(background.contains("inert"), "{background}");
            assert!(!background.contains("aria-hidden"), "{background}");
        }
        let menu = tag_with_id(&open, DRAWER_TOGGLE_ID);
        assert!(menu.contains("aria-expanded=\"true\""), "{menu}");
        assert!(menu.contains("class=\"is-active\""), "{menu}");
        let close = tag_with_id(&open, DRAWER_CLOSE_ID);
        assert!(close.contains("aria-label=\"Close menu\""), "{close}");

        let subnav = render(Who::Admin, "/admin", Some(Disclosure::Drawer));
        assert!(
            tag_with_class(&subnav, "site-subnav").contains("inert"),
            "{subnav}"
        );
    }

    /// SSR drops a false boolean attribute, so html alone cannot tell
    /// `inert: false` from no attribute. The web renderer writes Bool(false)
    /// as the string "false", which still makes the element inert.
    #[test]
    fn inert_is_written_as_text_or_removed_never_as_a_bool() {
        use dioxus::dioxus_core::{AttributeValue, Mutation, Mutations};

        for (open, want_inert) in [(None, false), (Some(Disclosure::Drawer), true)] {
            let _scene = Scene::new(Who::Guest, "/", open);
            let mut dom = VirtualDom::new(app);
            let mut edits = Mutations::default();
            dom.rebuild(&mut edits);
            let writes: Vec<AttributeValue> = edits
                .edits
                .into_iter()
                .filter_map(|edit| match edit {
                    Mutation::SetAttribute {
                        name: "inert",
                        value,
                        ..
                    } => Some(value),
                    _ => None,
                })
                .collect();
            assert!(
                !writes.is_empty(),
                "the shell writes inert on its background"
            );
            for value in &writes {
                match value {
                    AttributeValue::None => assert!(!want_inert, "open drawer: {value:?}"),
                    AttributeValue::Text(text) => {
                        assert!(want_inert, "closed drawer: {value:?}");
                        assert_eq!(text, "true");
                    }
                    other => panic!("inert must be text or removed, got {other:?}"),
                }
            }
        }
        assert_eq!(background_inert(None), None);
        assert_eq!(background_inert(Some(COMMUNITY)), None);
        assert_eq!(background_inert(Some(Disclosure::Account)), None);
        assert_eq!(background_inert(Some(Disclosure::Drawer)), Some("true"));
    }

    /// Vision blocking 2: behind the editor's map picker the shell chrome is
    /// inert, but `main` (which holds the picker) is not.
    #[test]
    fn a_page_modal_makes_the_chrome_inert_but_not_main() {
        assert_eq!(chrome_inert(None, false), None);
        assert_eq!(chrome_inert(Some(COMMUNITY), false), None);
        assert_eq!(chrome_inert(None, true), Some("true"));
        assert_eq!(chrome_inert(Some(Disclosure::Drawer), false), Some("true"));
        assert_eq!(background_inert(None), None, "main stays live");
    }

    #[test]
    fn focus_is_trapped_at_both_ends_of_the_drawer() {
        // Tab on the last stop wraps to the first, Shift+Tab on the first to the last.
        assert_eq!(
            tab_trap_target(false, true, false, true),
            Some(TrapEdge::First)
        );
        assert_eq!(
            tab_trap_target(true, true, true, false),
            Some(TrapEdge::Last)
        );
        // In the middle the browser moves focus.
        assert_eq!(tab_trap_target(false, true, false, false), None);
        assert_eq!(tab_trap_target(true, true, false, false), None);
        assert_eq!(tab_trap_target(false, true, true, false), None);
        assert_eq!(tab_trap_target(true, true, false, true), None);
        // Focus that got outside is pulled back in.
        assert_eq!(
            tab_trap_target(false, false, false, false),
            Some(TrapEdge::First)
        );
        assert_eq!(
            tab_trap_target(true, false, false, false),
            Some(TrapEdge::Last)
        );
    }

    #[test]
    fn drawer_lists_home_every_group_and_the_account_section() {
        let guest = render(Who::Guest, "/", None);
        let menu = drawer(&guest);
        for heading in ["Community", "Compete", "Strategy", "Account"] {
            assert!(
                menu.contains(&format!("<h2>{heading}</h2>")),
                "{heading}: {menu}"
            );
        }
        for href in [
            "/",
            "/members",
            "/forum",
            "/news",
            "/blog",
            "/wiki",
            "/tournaments",
            "/events",
            "/scrims",
            "/leaderboards",
            "/stats",
            "/strategy",
            "/strategy/heroes",
            "/patch-notes",
            "/apply",
            "/login",
        ] {
            assert!(menu.contains(&format!("href=\"{href}\"")), "{href}: {menu}");
        }
        // Meta stays hidden, as before the redesign. New strategy is for members.
        assert!(!guest.contains("href=\"/strategy/meta\""), "{guest}");
        assert!(!guest.contains("href=\"/strategy/editor\""), "{guest}");
        assert!(menu.contains("class=\"theme-toggle\""), "{menu}");
        assert!(!menu.contains("<h2>Admin</h2>"), "{menu}");

        let member = render(Who::Member, "/", None);
        let menu = drawer(&member);
        for href in [
            "/feed",
            "/polls",
            "/chat",
            "/community",
            "/strategy/my",
            "/strategy/editor",
            "/profile/edit",
            "/dm",
            "/identity",
            "/reports",
            "/link",
        ] {
            assert!(menu.contains(&format!("href=\"{href}\"")), "{href}: {menu}");
        }
        assert!(!menu.contains("href=\"/apply\""), "{menu}");
        assert!(menu.contains(">Sign out<"), "{menu}");
    }

    #[test]
    fn drawer_is_a_solid_panel() {
        let rule = css_rule(SHELL_CSS, ".site-drawer.is-open {");
        assert!(has_decl(&rule, "background: var(--bg)"), "{rule}");
        assert!(has_decl(&rule, "inset: 0"), "{rule}");
        assert!(!rule.contains("color-mix"), "{rule}");
        assert!(!rule.contains("transparent"), "{rule}");
        assert!(!rule.contains("opacity"), "{rule}");
        // Hidden with display, so a closed drawer holds no focus stop.
        let closed = css_rule(SHELL_CSS, ".site-drawer {");
        assert!(has_decl(&closed, "display: none"), "{closed}");
    }

    #[test]
    fn footer_has_the_four_columns_inside_a_named_nav() {
        let html = render(Who::Guest, "/", None);
        let footer = section(&html, "<footer", "</footer>");
        assert!(footer.contains("aria-label=\"Footer\""), "{footer}");
        let order: Vec<usize> = ["Community", "Compete", "Strategy", "Account"]
            .iter()
            .map(|heading| {
                footer
                    .find(&format!("<h2>{heading}</h2>"))
                    .unwrap_or_else(|| panic!("missing column {heading}: {footer}"))
            })
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{footer}");
        assert_eq!(footer.matches("<h2>").count(), 4, "{footer}");
        for href in ["/members", "/stats", "/patch-notes", "/apply", "/login"] {
            assert!(
                footer.contains(&format!("href=\"{href}\"")),
                "{href}: {footer}"
            );
        }
        // The shell adds no page heading of its own (settings are pending here).
        assert!(html.matches("<h1").count() <= 1, "{html}");

        let member = render(Who::Member, "/", None);
        let footer = section(&member, "<footer", "</footer>");
        assert!(footer.contains("href=\"/profile/edit\""), "{footer}");
        assert!(footer.contains(">Sign out<"), "{footer}");
        assert!(!footer.contains("href=\"/apply\""), "{footer}");
    }

    #[test]
    fn full_height_tools_leave_the_footer_off() {
        let chat = render(Who::Member, "/chat", None);
        assert!(!chat.contains("<footer"), "{chat}");
        assert!(chat.contains("class=\"site-tab-bar\""), "{chat}");
        let home = render(Who::Member, "/", None);
        assert!(home.contains("<footer"), "{home}");
    }

    /// QA nit 11: a signed-in account with no membership had no way to the
    /// application page from the shell.
    #[test]
    fn a_signed_in_account_without_a_membership_gets_apply_in_the_account_menu() {
        let html = render(Who::Applicant, "/", None);
        let account = section(&html, "id=\"nav-account-panel\"", "</header>");
        assert!(account.contains("href=\"/apply\""), "{account}");
        assert!(account.contains(">Apply<"), "{account}");
        assert!(drawer(&html).contains("href=\"/apply\""), "{html}");
        // Still no guest buttons, and no member-only entries.
        assert!(!html.contains("href=\"/login\""), "{html}");
        assert!(!html.contains("d1-btn is-primary"), "{html}");
        assert!(!html.contains("href=\"/strategy/editor\""), "{html}");
        assert!(!html.contains("href=\"/admin"), "{html}");
    }

    /// QA nit 6: a nested or detail page marks its parent link, and every
    /// page in a section marks the section's group button and phone tab.
    #[test]
    fn nested_and_detail_pages_mark_their_parents_with_aria_current() {
        let html = render(Who::Admin, "/admin/members/m1/matches", None);
        let subnav = section(&html, "class=\"admin-subnav\"", "</nav>");
        let members = section(subnav, "href=\"/admin/members\"", "</a>");
        assert!(members.contains("aria-current=\"true\""), "{members}");
        assert_eq!(subnav.matches("aria-current").count(), 1, "{subnav}");
        let admin = tag_with_class(&html, "btn-admin is-current");
        assert!(admin.contains("aria-current=\"true\""), "{admin}");

        for (path, parent) in [
            ("/tournaments/t1", "/tournaments"),
            ("/teams/t1", "/members"),
            ("/forum/b/general", "/forum"),
        ] {
            let html = render(Who::Guest, path, None);
            let link = section(top_bar(&html), &format!("href=\"{parent}\""), "</a>");
            assert!(link.contains("aria-current=\"true\""), "{path}: {link}");
            assert!(link.contains("is-active"), "{path}: {link}");
            let bar = section(&html, "class=\"site-tab-bar\"", "</nav>");
            assert_eq!(bar.matches("aria-current").count(), 1, "{path}: {bar}");
        }

        // A detail page with no list entry still marks its section.
        let html = render(Who::Guest, "/matches/m1", None);
        let compete = tag_with_id(&html, NavGroupId::Compete.toggle_id());
        assert!(compete.contains("aria-current=\"true\""), "{compete}");
        let community = tag_with_id(&html, NavGroupId::Community.toggle_id());
        assert!(!community.contains("aria-current"), "{community}");
        let bar = section(&html, "class=\"site-tab-bar\"", "</nav>");
        let tab = section(bar, "href=\"/tournaments\"", "</a>");
        assert!(tab.contains("aria-current=\"true\""), "{tab}");

        // QA nit 1 on 8ec68607: the shell writes `page` itself, because on
        // web a `None` after `Link`'s own value removes it. SSR prints both
        // attributes; a tag may repeat `aria-current` only with one value
        // (the browser keeps one), never `page` and `true` together.
        for (who, path) in [
            (Who::Guest, "/members"),
            (Who::Guest, "/"),
            (Who::Admin, "/admin"),
            (Who::Admin, "/admin/members"),
            (Who::Member, "/forum"),
        ] {
            let html = render(who, path, None);
            for tag in html.split('<').skip(1) {
                let tag = tag.split('>').next().unwrap_or_default();
                let values: Vec<&str> = tag
                    .split("aria-current=\"")
                    .skip(1)
                    .filter_map(|rest| rest.split('"').next())
                    .collect();
                assert!(
                    values.windows(2).all(|pair| pair[0] == pair[1]),
                    "{path}: conflicting aria-current in <{tag}>"
                );
            }
        }
        // Every shell link to the page carries `page` from the shell, so it
        // survives on web whatever `Link` does.
        for (who, path, places) in [
            (Who::Guest, "/members", 3),
            (Who::Admin, "/admin", 3),
            (Who::Admin, "/admin/members", 2),
        ] {
            let html = render(who, path, None);
            let href = format!("href=\"{path}\"");
            let mut links = 0;
            for tag in html.split("<a ").skip(1) {
                let tag = tag.split('>').next().unwrap_or_default();
                if tag.contains(&href) {
                    links += 1;
                    assert!(tag.contains("aria-current=\"page\""), "{path}: <a {tag}>");
                    assert!(!tag.contains("aria-current=\"true\""), "{path}: <a {tag}>");
                }
            }
            assert!(links >= places, "{path}: {links} links");
        }
        assert_eq!(
            section_link_current(&Route::Home {}, &Route::Home {}, true),
            Some("page"),
        );
        assert_eq!(
            section_link_current(&Route::AdminDashboard {}, &Route::AdminDashboard {}, true),
            Some("page"),
        );
        assert_eq!(
            section_link_current(&Route::Members {}, &Route::Forum {}, true),
            Some("true")
        );
        assert_eq!(
            section_link_current(&Route::Members {}, &Route::Stats {}, false),
            None
        );
    }

    /// QA nits 3 and 5: what closes without a click or Escape.
    #[test]
    fn the_drawer_closes_past_the_phone_width_and_panels_close_when_focus_leaves() {
        assert!(closes_on_resize(Some(Disclosure::Drawer), false));
        assert!(!closes_on_resize(Some(Disclosure::Drawer), true));
        assert!(!closes_on_resize(Some(COMMUNITY), false));
        assert!(!closes_on_resize(Some(Disclosure::Account), false));
        assert!(!closes_on_resize(None, false));
        // Vision nit 6: desktop panels close on entering the phone layout.
        assert!(closes_on_resize(Some(COMMUNITY), true));
        assert!(closes_on_resize(Some(Disclosure::Account), true));
        assert!(!closes_on_resize(None, true));
        // The listener and the stylesheet agree on where phone ends.
        assert_eq!(PHONE_QUERY, "(max-width: 820px)");
        assert!(SHELL_CSS.contains(&format!("@media {PHONE_QUERY} {{")));

        assert!(closes_on_focus_move(Some(COMMUNITY), false));
        assert!(closes_on_focus_move(Some(Disclosure::Account), false));
        assert!(!closes_on_focus_move(Some(COMMUNITY), true));
        assert!(!closes_on_focus_move(Some(Disclosure::Account), true));
        // The drawer traps focus instead.
        assert!(!closes_on_focus_move(Some(Disclosure::Drawer), false));
        assert!(!closes_on_focus_move(None, false));

        // A toggle and its panel share a wrapper, which is what "inside" means.
        let html = render(Who::Member, "/", Some(COMMUNITY));
        let group = section(&html, "class=\"d1-group is-open\"", "</nav>");
        assert!(group.contains(NavGroupId::Community.toggle_id()), "{group}");
        assert!(group.contains(NavGroupId::Community.panel_id()), "{group}");
        let wrap = section(&html, "class=\"account-wrap\"", "</header>");
        assert!(wrap.contains(ACCOUNT_TOGGLE_ID) && wrap.contains(ACCOUNT_PANEL_ID));

        // Route changes close too (Back and Forward included).
        let src = include_str!("shell.rs");
        let prod = src.split("mod tests").next().expect("production half");
        assert!(
            prod.contains("use_effect(use_reactive((&route,), move |(_route,)| {"),
            "a route change closes the open disclosure"
        );
    }

    /// QA blocking 1: `/admin/relay` was 518px wide at 390. Wide admin
    /// content scrolls inside the admin area, and the relay tables sit in
    /// the shared table scroller.
    #[test]
    fn wide_admin_content_scrolls_inside_the_admin_area() {
        let admin = include_str!("admin.rs");
        let rule = css_rule(admin, ".admin-main {");
        assert!(has_decl(&rule, "overflow-y: auto"), "{rule}");
        assert!(has_decl(&rule, "min-width: 0"), "{rule}");
        let main = css_rule(SHELL_CSS, ".site-main {");
        assert!(has_decl(&main, "min-width: 0"), "{main}");

        let scroller = css_rule(crate::styles::admin::CSS, ".data-table-scroll {");
        assert!(has_decl(&scroller, "overflow-x: auto"), "{scroller}");
        let relay = include_str!("../pages/admin/relay.rs");
        let tables = relay.matches("table { class: \"data-table\",").count();
        assert_eq!(tables, 2, "relay tables");
        assert_eq!(
            relay
                .matches("div { class: \"data-table-scroll\",\n")
                .count(),
            tables,
            "every relay table sits in its own scroller"
        );
        for (at, _) in relay.match_indices("table { class: \"data-table\",") {
            let before = relay[..at].trim_end();
            assert!(
                before.ends_with("div { class: \"data-table-scroll\","),
                "a relay table is outside the scroller"
            );
        }
    }

    /// QA nit 8: Firefox made the scrolling sub-nav a Tab stop of its own.
    #[test]
    fn the_sub_nav_scroller_is_not_a_tab_stop() {
        let html = render(Who::Admin, "/admin", None);
        let subnav = tag_with_class(&html, "admin-subnav");
        assert!(subnav.contains("tabindex=\"-1\""), "{subnav}");
        // Its links are the stops, and they are plain links.
        let links = section(&html, "class=\"admin-subnav\"", "</nav>");
        assert!(links.matches("<a ").count() >= 19, "{links}");
        assert!(!links.contains("<a tabindex"), "{links}");
    }

    /// QA nit 7: "Community" was 74px in a 72px column at 360 in Firefox.
    #[test]
    fn narrow_phones_give_the_tab_labels_room() {
        let narrow = SHELL_CSS
            .split("@media (max-width: 380px) {")
            .nth(1)
            .expect("narrow phone rules");
        let rule = css_rule(narrow, ".site-tab-bar a,\n        .site-tab-bar button {");
        assert!(has_decl(&rule, "font-size: var(--text-xs)"), "{rule}");
        assert!(has_decl(&rule, "padding-left: 0"), "{rule}");
        assert!(has_decl(&rule, "padding-right: 0"), "{rule}");
    }

    /// Vision blocking 2 on PR 693: the map picker that opens on a new
    /// strategy is a named modal dialog, and the editor behind it is inert.
    /// `main` holds the picker, so it stays live. The shell chrome goes inert
    /// through an effect (`use_page_modal`), which SSR does not run: see
    /// `a_page_modal_turns_the_chrome_inert_after_its_effect_runs`.
    #[test]
    fn the_editor_map_picker_is_a_modal_over_an_inert_page() {
        let html = render(Who::Member, "/strategy/editor", None);

        let dialog = tag_with_id(&html, "map-picker");
        assert!(dialog.contains("role=\"dialog\""), "{dialog}");
        assert!(dialog.contains("aria-modal=\"true\""), "{dialog}");
        assert!(
            dialog.contains("aria-labelledby=\"map-picker-title\""),
            "{dialog}"
        );
        assert!(!dialog.contains("inert"), "{dialog}");
        let title = section(&html, "id=\"map-picker-title\"", "</h3>");
        assert!(title.contains("Select a Map"), "{title}");
        let close = tag_with_id(&html, "map-picker-close");
        assert!(close.contains("aria-label=\"Close map picker\""), "{close}");

        assert!(!tag_with_id(&html, MAIN_ID).contains("inert"));
        for background in [
            tag_with_class(&html, "editor-container"),
            tag_with_class(&html, "editor-statusbar"),
        ] {
            assert!(background.contains("inert=\"true\""), "{background}");
        }
        // The Map button that reopens it, where focus returns on close.
        let map_button = tag_with_id(&html, "editor-map-button");
        assert!(map_button.contains("status-map-link"), "{map_button}");
        // Vision nit 21: map buttons are a one-of-many choice, not ~30
        // toggles. No map is chosen yet, so none is current and the old
        // toggle attribute is gone.
        let picker = section(&html, "id=\"map-picker\"", "</main>");
        assert!(picker.contains("map-picker-item"), "{picker}");
        assert!(!picker.contains("aria-pressed"), "{picker}");
        assert!(!picker.contains("aria-current"), "{picker}");
        assert!(!picker.contains("map-picker-selected"), "{picker}");
    }

    /// The rendered editor: the web map canvas is not a Tab stop (focus by
    /// click; as a Tab stop it would swallow every further Tab), the panel
    /// help names the real keys, and the key hint is a live region that is
    /// always mounted and empty until a key is pressed in the map.
    #[test]
    fn the_editor_canvas_help_and_hint_render() {
        let html = render(Who::Member, "/strategy/editor", None);

        let canvas = tag_with_class(&html, "map-canvas map-canvas-overlay");
        assert!(canvas.contains("tabindex=\"-1\""), "{canvas}");
        assert!(
            canvas.contains("Esc cancels a drawing or selection. Esc or Shift+Tab leaves the map."),
            "{canvas}"
        );
        assert_eq!(html.matches("map-canvas-overlay").count(), 1);

        assert!(html.contains("title=\"Toggle panel (Tab while the map has focus)\""));
        assert!(html.contains("title=\"Toggle panel (] while the map has focus)\""));
        assert!(html.contains("title=\"Toggle elements panel ([ while the map has focus)\""));
        assert!(!html.contains("(Shift+Tab)"));
        assert!(!html.contains("title=\"Toggle panel (Tab)\""));

        let hint = section(&html, "class=\"canvas-key-hint\"", "</p>");
        assert!(hint.contains("role=\"status\""), "{hint}");
        assert!(hint.ends_with('>'), "empty until a key: {hint}");
    }

    thread_local! {
        /// The stand-in host's `open` and `mounted` signals.
        static MODAL_HOST: std::cell::Cell<Option<(Signal<bool>, Signal<bool>)>> = const { std::cell::Cell::new(None) };
    }

    #[component]
    fn ModalPage(open: bool) -> Element {
        use_page_modal(open);
        rsx! {}
    }

    /// A stand-in for the shell: provides the flag and renders its chrome
    /// with `chrome_inert`, as `SiteShell` does, around a page with a modal.
    fn modal_host() -> Element {
        let flag = use_context_provider(|| PageModal(Signal::new(false)));
        let chrome = chrome_inert(None, (flag.0)());
        let open = use_signal(|| true);
        let mounted = use_signal(|| true);
        MODAL_HOST.with(|slot| slot.set(Some((open, mounted))));
        rsx! {
            header { class: "d1-top", inert: chrome }
            if mounted() {
                ModalPage { open: open() }
            }
        }
    }

    /// `use_page_modal` reaches the shell: after the page's effect runs the
    /// chrome is inert; closing the modal, or leaving the page with it
    /// open, makes the chrome live again.
    #[test]
    fn a_page_modal_turns_the_chrome_inert_after_its_effect_runs() {
        use dioxus::dioxus_core::NoOpMutations;

        fn settle(dom: &mut VirtualDom) -> String {
            for _ in 0..4 {
                dom.render_immediate(&mut NoOpMutations);
            }
            dioxus_ssr::render(dom)
        }
        fn chrome_is_inert(html: &str) -> bool {
            tag_with_class(html, "d1-top").contains("inert=\"true\"")
        }
        fn host() -> (Signal<bool>, Signal<bool>) {
            MODAL_HOST
                .with(std::cell::Cell::get)
                .expect("host rendered")
        }

        let mut dom = VirtualDom::new(modal_host);
        dom.rebuild_in_place();
        assert!(
            !chrome_is_inert(&dioxus_ssr::render(&dom)),
            "before the effect"
        );
        assert!(chrome_is_inert(&settle(&mut dom)), "modal open");

        let (mut open, mut mounted) = host();
        dom.in_runtime(|| open.set(false));
        assert!(!chrome_is_inert(&settle(&mut dom)), "modal closed");
        dom.in_runtime(|| open.set(true));
        assert!(chrome_is_inert(&settle(&mut dom)), "modal open again");

        // Navigating away with the modal open must not leave the chrome inert.
        dom.in_runtime(|| mounted.set(false));
        assert!(!chrome_is_inert(&settle(&mut dom)), "page unmounted");
        MODAL_HOST.with(|slot| slot.set(None));
    }

    /// QA nit 12: the map picker and the tab bar were both at 100, and the
    /// tab bar comes later in the document, so it painted over the picker.
    #[test]
    fn the_map_picker_sits_above_the_bars_and_below_the_drawer() {
        fn z_index(rule: &str) -> u32 {
            rule.lines()
                .find_map(|line| line.trim().strip_prefix("z-index:"))
                .and_then(|value| value.trim().trim_end_matches(';').parse().ok())
                .unwrap_or_else(|| panic!("no z-index in {rule}"))
        }
        let editor = include_str!("../pages/strategy/editor.rs");
        let picker = z_index(&css_rule(editor, ".map-picker-overlay {"));
        let top = z_index(&css_rule(SHELL_CSS, ".d1-top {"));
        let phone = SHELL_CSS
            .split("@media (max-width: 820px) {")
            .nth(1)
            .expect("phone rules");
        let tabs = z_index(&css_rule(phone, ".site-tab-bar {"));
        let drawer = z_index(&css_rule(SHELL_CSS, ".site-drawer.is-open {"));
        assert!(picker > top && picker > tabs, "{picker} {top} {tabs}");
        assert!(picker < drawer, "{picker} {drawer}");
    }

    /// QA nit 9: `/chat` at 390 was 10px taller than the viewport. The
    /// embedded chat was a fixed 70vh under a page header; on phone the page
    /// now takes the height the shell leaves and the chat fills it.
    #[test]
    fn the_chat_page_fits_the_shell_on_phone() {
        let chat = include_str!("../pages/chat.rs");
        let phone = chat
            .split("@media (max-width: 820px) {")
            .nth(1)
            .expect("phone rules");
        let page = css_rule(phone, ".team-chat-page {");
        assert!(
            has_decl(
                &page,
                "min-height: calc(100dvh - var(--shell-chrome-height, 0px))"
            ),
            "{page}"
        );
        assert!(has_decl(&page, "flex-direction: column"), "{page}");
        let widget = css_rule(phone, ".team-chat-page .chat-widget--embedded {");
        assert!(has_decl(&widget, "flex: 1 1 0"), "{widget}");
        assert!(has_decl(&widget, "height: auto"), "{widget}");

        // The relay-off state subtracts the tab bar too, not only the top bar.
        let gate = include_str!("../pages/chat_gate.rs");
        let gate = css_rule(gate, ".chat-unavailable {");
        assert!(
            gate.contains("var(--shell-chrome-height, var(--nav-height-used))"),
            "{gate}"
        );
    }

    /// QA nit 13: what sticks on the patch notes page sits under the top bar
    /// by the bar's real height (`--nav-height-used`), never a fixed length.
    /// The redesigned page has no table of contents column; its sticky parts
    /// are the controls and the shared scroll offset, so those are pinned.
    #[test]
    fn patch_notes_sticky_offsets_follow_the_top_bar() {
        let notes = include_str!("../pages/strategy/patch_notes.rs");
        let controls = css_rule(notes, "\n    .pn-controls {");
        assert!(has_decl(&controls, "position: sticky"), "{controls}");
        assert!(
            has_decl(&controls, "top: var(--nav-height-used)"),
            "{controls}"
        );
        let page = css_rule(notes, "\n    .patch-page {");
        assert!(
            page.contains("--pn-sticky-offset: calc(")
                && page.contains("var(--nav-height-used) + var(--hit-target-min) * 2"),
            "{page}"
        );
        // Vision nit 11 (PR 693): on phone the shell chrome is the top bar
        // plus the tab bar. The page's sticky parts sit at the top, so they
        // only need the top bar; the chrome height stays pinned for the
        // rules that fill the viewport.
        let phone = SHELL_CSS
            .split("@media (max-width: 820px) {")
            .nth(1)
            .expect("phone rules");
        assert!(phone.contains(
            "--shell-chrome-height: calc(var(--nav-height-used) + var(--tab-bar-height))"
        ));
        // Every sticky or viewport-height rule on the page uses the bar's
        // height; the old fixed offsets are gone with the old column.
        let old = ["100vh - 8.5", "rem"].concat();
        assert!(!notes.contains(&old), "the old fixed offset is gone");
        let css = notes
            .split("const PAGE_CSS: &str = r#\"")
            .nth(1)
            .and_then(|rest| rest.split("\"#;").next())
            .expect("PAGE_CSS");
        for line in css.lines() {
            let line = line.trim();
            if line.starts_with("top:") || line.contains("100vh") {
                assert!(line.contains("var(--nav-height-used)"), "{line}");
            }
        }
        assert!(!css.contains(".patch-hero-toc"), "the old column is gone");
    }

    fn settings_json() -> String {
        r#"{"id":"site","org_name":"Org","site_description":"desc","recruitment_open":true,"recruitment_message":"msg","min_age":16,"forum_backend":"local","extra_relay_urls":"","updated_at":"2026-10-06T00:00:00Z"}"#.to_string()
    }

    fn render_member_with_relay(relay: RelayAvailability) -> String {
        let _scene = Scene::new(Who::Member, "/", None);
        set_test_embedded_json(Some(settings_json()));
        set_test_relay_availability(Some(relay));
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn chat_and_dm_links_follow_the_relay() {
        let off = render_member_with_relay(RelayAvailability::Off);
        assert_eq!(off.matches("href=\"/chat\"").count(), 0, "{off}");
        assert_eq!(off.matches("href=\"/dm\"").count(), 0, "{off}");
        assert!(off.contains("href=\"/members\""), "{off}");
        assert!(off.contains(">My reports<"), "{off}");

        for relay in [RelayAvailability::Failed, RelayAvailability::On] {
            let html = render_member_with_relay(relay);
            // Mega menu, drawer and footer for Chat. Account menu, drawer and footer for DMs.
            assert_eq!(html.matches("href=\"/chat\"").count(), 3, "{html}");
            assert_eq!(html.matches("href=\"/dm\"").count(), 3, "{html}");
        }
    }

    #[test]
    fn org_name_fills_the_mark_the_home_link_and_the_footer_note() {
        let _scene = Scene::new(Who::Guest, "/", None);
        set_test_embedded_json(Some(settings_json()));
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        let home = tag_with_class(&html, "d1-home-link");
        assert!(home.contains("aria-label=\"Org, home\""), "{home}");
        assert!(html.contains("class=\"d1-mark\""), "{html}");
        assert!(html.contains("© Org · desc"), "{html}");
    }

    #[test]
    fn initials_and_avatar_sources() {
        assert_eq!(user_initials("Player01"), "PL");
        assert_eq!(user_initials("x"), "X");
        assert_eq!(user_initials("_-"), "?");
        assert_eq!(user_initials(""), "?");
        assert_eq!(
            safe_avatar_url(Some("/uploads/a.png")).as_deref(),
            Some("/uploads/a.png")
        );
        assert_eq!(
            safe_avatar_url(Some(" https://cdn.example/a.png ")).as_deref(),
            Some("https://cdn.example/a.png")
        );
        assert_eq!(safe_avatar_url(Some("javascript:alert(1)")), None);
        assert_eq!(safe_avatar_url(Some("http://plain.example/a.png")), None);
        assert_eq!(safe_avatar_url(None), None);
    }

    /// Browser pass: the bar was 1px taller than its token (button height
    /// plus the border), and full-height tools had nothing to subtract.
    #[test]
    fn shell_reports_the_height_it_takes() {
        let bar = css_rule(SHELL_CSS, ".d1-top {");
        assert!(has_decl(&bar, "height: var(--nav-height-used)"), "{bar}");
        assert!(has_decl(&bar, "box-sizing: border-box"), "{bar}");
        let button = css_rule(SHELL_CSS, ".d1-group-btn {");
        assert!(
            !button.contains("height: var(--nav-height-used)"),
            "group buttons stretch inside the bar, they do not set its height: {button}"
        );
        let app = css_rule(SHELL_CSS, ".site-app {");
        assert!(
            has_decl(&app, "--shell-chrome-height: var(--nav-height-used)"),
            "{app}"
        );
        let phone = SHELL_CSS
            .split("@media (max-width: 820px)")
            .nth(1)
            .expect("phone rules");
        assert!(
            phone.contains(
                "--shell-chrome-height: calc(var(--nav-height-used) + var(--tab-bar-height))"
            ),
            "{phone}"
        );
        assert!(
            phone.contains("padding-bottom: var(--tab-bar-height)"),
            "{phone}"
        );
        assert_eq!(MAIN_ID, "main");
    }

    #[test]
    fn site_main_is_a_block_that_can_shrink() {
        let main = css_rule(SHELL_CSS, ".site-main {");
        assert!(has_decl(&main, "min-width: 0"), "{main}");
        assert!(
            !main.contains("display:"),
            "site main stays a normal block: {main}"
        );
        assert!(
            !main.contains("padding"),
            "the top bar is sticky and in flow, so main needs no offset: {main}"
        );
        assert!(
            !SHELL_CSS.contains(".site-main > *"),
            "page roots keep their own widths"
        );
        let app = css_rule(SHELL_CSS, ".site-app {");
        assert!(has_decl(&app, "min-height: 100vh"), "{app}");
        let bar = css_rule(SHELL_CSS, ".d1-top {");
        assert!(has_decl(&bar, "position: sticky"), "{bar}");
        assert!(has_decl(&bar, "background: var(--bg)"), "{bar}");
    }

    #[test]
    fn shell_type_and_targets_come_from_tokens() {
        for line in SHELL_CSS.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("font-size:") {
                assert!(
                    value.trim().starts_with("var(--"),
                    "shell font sizes are tokens, so the 12px floor holds: {line}"
                );
            }
        }
        // 44px targets for everything a thumb reaches on phone.
        for marker in [
            ".site-tab-bar a,\n    .site-tab-bar button {",
            ".site-drawer a,\n    .site-drawer .drawer-link {",
            ".drawer-close {",
            ".footer-col a,\n    .footer-col .footer-link {",
            ".admin-subnav a {",
            ".d1-mega a {",
        ] {
            let rule = css_rule(SHELL_CSS, marker);
            assert!(
                has_decl(&rule, "min-height: var(--hit-target-min)"),
                "{marker}: {rule}"
            );
        }
        let phone = SHELL_CSS
            .split("@media (max-width: 820px) {")
            .nth(1)
            .expect("phone rules");
        let toggle = css_rule(phone, ".d1-mark,\n        .d1-top .theme-toggle {");
        assert!(
            has_decl(&toggle, "width: var(--hit-target-min)"),
            "{toggle}"
        );
        assert!(
            has_decl(&toggle, "height: var(--hit-target-min)"),
            "{toggle}"
        );
        let account = css_rule(phone, ".account-btn {");
        assert!(
            has_decl(&account, "width: var(--hit-target-min)"),
            "{account}"
        );
    }

    /// Every `font-size:` value in `src` that is a rem or px size under 12px.
    /// `var(--text-*)` sizes are skipped, theme tokens floor those.
    fn sizes_below_twelve_px(src: &str) -> Vec<String> {
        let mut small = Vec::new();
        for (at, _) in src.match_indices("font-size:") {
            let rest = &src[at + "font-size:".len()..];
            let end = rest.find([';', '"', '}', '\n']).unwrap_or(rest.len());
            let value = rest[..end].trim();
            let px = if let Some(rem) = value.strip_suffix("rem") {
                rem.parse::<f32>().ok().map(|r| r * 16.0)
            } else if let Some(px) = value.strip_suffix("px") {
                px.parse::<f32>().ok()
            } else {
                None
            };
            if px.is_some_and(|px| px < 12.0) {
                small.push(value.to_string());
            }
        }
        small
    }

    #[test]
    fn pages_in_the_a11y_pass_never_drop_below_twelve_px() {
        let files = [
            ("layouts/shell.rs", include_str!("shell.rs")),
            ("layouts/public.rs", include_str!("public.rs")),
            ("layouts/admin.rs", include_str!("admin.rs")),
            (
                "components/ui/toggle.rs",
                include_str!("../components/ui/toggle.rs"),
            ),
            (
                "pages/leaderboards.rs",
                include_str!("../pages/leaderboards.rs"),
            ),
            (
                "pages/stats_member.rs",
                include_str!("../pages/stats_member.rs"),
            ),
            ("pages/stats/mod.rs", include_str!("../pages/stats/mod.rs")),
            (
                "pages/stats/game.rs",
                include_str!("../pages/stats/game.rs"),
            ),
            (
                "pages/stats/heroes.rs",
                include_str!("../pages/stats/heroes.rs"),
            ),
            (
                "pages/stats/history.rs",
                include_str!("../pages/stats/history.rs"),
            ),
            (
                "pages/strategy/browse.rs",
                include_str!("../pages/strategy/browse.rs"),
            ),
            (
                "pages/strategy/patch_notes.rs",
                include_str!("../pages/strategy/patch_notes.rs"),
            ),
        ];
        for (name, src) in files {
            let small = sizes_below_twelve_px(src);
            assert!(small.is_empty(), "{name} has sizes below 12px: {small:?}");
        }
    }

    #[test]
    fn small_size_scan_sees_rem_and_px_but_not_tokens() {
        // Built from pieces so this file's own scan does not see a small size.
        let fs = ["font", "-size:"].concat();
        let css = format!(
            "a {{ {fs} 0.65rem; }} b {{ {fs} 11px; }} c {{ {fs} 0.75rem; }} \
             d {{ {fs} var(--text-xs); }}"
        );
        assert_eq!(sizes_below_twelve_px(&css), vec!["0.65rem", "11px"]);
    }
}
