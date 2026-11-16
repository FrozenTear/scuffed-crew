//! Badges and status pills.
//!
//! One recipe for every small label: the `--badge-*` component tokens, with an
//! opaque soft fill and its own ink (`--win-soft` under `--win`, and so on).
//! Each ink clears 4.5:1 on its fill in dark and light; the tests below resolve
//! the pairs to hex and check. The label is 11px, which the type floor allows
//! only because it is uppercase and weight 600.
//!
//! The same rules style [`Pill`] (`.ui-pill`) and the older `.status-pill` /
//! `.role-pill` classes, so pages that still use those need no rename. New
//! code uses [`Pill`] with a tone. A tone is never the only signal: the pill
//! always carries the word.

use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillTone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Danger,
    Info,
}

impl PillTone {
    /// Modifier class next to `ui-pill`.
    pub const fn class(self) -> &'static str {
        match self {
            PillTone::Neutral => "ui-pill--neutral",
            PillTone::Accent => "ui-pill--accent",
            PillTone::Ok => "ui-pill--ok",
            PillTone::Warn => "ui-pill--warn",
            PillTone::Danger => "ui-pill--danger",
            PillTone::Info => "ui-pill--info",
        }
    }
}

pub const PILL_CSS: &str = r#"
.ui-pill,
.status-pill,
.role-pill {
  box-sizing: border-box;
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  min-height: var(--badge-height);
  padding: 0 var(--badge-pad-x);
  border: 1px solid var(--badge-border);
  border-radius: var(--badge-radius);
  background: var(--badge-bg);
  color: var(--badge-fg);
  font-family: var(--badge-font);
  font-size: var(--badge-font-size);
  font-weight: var(--badge-font-weight);
  letter-spacing: var(--font-head-tracking);
  text-transform: var(--badge-font-transform);
  line-height: var(--leading-ui);
  white-space: nowrap;
  vertical-align: middle;
  text-box-trim: trim-both;
  text-box-edge: cap alphabetic;
}
.ui-pill--neutral,
.role-pill.admin,
.role-pill.officer,
.role-pill.member,
.role-pill.recruit,
.status-pill.draft,
.status-pill.withdrawn {
  background: var(--badge-neutral-bg);
  color: var(--badge-neutral-fg);
}
.ui-pill--accent,
.status-pill.registration {
  background: var(--accent-soft);
  color: var(--accent-text);
}
.ui-pill--ok,
.status-pill.active,
.status-pill.accepted,
.status-pill.completed {
  background: var(--badge-win-bg);
  color: var(--badge-win-fg);
}
.ui-pill--warn,
.status-pill.pending,
.status-pill.in_progress {
  background: var(--badge-warn-bg);
  color: var(--badge-warn-fg);
}
.ui-pill--danger,
.status-pill.inactive,
.status-pill.rejected {
  background: var(--badge-loss-bg);
  color: var(--badge-loss-fg);
}
.ui-pill--info,
.status-pill.trial {
  background: var(--badge-info-bg);
  color: var(--badge-info-fg);
}
@supports not (text-box-trim: trim-both) {
  .ui-pill,
  .status-pill,
  .role-pill {
    padding-top: 0.08em;
  }
}
"#;

