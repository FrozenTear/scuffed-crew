use dioxus::prelude::*;
#[derive(Clone, Copy, PartialEq)]
pub enum PillTone {
    Neutral,
    Accent,
    Ok,
    Warn,
    Danger,
}
pub const PILL_CSS: &str = r#"
.ui-pill { font-family: var(--font-body); font-weight: 500; font-size: var(--text-xs);
  padding: 3px var(--space-2); border-radius: var(--radius-control); display: inline-block; }
.ui-pill--neutral { background: var(--surface-sunken); color: var(--text-3); }
.ui-pill--accent { background: var(--accent-soft); color: var(--accent-text); }
.ui-pill--ok { background: color-mix(in srgb, var(--win) 15%, transparent); color: var(--win); }
.ui-pill--warn { background: color-mix(in srgb, var(--warn) 18%, transparent); color: var(--warn); }
.ui-pill--danger { background: color-mix(in srgb, var(--loss) 15%, transparent); color: var(--loss); }
"#;
#[component]
pub fn Pill(#[props(default = PillTone::Neutral)] tone: PillTone, children: Element) -> Element {
    let t = match tone {
        PillTone::Neutral => "ui-pill--neutral",
        PillTone::Accent => "ui-pill--accent",
        PillTone::Ok => "ui-pill--ok",
        PillTone::Warn => "ui-pill--warn",
        PillTone::Danger => "ui-pill--danger",
    };
    rsx! { span { class: "ui-pill {t}", {children} } }
}
