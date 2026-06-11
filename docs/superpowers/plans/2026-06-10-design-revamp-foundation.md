# Design Revamp Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a token-based design system (light/dark), a shared `components/ui/` library, and a theming engine, then migrate the entire Dioxus app to the "Direction C — Clean Editorial" aesthetic.

**Architecture:** A single `theme/` module emits primitive + semantic CSS custom properties scoped by `[data-theme="light|dark"]`. A `ThemeProvider` sets that attribute (persisted to `localStorage`, defaulting to `prefers-color-scheme`). All visual values come from a `BrandConfig` seam (hardcoded now; settings-driven in a later sub-project). Pages compose from `components/ui/` primitives and reference only semantic tokens; a CI guard blocks raw hex in component CSS.

**Tech Stack:** Rust, Dioxus 0.7 (WASM), CSS custom properties, Google Fonts (Space Grotesk / Inter / JetBrains Mono).

**Spec:** `docs/superpowers/specs/2026-06-10-design-revamp-foundation-design.md`

**Conventions discovered in the codebase (follow these exactly):**
- Components are `#[component]` fns with typed props and `#[props(default)]` (see `components/section_header.rs`).
- Components are declared in `components/mod.rs` (`pub mod x;` + selective `pub use`).
- Each page injects its own CSS via `style { {PAGE_CSS} }` where `const PAGE_CSS: &str = r#"..."#;` lives in the page file.
- The app root (`main.rs` `App`) injects `style { {theme::THEME_CSS} }` and `style { {styles::common::CSS} }`, loads fonts via `document::Link`, and wraps `Router::<Route>{}` in `ToastProvider`.
- Tests for WASM components run with the `web` feature target where needed; pure-Rust helper tests run on host. Use `#[cfg(test)] mod tests` in-file.

**Verification commands used throughout:**
- Wasm build/lint: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
- Host tests (pure logic): `cargo test -p scuffed-app <name>`
- Format: `cargo fmt -p scuffed-app`

---

## Phase A — Foundation

### Task 1: Brand seam + theme token module

**Files:**
- Create: `crates/app/src/theme/mod.rs`
- Create: `crates/app/src/theme/brand.rs`
- Create: `crates/app/src/theme/tokens.rs`
- Delete (after migration of references): `crates/app/src/theme.rs` (currently a file, becomes a module dir)

This replaces the single `theme.rs` file with a `theme/` directory. `brand.rs` is the seam a later sub-project will make settings-driven. `tokens.rs` produces the full CSS string.

- [ ] **Step 1: Create the brand seam**

`crates/app/src/theme/brand.rs`:
```rust
//! The single source of brand-configurable values. Hardcoded to Scuffed Crew
//! defaults today; a later sub-project swaps `current()` to read /api/settings.
//! Nothing else in the app should hardcode brand colors — they consume this.

#[derive(Clone, PartialEq)]
pub struct BrandConfig {
    /// Accent in dark mode (hex).
    pub accent_dark: &'static str,
    /// Accent in light mode (hex).
    pub accent_light: &'static str,
    /// Soft accent fill, dark mode (rgba).
    pub accent_soft_dark: &'static str,
    /// Soft accent fill, light mode (rgba).
    pub accent_soft_light: &'static str,
}

impl BrandConfig {
    pub const SCUFFED: BrandConfig = BrandConfig {
        accent_dark: "#8f73ff",
        accent_light: "#6d4aff",
        accent_soft_dark: "rgba(143,115,255,.17)",
        accent_soft_light: "rgba(109,74,255,.10)",
    };
}

/// The active brand config. Single call-site that the settings sub-project rewires.
pub fn current() -> BrandConfig {
    BrandConfig::SCUFFED
}
```

- [ ] **Step 2: Write a failing test for token emission**

`crates/app/src/theme/tokens.rs`:
```rust
use crate::theme::brand::{current, BrandConfig};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_both_theme_scopes_and_uses_brand_accent() {
        let css = theme_css(&BrandConfig::SCUFFED);
        assert!(css.contains("[data-theme=\"dark\"]"));
        assert!(css.contains("[data-theme=\"light\"]"));
        // Brand accent threaded through, not hardcoded duplicate:
        assert!(css.contains("#8f73ff"));
        assert!(css.contains("#6d4aff"));
        // Semantic tokens present:
        assert!(css.contains("--bg:"));
        assert!(css.contains("--text-2:"));
        assert!(css.contains("--space-1:"));
    }
}
```

- [ ] **Step 3: Run the test, verify it fails**

Run: `cargo test -p scuffed-app emits_both_theme_scopes`
Expected: FAIL — `theme_css` not found.

