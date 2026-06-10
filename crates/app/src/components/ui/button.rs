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
  border-radius: var(--radius-md); display: inline-flex; align-items: center; gap: var(--space-2);
  transition: filter .12s, background .12s; }
.ui-btn--sm { font-size: var(--text-sm); padding: var(--space-2) var(--space-3); }
.ui-btn--md { font-size: var(--text-base); padding: var(--space-2) var(--space-4); }
.ui-btn--primary { background: var(--accent); color: var(--accent-fg); }
.ui-btn--ghost { background: transparent; border: 1px solid var(--border); color: var(--text); }
.ui-btn--danger { background: var(--danger); color: var(--accent-fg); }
.ui-btn:hover:not(:disabled) { filter: brightness(1.08); }
.ui-btn:disabled { opacity: .5; cursor: not-allowed; }
"#;

#[component]
pub fn Button(
    #[props(default = BtnVariant::Primary)] variant: BtnVariant,
    #[props(default = BtnSize::Md)] size: BtnSize,
    #[props(default = false)] disabled: bool,
    #[props(default)] onclick: Option<EventHandler<MouseEvent>>,
    children: Element,
) -> Element {
    let class = class_for(variant, size);
    rsx! {
        button {
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
}