#[component]
pub fn Pill(#[props(default = PillTone::Neutral)] tone: PillTone, children: Element) -> Element {
    let t = tone.class();
    rsx! { span { class: "ui-pill {t}", {children} } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{
        assert_ink_clears_aa, assert_sizes_from_tokens, css_rule, has_decl, render, rules,
    };

    /// One selector per tone, then the fill and ink tokens its rule must set.
    const TONES: [(&str, &str, &str); 7] = [
        (".ui-pill", "--badge-bg", "--badge-fg"),
        (
            ".ui-pill--neutral",
            "--badge-neutral-bg",
            "--badge-neutral-fg",
        ),
        (".ui-pill--accent", "--accent-soft", "--accent-text"),
        (".ui-pill--ok", "--badge-win-bg", "--badge-win-fg"),
        (".ui-pill--warn", "--badge-warn-bg", "--badge-warn-fg"),
        (".ui-pill--danger", "--badge-loss-bg", "--badge-loss-fg"),
        (".ui-pill--info", "--badge-info-bg", "--badge-info-fg"),
    ];

    /// The rule whose selector list names `selector` exactly.
    fn rule_for(selector: &str) -> String {
        rules(PILL_CSS)
            .into_iter()
            .find(|(sel, body)| {
                body.contains("background:") && sel.split(',').any(|s| s.trim() == selector)
            })
            .unwrap_or_else(|| panic!("no colour rule for {selector}"))
            .1
    }

    /// PR 688 QA nit 1: small pills reach 4.5:1 in dark and light, on the
    /// product default accent and on every other brand the app derives.
    #[test]
    fn every_tone_clears_text_contrast_in_dark_and_light() {
        for (selector, fill, ink) in TONES {
            let rule = rule_for(selector);
            assert!(
                has_decl(&rule, &format!("background: var({fill})")),
                "{selector}: {rule}"
            );
            assert!(
                has_decl(&rule, &format!("color: var({ink})")),
                "{selector}: {rule}"
            );
            assert_ink_clears_aa(ink, fill, 4.5);
        }
    }

    /// The check is real: disabled text is about 3:1 by design and fails 4.5.
    #[test]
    #[should_panic(expected = "needs 4.5")]
    fn the_contrast_check_can_fail() {
        assert_ink_clears_aa("--text-disabled", "--surface", 4.5);
    }

    /// Every rule that paints a pill is one of the checked tones, so a new
    /// tone cannot ship without a contrast check.
    #[test]
    fn no_tone_is_left_unchecked() {
        let mut painted = 0;
        for (selector, body) in rules(PILL_CSS) {
            if !body.contains("color:") {
                continue;
            }
            painted += 1;
            let known = TONES.iter().any(|(_, fill, ink)| {
                has_decl(&body, &format!("background: var({fill})"))
                    && has_decl(&body, &format!("color: var({ink})"))
            });
            assert!(known, "unchecked colours in {selector}: {body}");
        }
        assert_eq!(painted, TONES.len());
        assert!(!PILL_CSS.contains("color-mix"), "soft fills are tokens");
    }

    #[test]
    fn older_class_names_keep_a_tone() {
        for (selector, fill) in [
            (".status-pill", "--badge-bg"),
            (".role-pill", "--badge-bg"),
            (".status-pill.pending", "--badge-warn-bg"),
            (".status-pill.in_progress", "--badge-warn-bg"),
            (".status-pill.active", "--badge-win-bg"),
            (".status-pill.accepted", "--badge-win-bg"),
            (".status-pill.completed", "--badge-win-bg"),
            (".status-pill.inactive", "--badge-loss-bg"),
            (".status-pill.rejected", "--badge-loss-bg"),
            (".status-pill.trial", "--badge-info-bg"),
            (".status-pill.registration", "--accent-soft"),
            (".status-pill.draft", "--badge-neutral-bg"),
            (".status-pill.withdrawn", "--badge-neutral-bg"),
            (".role-pill.admin", "--badge-neutral-bg"),
            (".role-pill.officer", "--badge-neutral-bg"),
            (".role-pill.member", "--badge-neutral-bg"),
            (".role-pill.recruit", "--badge-neutral-bg"),
        ] {
            let rule = rule_for(selector);
            assert!(
                has_decl(&rule, &format!("background: var({fill})")),
                "{selector}: {rule}"
            );
        }
        let common = crate::styles::common::CSS;
        assert!(
            !common.contains("-pill {") && !common.contains("-pill."),
            "pill rules live in PILL_CSS only"
        );
        // One pill API: no page writes a raw badge class, so none is styled.
        assert!(!PILL_CSS.contains(".badge"), "use Pill with a tone");
    }

    /// 11px is allowed only for an uppercase, weight 600 label.
    #[test]
    fn badge_type_is_uppercase_semibold_from_tokens() {
        assert_sizes_from_tokens(PILL_CSS);
        let base = css_rule(PILL_CSS, ".role-pill {");
        for decl in [
            "font-size: var(--badge-font-size)",
            "font-weight: var(--badge-font-weight)",
            "text-transform: var(--badge-font-transform)",
            "font-family: var(--badge-font)",
            "min-height: var(--badge-height)",
            "border-radius: var(--badge-radius)",
        ] {
            assert!(has_decl(&base, decl), "missing {decl}: {base}");
        }
    }

    #[test]
    fn pill_renders_its_tone_class() {
        fn app() -> Element {
            rsx! {
                Pill { "Pinned" }
                Pill { tone: PillTone::Ok, "Win" }
                Pill { tone: PillTone::Info, "Trial" }
            }
        }
        let html = render(app);
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--neutral\">Pinned</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--ok\">Win</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--info\">Trial</span>"),
            "{html}"
        );
    }
}