- [ ] **Step 4: Implement `theme_css`**

Prepend to `crates/app/src/theme/tokens.rs` (above the test module):
```rust
/// Build the full design-system stylesheet (primitives + both semantic scopes)
/// from a brand config. The ONLY place raw color/scale literals live.
pub fn theme_css(brand: &BrandConfig) -> String {
    format!(
        r#"
:root {{
  --font-head: 'Space Grotesk', system-ui, sans-serif;
  --font-body: 'Inter', system-ui, sans-serif;
  --font-mono: 'JetBrains Mono', ui-monospace, monospace;

  --text-xs: 11px;  --text-sm: 12.5px; --text-base: 14px; --text-lg: 15.5px;
  --text-xl: 18px;  --text-2xl: 21px;  --text-3xl: 30px;

  --space-1: 4px;  --space-2: 8px;  --space-3: 12px; --space-4: 16px;
  --space-6: 24px; --space-8: 32px; --space-12: 48px;

  --radius-sm: 7px; --radius-md: 9px; --radius-lg: 12px; --radius-pill: 999px;
}}

[data-theme="dark"] {{
  --bg: #17171d; --surface: #1f1f27; --surface-2: #282831; --border: #353541;
  --text: #f4f4f8; --text-2: #c1c1cd; --text-3: #9696a3;
  --accent: {accent_d}; --accent-fg: #ffffff; --accent-soft: {soft_d};
  --ok: #46d8a4; --warn: #fbbf24; --danger: #f06a6a;
}}

[data-theme="light"] {{
  --bg: #f7f7f9; --surface: #ffffff; --surface-2: #f0f0f4; --border: #e3e3e9;
  --text: #16161c; --text-2: #545462; --text-3: #83838f;
  --accent: {accent_l}; --accent-fg: #ffffff; --accent-soft: {soft_l};
  --ok: #0ea66e; --warn: #c2830a; --danger: #d63031;
}}

html, body {{
  background: var(--bg); color: var(--text);
  font-family: var(--font-body); font-size: var(--text-base);
}}
"#,
        accent_d = brand.accent_dark,
        accent_l = brand.accent_light,
        soft_d = brand.accent_soft_dark,
        soft_l = brand.accent_soft_light,
    )
}

/// Convenience for the app root.
pub fn theme_css_current() -> String {
    theme_css(&current())
}
```

- [ ] **Step 5: Wire the module**

`crates/app/src/theme/mod.rs`:
```rust
pub mod brand;
pub mod tokens;

pub use tokens::theme_css_current;
```
Then delete the old file: `git rm crates/app/src/theme.rs` (its `THEME_CSS` is replaced; references updated in Task 3).

- [ ] **Step 6: Run the test, verify it passes**

Run: `cargo test -p scuffed-app emits_both_theme_scopes`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/theme/
git commit -m "feat(app): token module with light/dark scopes and brand seam"
```

---

### Task 2: ThemeProvider + ThemeToggle

**Files:**
- Create: `crates/app/src/theme/provider.rs`
- Modify: `crates/app/src/theme/mod.rs`

- [ ] **Step 1: Implement the provider and toggle**

`crates/app/src/theme/provider.rs`:
```rust
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    fn as_attr(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

#[derive(Clone, Copy)]
pub struct ThemeCtx {
    pub mode: Signal<ThemeMode>,
}

/// Read persisted choice from localStorage, else fall back to prefers-color-scheme.
fn initial_mode() -> ThemeMode {
    #[cfg(feature = "web")]
    {
        if let Some(win) = web_sys::window() {
            if let Ok(Some(storage)) = win.local_storage() {
                if let Ok(Some(v)) = storage.get_item("sc-theme") {
                    return if v == "light" { ThemeMode::Light } else { ThemeMode::Dark };
                }
            }
            if let Ok(Some(mql)) = win.match_media("(prefers-color-scheme: light)") {
                if mql.matches() {
                    return ThemeMode::Light;
                }
            }
        }
    }
    ThemeMode::Dark
}

fn apply(mode: ThemeMode) {
    #[cfg(feature = "web")]
    {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(el) = doc.document_element() {
                let _ = el.set_attribute("data-theme", mode.as_attr());
            }
            if let Ok(Some(storage)) = web_sys::window().unwrap().local_storage() {
                let _ = storage.set_item("sc-theme", mode.as_attr());
            }
        }
    }
}

#[component]
pub fn ThemeProvider(children: Element) -> Element {
    let mode = use_signal(initial_mode);
    use_context_provider(|| ThemeCtx { mode });
    use_effect(move || apply(mode()));
    rsx! { {children} }
}

