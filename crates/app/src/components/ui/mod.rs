pub mod avatar;
pub mod button;
pub mod card;
pub mod feedback;
pub mod field;
pub mod hero_select;
pub mod inline_confirm;
pub mod label;
pub mod pager;
pub mod pill;
pub mod season_select;
pub mod shell;
pub mod toggle;
#[allow(unused_imports)]
pub use button::BtnSize;
#[allow(unused_imports)]
pub use button::{BtnVariant, Button, btn_class};
pub use card::Card;
#[allow(unused_imports)]
pub use feedback::EmptyState;
#[allow(unused_imports)]
pub use feedback::Spinner;
#[allow(unused_imports)]
pub use feedback::{EmptyCard, LoadError, SkeletonBars, SkeletonRows, StatusLine};
pub use field::FieldError;
#[allow(unused_imports)]
pub use field::Textarea;
#[allow(unused_imports)]
pub use field::{Input, Select};
#[allow(unused_imports)]
pub use hero_select::HeroSelect;
#[allow(unused_imports)]
pub use inline_confirm::InlineConfirm;
#[allow(unused_imports)]
pub use label::Label;
#[allow(unused_imports)]
pub use pager::{
    PageSlot, Pager, clamp_page, page_count, page_range, page_slice, page_window, range_text,
    remember_landing_url, show_list_top, use_url_page,
};
pub use pill::{Pill, PillTone};
pub use season_select::{SeasonSelect, use_season_change, use_stats_season};
#[allow(unused_imports)]
pub use shell::{PageHeader, PageShell, PageWidth};
#[allow(unused_imports)]
pub use toggle::ChipRow;
pub use toggle::{ChipButton, TabButton};

/// Concatenated CSS for every ui/ component, injected once at the app root.
pub fn ui_css() -> String {
    [
        button::BUTTON_CSS,
        button::BTN_CSS,
        card::CARD_CSS,
        pill::PILL_CSS,
        label::LABEL_CSS,
        shell::SHELL_CSS,
        toggle::CHIP_CSS,
        pager::PAGER_CSS,
        field::FIELD_CSS,
        field::FORM_FIELD_CSS,
        hero_select::HERO_SELECT_CSS,
        season_select::SEASON_SELECT_CSS,
        feedback::FEEDBACK_CSS,
        feedback::EMPTY_CARD_CSS,
        inline_confirm::INLINE_CONFIRM_CSS,
    ]
    .concat()
}

