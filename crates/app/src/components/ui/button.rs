use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum BtnVariant {
    Primary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, PartialEq)]
pub enum BtnSize {
    Sm,
    Md,
}

fn class_for(variant: BtnVariant, size: BtnSize) -> String {
    let v = match variant {
        BtnVariant::Primary => "ui-btn--primary",
        BtnVariant::Ghost => "ui-btn--ghost",
        BtnVariant::Danger => "ui-btn--danger",
    };
    let s = match size {
        BtnSize::Sm => "ui-btn--sm",
        BtnSize::Md => "ui-btn--md",
    };
    format!("ui-btn {v} {s}")
}

pub const BUTTON_CSS: &str = r#"
.ui-btn { font-family: var(--font-body); font-weight: 600; border: none; cursor: pointer;
  border-radius: var(--radius-control); display: inline-flex; align-items: center; gap: var(--space-2);
  transition: filter .12s, background .12s; }
.ui-btn--sm { font-size: var(--text-sm); padding: var(--space-2) var(--space-3); }
.ui-btn--md { font-size: var(--text-md); padding: var(--space-2) var(--space-4); }
.ui-btn--primary { background: var(--accent-solid); color: var(--on-accent); }
.ui-btn--ghost { background: transparent; border: 1px solid var(--border); color: var(--text); }
.ui-btn--danger { background: var(--loss); color: var(--on-loss); }
.ui-btn:hover:not(:disabled) { filter: brightness(1.08); }
.ui-btn:disabled { opacity: .5; cursor: not-allowed; }
"#;

/// The redesign button (`.btn` in the frames), for links and buttons alike:
/// `btn` is the secondary button, `btn is-primary` the solid one, `btn
/// is-ghost` the outline on a transparent fill. 44px tall at every width, on
/// the `--button-*` tokens only. The modifiers are `is-*` classes so they
/// cannot meet the older admin `btn-ghost` rules.
pub const BTN_CSS: &str = r#"
.btn {
  box-sizing: border-box;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  min-height: var(--button-height);
  min-width: var(--hit-target-min);
  margin: 0;
  padding: var(--space-2) var(--button-pad-x);
  border: 1px solid var(--button-secondary-border);
  border-radius: var(--button-radius);
  background: var(--button-secondary-bg);
  color: var(--button-secondary-fg);
  font-family: var(--button-font);
  font-size: var(--button-font-size);
  font-weight: var(--button-font-weight);
  letter-spacing: var(--font-head-tracking);
  line-height: var(--leading-ui);
  text-align: center;
  text-decoration: none;
  cursor: pointer;
}
.btn.is-ghost {
  background: transparent;
  color: var(--text);
  border-color: var(--border-strong);
}
.btn.is-primary {
  background: var(--button-primary-bg);
  color: var(--button-primary-fg);
  border-color: var(--button-primary-border);
}
.btn:hover { border-color: var(--text); }
.btn.is-primary:hover {
  background: var(--button-primary-hover-bg);
  border-color: var(--button-primary-border);
  text-decoration: underline;
}
.btn:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
/* Busy: still focusable, so focus is not dropped while a request runs. */
.btn[aria-disabled="true"] {
  opacity: 0.6;
  cursor: progress;
}
"#;

/// `btn` class string for the first (primary) action of a row or a later one.
pub const fn btn_class(primary: bool) -> &'static str {
    if primary {
        "btn is-primary"
    } else {
        "btn is-ghost"
    }
}

#[component]
pub fn Button(
    #[props(default = BtnVariant::Primary)] variant: BtnVariant,
    #[props(default = BtnSize::Md)] size: BtnSize,
    #[props(default = false)] disabled: bool,
    #[props(default)] id: Option<String>,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let class = class_for(variant, size);
    rsx! {
        button {
            id,
            class: "{class}",
            disabled,
            onclick: move |e| {
                if let Some(h) = &onclick {
                    h.call(e);
                }
            },
            {children}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_variant_and_size_to_classes() {
        assert_eq!(
            class_for(BtnVariant::Primary, BtnSize::Md),
            "ui-btn ui-btn--primary ui-btn--md"
        );
        assert_eq!(
            class_for(BtnVariant::Ghost, BtnSize::Sm),
            "ui-btn ui-btn--ghost ui-btn--sm"
        );
    }

    #[test]
    fn redesign_button_is_44px_on_the_button_tokens() {
        use crate::components::ui::test_support::{
            assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl,
        };
        assert_sizes_from_tokens(BTN_CSS);
        let btn = css_rule(BTN_CSS, ".btn {");
        for decl in [
            "min-height: var(--button-height)",
            "min-width: var(--hit-target-min)",
            "padding: var(--space-2) var(--button-pad-x)",
            "border: 1px solid var(--button-secondary-border)",
            "border-radius: var(--button-radius)",
            "background: var(--button-secondary-bg)",
            "color: var(--button-secondary-fg)",
            "font-family: var(--button-font)",
            "font-size: var(--button-font-size)",
            "font-weight: var(--button-font-weight)",
        ] {
            assert!(has_decl(&btn, decl), "missing {decl}: {btn}");
        }
        let primary = css_rule(BTN_CSS, ".btn.is-primary {");
        for decl in [
            "background: var(--button-primary-bg)",
            "color: var(--button-primary-fg)",
            "border-color: var(--button-primary-border)",
        ] {
            assert!(has_decl(&primary, decl), "missing {decl}: {primary}");
        }
        // Hovering the primary button keeps its own border.
        let hover = css_rule(BTN_CSS, ".btn.is-primary:hover {");
        assert!(
            has_decl(&hover, "border-color: var(--button-primary-border)"),
            "{hover}"
        );
        assert!(
            BTN_CSS.find(".btn:hover {").unwrap()
                < BTN_CSS.find(".btn.is-primary:hover {").unwrap()
        );
        let focus = css_rule(BTN_CSS, ".btn:focus-visible {");
        assert!(
            has_decl(
                &focus,
                "outline: var(--focus-ring-width) solid var(--focus-ring)"
            ),
            "{focus}"
        );
        // --button-height is the 44px hit target at every width.
        let tokens = include_str!("../../theme/css/components.css");
        assert!(tokens.contains("--button-height: var(--hit-target-min);"));
        assert_ink_clears_aa("--on-accent", "--accent-solid", 4.5);
        assert_ink_clears_aa("--text", "--surface", 4.5);
        assert_ink_clears_aa("--text", "--bg", 4.5);
        assert_eq!(btn_class(true), "btn is-primary");
        assert_eq!(btn_class(false), "btn is-ghost");
    }
}