#[component]
pub fn ThemeToggle() -> Element {
    let mut ctx = use_context::<ThemeCtx>();
    let is_dark = (ctx.mode)() == ThemeMode::Dark;
    rsx! {
        button {
            class: "theme-toggle",
            "aria-label": "Toggle color theme",
            onclick: move |_| {
                let next = if is_dark { ThemeMode::Light } else { ThemeMode::Dark };
                ctx.mode.set(next);
            },
            if is_dark { "\u{2600}" } else { "\u{263d}" }
        }
    }
}
```

- [ ] **Step 2: Export it**

In `crates/app/src/theme/mod.rs` add:
```rust
pub mod provider;
pub use provider::{ThemeCtx, ThemeMode, ThemeProvider, ThemeToggle};
```

- [ ] **Step 3: Verify it compiles for wasm**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/theme/
git commit -m "feat(app): ThemeProvider + ThemeToggle (localStorage + prefers-color-scheme)"
```

---

### Task 3: Wire foundation into the app root

**Files:**
- Modify: `crates/app/src/main.rs:46-72` (the `rsx!` in `App`)

- [ ] **Step 1: Replace fonts, theme CSS, and providers**

In `App`'s `rsx!`, replace the Bebas/Rajdhani font `document::Link`, the `style { {theme::THEME_CSS} }` line, and the `ToastProvider { Router... }` wrapper with:
```rust
        document::Link {
            rel: "stylesheet",
            href: "https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Space+Grotesk:wght@500;600;700&family=JetBrains+Mono:wght@500&display=swap",
        }
        style { {theme::theme_css_current()} }
        style { {styles::common::CSS} }
        theme::ThemeProvider {
            ToastProvider {
                Router::<Route> {}
            }
        }
```
Keep the existing `document::Stylesheet` (tailwind) and the two `preconnect` links above.

- [ ] **Step 2: Verify it compiles for wasm**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: clean compile (no remaining reference to `theme::THEME_CSS`).

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/main.rs
git commit -m "feat(app): load new fonts + tokens, mount ThemeProvider at root"
```

---

### Task 4: `Button` component (worked example for the library)

**Files:**
- Create: `crates/app/src/components/ui/mod.rs`
- Create: `crates/app/src/components/ui/button.rs`
- Modify: `crates/app/src/components/mod.rs`

- [ ] **Step 1: Write a failing test for the variant class mapping**

`crates/app/src/components/ui/button.rs`:
```rust
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum BtnVariant { Primary, Ghost, Danger }

#[derive(Clone, Copy, PartialEq)]
pub enum BtnSize { Sm, Md }

fn class_for(variant: BtnVariant, size: BtnSize) -> String {
    let v = match variant { BtnVariant::Primary => "ui-btn--primary", BtnVariant::Ghost => "ui-btn--ghost", BtnVariant::Danger => "ui-btn--danger" };
    let s = match size { BtnSize::Sm => "ui-btn--sm", BtnSize::Md => "ui-btn--md" };
    format!("ui-btn {v} {s}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_variant_and_size_to_classes() {
        assert_eq!(class_for(BtnVariant::Primary, BtnSize::Md), "ui-btn ui-btn--primary ui-btn--md");
        assert_eq!(class_for(BtnVariant::Ghost, BtnSize::Sm), "ui-btn ui-btn--ghost ui-btn--sm");
    }
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cargo test -p scuffed-app maps_variant_and_size`
Expected: FAIL — module not found.

- [ ] **Step 3: Add the component and its tokenized CSS**

Append to `button.rs` (above the test module):
```rust
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
            onclick: move |e| { if let Some(h) = &onclick { h.call(e); } },
            {children}
        }
    }
}
```

- [ ] **Step 4: Create the ui module and register CSS aggregation**

`crates/app/src/components/ui/mod.rs`:
```rust
pub mod button;
pub use button::{BtnSize, BtnVariant, Button};

/// Concatenated CSS for every ui/ component, injected once at the app root.
pub fn ui_css() -> String {
    [button::BUTTON_CSS].concat()
}
```
In `crates/app/src/components/mod.rs` add `pub mod ui;`.

- [ ] **Step 5: Inject `ui_css()` at the root**

In `crates/app/src/main.rs`, after the `style { {styles::common::CSS} }` line, add:
```rust
        style { {components::ui::ui_css()} }