/// Helpers shared by the ui/ tests: SSR rendering, CSS rule lookup, and the
/// token audits (contrast, type floor, sizes).
#[cfg(test)]
#[allow(unreachable_code)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    use dioxus::prelude::*;

    use crate::theme::brand::{BrandConfig, STRATEGY_BRAND, contrast_ratio};
    use crate::theme::tokens::{resolved_tokens, theme_css};

    pub fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    type View = std::rc::Rc<dyn Fn() -> Element>;

    thread_local! {
        static VIEW: std::cell::RefCell<Option<View>> = const { std::cell::RefCell::new(None) };
    }

    #[derive(Clone, Routable, Debug, PartialEq)]
    enum ProbeRoute {
        #[route("/:..segments")]
        ProbeView { segments: Vec<String> },
    }

    #[component]
    fn ProbeView(segments: Vec<String>) -> Element {
        let _ = segments;
        let view = VIEW.with(|slot| slot.borrow().clone());
        match view {
            Some(view) => view(),
            None => rsx! {},
        }
    }

    fn routed_shell() -> Element {
        let history = use_hook(|| {
            std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path("/"))
                as std::rc::Rc<dyn History>
        });
        rsx! {
            dioxus::router::components::HistoryProvider {
                history: move |_| history.clone(),
                Router::<ProbeRoute> {}
            }
        }
    }

    /// Render a view that contains `Link`s. A `Link` needs a router above it,
    /// so the view is mounted as the single route of a small router over an
    /// in-memory history.
    pub fn render_routed(view: impl Fn() -> Element + 'static) -> String {
        VIEW.with(|slot| *slot.borrow_mut() = Some(std::rc::Rc::new(view)));
        let mut dom = VirtualDom::new(routed_shell);
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        VIEW.with(|slot| *slot.borrow_mut() = None);
        html
    }

    /// Declarations of the first rule whose text starts at `marker`
    /// (`".pager {"`).
    pub fn css_rule(css: &str, marker: &str) -> String {
        let start = css
            .find(marker)
            .unwrap_or_else(|| panic!("missing {marker}"));
        css[start + marker.len()..]
            .split('}')
            .next()
            .unwrap_or_else(|| panic!("unclosed {marker}"))
            .to_string()
    }

    /// Body of the first at-rule block that starts at `head`
    /// (`"@media (forced-colors: active)"`), nested rules included.
    pub fn media_block(css: &str, head: &str) -> String {
        let start = css.find(head).unwrap_or_else(|| panic!("missing {head}"));
        let open = start
            + css[start..]
                .find('{')
                .unwrap_or_else(|| panic!("unopened {head}"));
        let mut depth = 0usize;
        for (at, ch) in css[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return css[open + 1..open + at].to_string();
                    }
                }
                _ => {}
            }
        }
        panic!("unclosed {head}")
    }

    /// Resolve two font-weight tokens to numbers in every scope and require
    /// `heavy` to be at least 100 above `base`. Asserting only that a rule
    /// names a weight token cannot catch two tokens with the same value.
    pub fn assert_weight_heavier(heavy: &str, base: &str) {
        let number = |tokens: &HashMap<String, String>, scope: &str, name: &str| {
            let value = tokens
                .get(name)
                .unwrap_or_else(|| panic!("{scope}: {name} is not a token"));
            value
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("{scope}: {name} resolves to {value}, not a weight"))
        };
        for (scope, tokens) in scopes() {
            let (h, b) = (number(tokens, scope, heavy), number(tokens, scope, base));
            assert!(
                h >= b + 100,
                "{scope}: {heavy} is {h} and {base} is {b}, so there is no weight cue"
            );
        }
    }

    pub fn has_decl(block: &str, decl: &str) -> bool {
        block.split(';').any(|part| part.trim() == decl)
    }

    fn without_comments(css: &str) -> String {
        let mut text = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            rest = rest[start + 2..]
                .split_once("*/")
                .map_or("", |(_, tail)| tail);
        }
        text.push_str(rest);
        text
    }

    /// Every `(selector list, declarations)` rule. Rules inside `@media` or
    /// `@supports` come out like top level ones.
    pub fn rules(css: &str) -> Vec<(String, String)> {
        without_comments(css)
            .split('}')
            .filter_map(|chunk| {
                let (head, body) = chunk.rsplit_once('{')?;
                let selector = head.rsplit('{').next().unwrap_or(head);
                Some((selector.trim().to_string(), body.trim().to_string()))
            })
            .collect()
    }

    /// Token maps for dark and light under each brand the app can derive:
    /// the product default purple, the pending gray, the live mint and the
    /// strategy orange.
    fn scopes() -> &'static [(String, HashMap<String, String>)] {
        static SCOPES: OnceLock<Vec<(String, HashMap<String, String>)>> = OnceLock::new();
        SCOPES.get_or_init(|| {
            let product = BrandConfig::product_default();
            let primitives = resolved_tokens(&theme_css(&product), "dark");
            let mint =
                BrandConfig::from_accents(&primitives["--mint-400"], &primitives["--mint-600"]);
            let strategy = BrandConfig::from_accents(STRATEGY_BRAND, STRATEGY_BRAND);
            let mut out = Vec::new();
            for (name, brand) in [
                ("product default", product),
                ("pending", BrandConfig::pending()),
                ("mint", mint),
                ("strategy", strategy),
            ] {
                let css = theme_css(&brand);
                for theme in ["dark", "light"] {
                    out.push((format!("{name} {theme}"), resolved_tokens(&css, theme)));
                }
            }
            out
        })
    }

    /// Resolve the `ink` and `fill` tokens to hex in every scope and require
    /// `min` contrast between them.
    pub fn assert_ink_clears_aa(ink: &str, fill: &str, min: f64) {
        let hex = |tokens: &HashMap<String, String>, scope: &str, name: &str| {
            let value = tokens
                .get(name)
                .unwrap_or_else(|| panic!("{scope}: {name} is not a token"))
                .clone();
            assert!(
                value.len() == 7 && value.starts_with('#'),
                "{scope}: {name} resolves to {value}, not an opaque hex"
            );
            value
        };
        assert_eq!(scopes().len(), 8);
        for (scope, tokens) in scopes() {
            let (fg, bg) = (hex(tokens, scope, ink), hex(tokens, scope, fill));
            let ratio = contrast_ratio(&fg, &bg);
            assert!(
                ratio >= min,
                "{scope}: {ink} {fg} on {fill} {bg} is {ratio:.2}, needs {min}"
            );
        }
    }

    /// Font sizes are `--text-*` tokens (12px floor). The 11px badge size is
    /// allowed only in a rule that is also uppercase and weight 600.
    pub fn assert_type_from_tokens(css: &str) {
        let mut seen = 0;
        for (selector, body) in rules(css) {
            for decl in body.split(';') {
                let Some((prop, value)) = decl.split_once(':') else {
                    continue;
                };
                if prop.trim() != "font-size" {
                    continue;
                }
                seen += 1;
                let value = value.trim();
                if value == "var(--badge-font-size)" {
                    assert!(
                        has_decl(&body, "text-transform: var(--badge-font-transform)")
                            && has_decl(&body, "font-weight: var(--badge-font-weight)"),
                        "{selector}: 11px needs uppercase and weight 600"
                    );
                } else {
                    // Component size tokens that resolve to a `--text-*` step.
                    assert!(
                        value == "var(--chip-font-size)"
                            || value == "var(--button-font-size)"
                            || (value.starts_with("var(--text-") && value.ends_with(')')),
                        "{selector}: font-size {value} is not a type token"
                    );
                }
            }
        }
        assert!(seen > 0, "no font-size in this stylesheet");
    }

    /// Type from tokens, no rem, and px only for hairlines (1px).
    pub fn assert_sizes_from_tokens(css: &str) {
        assert_type_from_tokens(css);
        for (selector, body) in rules(css) {
            for decl in body.split(';') {
                let Some((_, value)) = decl.split_once(':') else {
                    continue;
                };
                for word in
                    value.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
                {
                    assert!(!word.ends_with("rem"), "{selector}: {decl}");
                    if word.ends_with("px") {
                        assert!(
                            word == "1px" || word == "-1px",
                            "{selector}: {} is not a token",
                            decl.trim()
                        );
                    }
                }
            }
        }
    }
}