```

- [ ] **Step 6: Run test + wasm lint**

Run: `cargo test -p scuffed-app maps_variant_and_size && cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: PASS + clean.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/components/ui/ crates/app/src/components/mod.rs crates/app/src/main.rs
git commit -m "feat(app/ui): Button component + ui_css aggregation"
```

---

### Task 5: `Card`, `Pill`, `Label`, `PageShell`

**Files:**
- Create: `crates/app/src/components/ui/card.rs`, `pill.rs`, `label.rs`, `shell.rs`
- Modify: `crates/app/src/components/ui/mod.rs`

- [ ] **Step 1: Implement the four components**

`card.rs`:
```rust
use dioxus::prelude::*;
pub const CARD_CSS: &str = r#"
.ui-card { background: var(--surface); border: 1px solid var(--border);
  border-radius: var(--radius-lg); padding: var(--space-4); }
.ui-card--accent { border-left: 2px solid var(--accent); }
"#;
#[component]
pub fn Card(#[props(default = false)] accent_edge: bool, children: Element) -> Element {
    let class = if accent_edge { "ui-card ui-card--accent" } else { "ui-card" };
    rsx! { div { class: "{class}", {children} } }
}
```

`pill.rs`:
```rust
use dioxus::prelude::*;
#[derive(Clone, Copy, PartialEq)]
pub enum PillTone { Neutral, Accent, Ok, Warn, Danger }
pub const PILL_CSS: &str = r#"
.ui-pill { font-family: var(--font-body); font-weight: 500; font-size: var(--text-xs);
  padding: 3px var(--space-2); border-radius: var(--radius-sm); display: inline-block; }
.ui-pill--neutral { background: var(--surface-2); color: var(--text-2); }
.ui-pill--accent { background: var(--accent-soft); color: var(--accent); }
.ui-pill--ok { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.ui-pill--warn { background: color-mix(in srgb, var(--warn) 18%, transparent); color: var(--warn); }
.ui-pill--danger { background: color-mix(in srgb, var(--danger) 15%, transparent); color: var(--danger); }
"#;
#[component]
pub fn Pill(#[props(default = PillTone::Neutral)] tone: PillTone, children: Element) -> Element {
    let t = match tone {
        PillTone::Neutral => "ui-pill--neutral", PillTone::Accent => "ui-pill--accent",
        PillTone::Ok => "ui-pill--ok", PillTone::Warn => "ui-pill--warn", PillTone::Danger => "ui-pill--danger",
    };
    rsx! { span { class: "ui-pill {t}", {children} } }
}
```

`label.rs`:
```rust
use dioxus::prelude::*;
pub const LABEL_CSS: &str = r#"
.ui-label { font-family: var(--font-mono); font-weight: 500; font-size: var(--text-xs);
  letter-spacing: .08em; text-transform: uppercase; color: var(--text-3); }
"#;
#[component]
pub fn Label(children: Element) -> Element { rsx! { span { class: "ui-label", {children} } } }
```

`shell.rs`:
```rust
use dioxus::prelude::*;
pub const SHELL_CSS: &str = r#"
.ui-shell { max-width: 1100px; margin: 0 auto; padding: var(--space-6) var(--space-4) var(--space-12); }
"#;
#[component]
pub fn PageShell(children: Element) -> Element { rsx! { div { class: "ui-shell", {children} } } }
```

- [ ] **Step 2: Register them**

In `components/ui/mod.rs`:
```rust
pub mod card; pub mod pill; pub mod label; pub mod shell;
pub use card::Card;
pub use pill::{Pill, PillTone};
pub use label::Label;
pub use shell::PageShell;
```
And extend `ui_css()`:
```rust
pub fn ui_css() -> String {
    [button::BUTTON_CSS, card::CARD_CSS, pill::PILL_CSS, label::LABEL_CSS, shell::SHELL_CSS].concat()
}
```

- [ ] **Step 3: Verify wasm lint**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/components/ui/
git commit -m "feat(app/ui): Card, Pill, Label, PageShell"
```

---

### Task 6: `Input`, `Textarea`, `Select`

**Files:**
- Create: `crates/app/src/components/ui/field.rs`
- Modify: `crates/app/src/components/ui/mod.rs`

- [ ] **Step 1: Implement the form fields**

`field.rs`:
```rust
use dioxus::prelude::*;
pub const FIELD_CSS: &str = r#"
.ui-field { width: 100%; font-family: var(--font-body); font-size: var(--text-sm); font-weight: 500;
  background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-md);
  padding: var(--space-2) var(--space-3); color: var(--text); }
.ui-field::placeholder { color: var(--text-3); }
.ui-field:focus { outline: none; border-color: var(--accent); }
.ui-field--invalid { border-color: var(--danger); }
textarea.ui-field { resize: vertical; min-height: 80px; }
"#;

fn field_class(invalid: bool) -> &'static str { if invalid { "ui-field ui-field--invalid" } else { "ui-field" } }

#[component]
pub fn Input(
    value: String,
    #[props(default)] placeholder: String,
    #[props(default = false)] invalid: bool,
    #[props(default = false)] disabled: bool,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
) -> Element {
    rsx! {
        input { class: field_class(invalid), value, placeholder, disabled,
            oninput: move |e| { if let Some(h) = &oninput { h.call(e); } } }
    }
}

#[component]
pub fn Textarea(
    value: String,
    #[props(default)] placeholder: String,
    #[props(default = false)] invalid: bool,
    #[props(default)] oninput: Option<EventHandler<FormEvent>>,
) -> Element {
    rsx! {
        textarea { class: field_class(invalid), placeholder,
            oninput: move |e| { if let Some(h) = &oninput { h.call(e); } }, "{value}" }
    }
}

#[component]
pub fn Select(
    #[props(default = false)] invalid: bool,
    #[props(default)] onchange: Option<EventHandler<FormEvent>>,
    children: Element,
) -> Element {
    rsx! {
        select { class: field_class(invalid),
            onchange: move |e| { if let Some(h) = &onchange { h.call(e); } }, {children} }
    }
}
```

- [ ] **Step 2: Register + extend ui_css**

In `components/ui/mod.rs`:
```rust
pub mod field;
pub use field::{Input, Select, Textarea};
```
Add `field::FIELD_CSS` to the `ui_css()` array.

- [ ] **Step 3: Verify wasm lint**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/components/ui/
git commit -m "feat(app/ui): Input, Textarea, Select form fields"
```

---

### Task 7: `EmptyState`, `Spinner` + refactor existing shared components to tokens

**Files:**
- Create: `crates/app/src/components/ui/feedback.rs`
- Modify: `crates/app/src/components/ui/mod.rs`
- Modify: `crates/app/src/components/modal.rs`, `crates/app/src/components/toast.rs`, `crates/app/src/components/section_header.rs`, `crates/app/src/styles/common.rs`

- [ ] **Step 1: Implement EmptyState + Spinner**

`feedback.rs`:
```rust
use dioxus::prelude::*;
pub const FEEDBACK_CSS: &str = r#"
.ui-empty { text-align: center; color: var(--text-2); padding: var(--space-8) var(--space-4); }
.ui-empty h3 { font-family: var(--font-head); color: var(--text); font-size: var(--text-lg); margin-bottom: var(--space-2); }
.ui-empty p { font-size: var(--text-sm); }
.ui-spinner { width: 18px; height: 18px; border: 2px solid var(--border);
  border-top-color: var(--accent); border-radius: 50%; animation: ui-spin .7s linear infinite; }
@keyframes ui-spin { to { transform: rotate(360deg); } }
"#;
#[component]
pub fn EmptyState(title: String, #[props(default)] message: String) -> Element {
    rsx! { div { class: "ui-empty", h3 { "{title}" } p { "{message}" } } }
}
#[component]
pub fn Spinner() -> Element { rsx! { div { class: "ui-spinner" } } }
```
Register in `mod.rs` (`pub mod feedback; pub use feedback::{EmptyState, Spinner};` + add `feedback::FEEDBACK_CSS` to `ui_css()`).

- [ ] **Step 2: Retokenize the existing shared components**

In `components/modal.rs`, `components/toast.rs`, `components/section_header.rs`, and `styles/common.rs`, replace every legacy variable (`--bg-card`, `--bg-elevated`, `--accent-soft`, `--text-bright`, `--text-primary`, `--text-secondary`, `--text-muted`, `--border-light`, `--bg-void`, `--bg-surface`, `--bg-card-alt`, `--danger`, `--success`, `--warning`) with the new semantic tokens per this mapping:
```
--bg-void        -> --bg
--bg-surface     -> --surface
--bg-card        -> --surface
--bg-card-alt    -> --surface-2
--bg-elevated    -> --surface-2
--text-bright    -> --text
--text-primary   -> --text
--text-secondary -> --text-2
--text-muted     -> --text-3
--border-light   -> --border
--accent-bright  -> --accent
--accent-glow    -> --accent-soft
--success        -> --ok
--warning        -> --warn
(--accent, --accent-soft, --border, --danger keep their names)
```
Also change the `SectionHeader` `color` prop default from `"purple"` to `"accent"` and update its `.sec-label-*` CSS to use `--accent`.

- [ ] **Step 3: Verify wasm lint**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/components/ crates/app/src/styles/common.rs
git commit -m "feat(app/ui): EmptyState + Spinner; retokenize modal/toast/section_header/common"
```

---

### Task 8: CI raw-hex guard

**Files:**
- Create: `scripts/check-design-tokens.sh`
- Modify: `.github/workflows/ci.yml` (the `dep-guardrails` job)

- [ ] **Step 1: Write the guard**

`scripts/check-design-tokens.sh`:
```bash
#!/usr/bin/env bash
# CI guardrail: component CSS must use semantic tokens, never raw colors.
# The ONLY file allowed to contain raw hex / rgb literals is the token module.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_SRC="$REPO_ROOT/crates/app/src"

# Allowed: the token source and the brand seam.
ALLOW_REGEX='theme/tokens.rs|theme/brand.rs'

violations=$(grep -rnE '#[0-9a-fA-F]{3,8}\b|rgba?\(' "$APP_SRC" \
  --include='*.rs' | grep -vE "$ALLOW_REGEX" || true)

if [[ -n "$violations" ]]; then
  echo "ERROR: raw color literals found outside the token module:"
  echo "$violations"
  echo ""
  echo "Use semantic CSS variables (var(--accent), var(--text-2), ...) instead."
  exit 1
fi
echo "OK: no raw color literals in component CSS."
```
Make executable: `chmod +x scripts/check-design-tokens.sh`.

- [ ] **Step 2: Run it — expect it to FAIL initially (pages not migrated yet)**

Run: `bash scripts/check-design-tokens.sh`
Expected: FAIL, listing the still-unmigrated pages. This is correct — the guard goes green as migration completes. **Do not add it to CI yet.**

- [ ] **Step 3: Commit the script only**

```bash
git add scripts/check-design-tokens.sh
git commit -m "chore(ci): add design-token guard script (not yet wired into CI)"
```

> The CI wiring happens in Task 16 once all pages are migrated and the guard passes.

---

## Phase B — Shell & Layouts

### Task 9: Migrate the three layouts + add the theme toggle

**Files:**
- Modify: `crates/app/src/layouts/public.rs`, `crates/app/src/layouts/admin.rs`, `crates/app/src/layouts/strategy.rs`

- [ ] **Step 1: Apply the Migration Recipe (see Task 10) to each layout file, plus:**
- Replace the hand-rolled nav/footer typography with `--font-head`/`--font-body` and semantic tokens.
- Insert `theme::ThemeToggle {}` into the public nav (right side, before/after the auth links) and the admin sidebar header.
- Convert the brand wordmark to `--font-head`.

- [ ] **Step 2: Verify wasm lint + manual both-modes check**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Then run the app (`cd crates/app && dx serve`) and confirm: nav renders, toggle flips light↔dark and persists across reload, no unreadable text in either mode.

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/layouts/
git commit -m "feat(app): migrate layouts to design system + theme toggle in nav"
```

---

## Phase C — Page Migration

### Task 10: Migration Recipe (reference procedure) + first content batch

**The Migration Recipe** — apply these exact steps to each page/component file listed in Tasks 10–15:

1. In the file's `const PAGE_CSS` (or component CSS), replace every legacy variable using the mapping table in Task 7 Step 2, and replace every raw hex/`rgba()` literal (e.g. `#7c3aed`, `#ff6a00`, `rgba(124,58,237,…)`) with the nearest semantic token (`var(--accent)`, `var(--accent-soft)`, etc.).
2. Replace hand-rolled buttons with `Button {…}`, cards/panels with `Card {…}`, status chips with `Pill {…}`, form inputs with `Input`/`Textarea`/`Select`, empty-list markup with `EmptyState {…}`, and section labels with `Label {…}` — importing from `crate::components::ui`.
3. Replace any legacy font references (`Bebas Neue`, `Rajdhani`, `Source Sans 3`, `DM Mono`) with the token font stacks (`var(--font-head)`, `var(--font-body)`, `var(--font-mono)`).
4. Where a page hardcodes a fixed page width/padding, wrap content in `PageShell {…}`.
5. After edits: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`, then visually verify the page in both light and dark.

**Files (content batch 1):**
- Modify: `crates/app/src/pages/home.rs`, `members.rs`, `member_profile.rs`, `news.rs`, `apply.rs`

- [ ] **Step 1: Apply the Migration Recipe to home.rs**
- [ ] **Step 2: Apply the Migration Recipe to members.rs and member_profile.rs**
- [ ] **Step 3: Apply the Migration Recipe to news.rs and apply.rs**
- [ ] **Step 4: Wasm lint + both-modes visual check (all five pages)**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/pages/home.rs crates/app/src/pages/members.rs crates/app/src/pages/member_profile.rs crates/app/src/pages/news.rs crates/app/src/pages/apply.rs
git commit -m "feat(app): migrate home/members/news/apply to design system"
```

---

### Task 11: Content batch 2 — community, feed, blog, polls, events

**Files:**
- Modify: `crates/app/src/pages/community.rs`, `feed.rs`, `blog.rs`, `blog_post.rs`, `blog_article.rs`, `polls.rs`, `events.rs`
- Modify: `crates/app/src/components/post/card.rs`, `post/compose.rs`, `poll/card.rs`, `poll/poll_card.rs`, `poll/create.rs`, `poll/poll_create.rs`

- [ ] **Step 1: Apply the Migration Recipe to the page files (community, feed, blog×3, polls, events)**
- [ ] **Step 2: Apply the Migration Recipe to the post/ and poll/ components (these hold known hardcoded `#7c3aed`)**
- [ ] **Step 3: Wasm lint + both-modes visual check**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/pages/community.rs crates/app/src/pages/feed.rs crates/app/src/pages/blog.rs crates/app/src/pages/blog_post.rs crates/app/src/pages/blog_article.rs crates/app/src/pages/polls.rs crates/app/src/pages/events.rs crates/app/src/components/post/ crates/app/src/components/poll/
git commit -m "feat(app): migrate community/feed/blog/polls/events + post/poll components"
```

---

### Task 12: Content batch 3 — wiki, forum, tournaments, scrims, stats, dm, identity

**Files:**
- Modify: `crates/app/src/pages/wiki.rs`, `wiki_page.rs`, `forum.rs`, `forum_thread.rs`, `tournaments.rs`, `tournament.rs`, `scrims.rs`, `stats.rs`, `stats_member.rs`, `stats_tokens.rs`, `dm.rs`, `identity.rs`, `not_found.rs`
- Modify: `crates/app/src/components/dm/reply_input.rs`, `dm/compose.rs`, `dm/conversation_list.rs`, `dm/message_thread.rs`, `crates/app/src/components/chat/reaction_bar.rs`, `chat/chat_widget.rs`, `chat/compose_box.rs`, `chat/message_list.rs`, `chat/relay_status.rs`, `crates/app/src/components/bracket/styles.rs`, `crates/app/src/components/charts.rs`

- [ ] **Step 1: Apply the Migration Recipe to the wiki/forum/tournament/scrims pages**
- [ ] **Step 2: Apply the Migration Recipe to the stats/dm/identity/not_found pages**
- [ ] **Step 3: Apply the Migration Recipe to the dm/, chat/, bracket/, charts components (hold known hardcoded `#7c3aed`)**
- [ ] **Step 4: Wasm lint + both-modes visual check**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/pages/ crates/app/src/components/dm/ crates/app/src/components/chat/ crates/app/src/components/bracket/ crates/app/src/components/charts.rs
git commit -m "feat(app): migrate wiki/forum/tournaments/stats/dm/identity + chat/dm/bracket components"
```

---

### Task 13: Admin batch 1 — shell, dashboard, members, teams, games, matches

**Files:**
- Modify: `crates/app/src/styles/admin.rs`
- Modify: `crates/app/src/components/admin_shared.rs`
- Modify: `crates/app/src/pages/admin/dashboard.rs`, `members.rs`, `teams.rs`, `games.rs`, `matches.rs`

- [ ] **Step 1: Apply the Migration Recipe to `styles/admin.rs` and `admin_shared.rs` first** (these define the shared admin table/form/dialog CSS every admin page inherits).
- [ ] **Step 2: Apply the Migration Recipe to dashboard, members, teams, games, matches**
- [ ] **Step 3: Wasm lint + both-modes visual check**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/styles/admin.rs crates/app/src/components/admin_shared.rs crates/app/src/pages/admin/dashboard.rs crates/app/src/pages/admin/members.rs crates/app/src/pages/admin/teams.rs crates/app/src/pages/admin/games.rs crates/app/src/pages/admin/matches.rs
git commit -m "feat(app): migrate admin shell + dashboard/members/teams/games/matches"
```

---

### Task 14: Admin batch 2 — remaining admin pages

**Files:**
- Modify: `crates/app/src/pages/admin/tournaments.rs`, `schedule.rs`, `announcements.rs`, `articles.rs`, `moderation.rs`, `audit_log.rs`, `applications.rs`, `settings.rs`, `relay.rs`

- [ ] **Step 1: Apply the Migration Recipe to tournaments, schedule, announcements, articles, moderation**
- [ ] **Step 2: Apply the Migration Recipe to audit_log, applications, settings, relay**
- [ ] **Step 3: Wasm lint + both-modes visual check**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/pages/admin/
git commit -m "feat(app): migrate remaining admin pages to design system"
```

---

### Task 15: Strategy section — browse/meta pages + editor + panels

**Files:**
- Modify: `crates/app/src/pages/strategy/browse.rs`, `my_strategies.rs`, `heroes.rs`, `meta.rs`, `patch_notes.rs`, `editor.rs`
- Modify: `crates/app/src/components/strategy/properties_panel.rs`, `team_panel.rs`, `timeline.rs`, `toolbar.rs`, `hero_picker.rs`, `map_canvas_css.rs`, `connection_status.rs`

- [ ] **Step 1: Apply the Migration Recipe to the strategy list pages (browse, my_strategies, heroes, meta, patch_notes)**

- [ ] **Step 2: Migrate the editor and wire its real panels**

In `crates/app/src/pages/strategy/editor.rs`: apply the Recipe, then replace the three `.panel-placeholder` stub divs (TeamPanel / PropertiesPanel / Timeline) with the real components `components::strategy::{TeamPanel, PropertiesPanel, Timeline}` (these files exist). Remove the `.panel-placeholder` CSS block.

- [ ] **Step 3: Convert the strategy orange to a per-section accent override (not raw hex)**

In `theme/tokens.rs`, add to the emitted CSS a section override block (keep it in the token module so it stays the single source of color truth):
```css
[data-accent="strategy"] { --accent: #ff7a1a; --accent-soft: rgba(255,122,26,.16); }
```
Then in `layouts/strategy.rs` set `div { "data-accent": "strategy", ... }` on the strategy layout root, and remove the hardcoded `#ff6a00` from `map_canvas.rs` / `editor.rs`, replacing canvas stroke colors that must stay literal (canvas 2D API needs concrete strings) by reading the computed `--accent` value via the existing canvas color plumbing, or passing the strategy accent constant from `theme::tokens`. Add a `pub const STRATEGY_ACCENT: &str = "#ff7a1a";` to `theme/tokens.rs` for canvas use so no raw hex lives in component files.

- [ ] **Step 4: Apply the Recipe to the remaining strategy components (properties_panel, team_panel, timeline, toolbar, hero_picker, map_canvas_css, connection_status)**

- [ ] **Step 5: Wasm lint + both-modes visual check of the full strategy section**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/pages/strategy/ crates/app/src/components/strategy/ crates/app/src/theme/tokens.rs crates/app/src/layouts/strategy.rs
git commit -m "feat(app): migrate strategy section, wire real editor panels, tokenize accent"
```

---

### Task 16: Enable the token guard in CI + final verification

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Confirm the guard now passes**

Run: `bash scripts/check-design-tokens.sh`
Expected: `OK: no raw color literals in component CSS.` (If any remain, fix those files with the Recipe before continuing.)

- [ ] **Step 2: Wire it into CI**

In `.github/workflows/ci.yml`, in the `dep-guardrails` job `steps:`, add after the existing check:
```yaml
      - name: Check design tokens (no raw hex in component CSS)
        run: bash scripts/check-design-tokens.sh
```

- [ ] **Step 3: Full verification**

Run:
```
cargo fmt --all
cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings
cargo clippy --workspace --exclude scuffed-app --exclude scuffed-stat-tracker -- -D warnings
cargo test -p scuffed-app
```
Expected: all clean/pass.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "chore(ci): enforce design-token guard now that migration is complete"
```

---

## Self-Review Notes

- **Spec coverage:** token architecture (T1), semantic light/dark values (T1), hard rule + CI guard (T8/T16), component library inventory (T4–T7), ThemeProvider/toggle + brand seam (T1–T3), full-app migration phases A/B/C (T9–T15), strategy placeholder removal + orange→accent override (T15), testing (per-task lint + smoke tests + guard). All spec sections map to tasks.
- **Type consistency:** `BrandConfig`/`current()`/`theme_css`/`theme_css_current` (T1), `ThemeProvider`/`ThemeToggle`/`ThemeCtx`/`ThemeMode` (T2), `ui_css()` aggregation extended in T4–T7, `Button`/`Card`/`Pill`/`Label`/`PageShell`/`Input`/`Textarea`/`Select`/`EmptyState`/`Spinner` names consistent across migration tasks.
- **Repetition:** the per-page migration is genuinely mechanical; the fully-specified Migration Recipe (T10) is the single procedure, with each batch task listing exact files + page-specific notes (post/poll/dm/chat hold the known hardcoded `#7c3aed`; strategy holds `#ff6a00`).
