# Design Revamp — Sub-project 1: Design System Foundation + Direction C Redesign

**Date:** 2026-06-10
**Status:** Approved design, pending implementation plan
**Crate:** `crates/app` (Dioxus 0.7 WASM)

## Context

The Scuffed Crew app's styling is ~3,500 lines of per-component `const CSS: &str` blocks across 61 files, on top of an ad-hoc token set in `crates/app/src/theme.rs`. There are 40+ hardcoded brand hex values bypassing tokens, no shared UI components (~600 lines of duplicated card/form/button CSS), no type/spacing scale, and no light mode. Tailwind v4 is loaded but unused.

This is the first of three sequenced sub-projects toward a modern, white-labelable platform that other clans can run as their backend:

1. **(this spec) Design System Foundation + full Direction C redesign** — token architecture, shared component library, light/dark theming engine, and the new aesthetic applied across the entire app.
2. **Brand & Theme settings** (future) — extend `site_settings` + admin UI with org name, logo, palette, font, density; wire through the engine; `primary_domain`/NIP-05 externalization.
3. **Layout customisation** (future) — section toggle/reorder, nav editor, homepage blocks.

Sub-project 1 is the prerequisite foundation for 2 and 3, and delivers the visual revamp on its own. **This spec covers Sub-project 1 only.**

## Goals

- Replace the ad-hoc styling with a single token system (primitive + semantic), enforced so components never reference raw hex.
- Ship a shared UI component library that eliminates duplicated card/form/button CSS.
- Add first-class light + dark modes with a user toggle, defaulting to `prefers-color-scheme`.
- Apply the **Direction C — Clean Editorial** aesthetic (locked palette/type below) across the entire app (all public + admin + strategy pages).
- Build the theming *seam* (one config source for brand values) so Sub-project 2 can make it settings-driven without touching pages.

## Non-goals (deferred to SP2/SP3)

- Admin settings UI for branding, logo upload.
- Externalizing org name / `scuffed.gg` domain / NIP-05 / copy strings.
- Per-clan layout customisation (section toggle/reorder, nav editor).
- Multi-tenancy (org_id scoping).
- Changing game-specific data models (heroes/maps/modes).

## Aesthetic Direction — locked

**Direction C "Clean Editorial":** content-first, whitespace-led, neutral palette with a single accent. Calm modern community-platform feel (Linear/Vercel energy), not the old esports/clan DNA. No film grain, no glow effects.

**Typography:**
- Headings: `Space Grotesk` (500/600/700)
- Body/UI: `Inter` (400/500/600/700)
- Labels/mono: `JetBrains Mono` (500)
- Replaces the legacy Bebas Neue / Rajdhani / Source Sans 3 / DM Mono stack.

**Shape language:** radii 7–12px; soft 1px borders; real whitespace; no gradients on surfaces.

## Token Architecture

New module `crates/app/src/theme/` (replacing `crates/app/src/theme.rs`). Tokens are CSS custom properties emitted into a single `<style>` at the app root.

### Primitive tokens (raw scales, mode-independent)

- Neutral ramp: `--n-0` … `--n-12`
- Accent ramp: `--accent-50` … `--accent-700`
- Semantic status: ok / warn / danger base hues
- Type scale: `--text-xs` (11px), `--text-sm` (12.5px), `--text-base` (14px), `--text-lg` (15.5px), `--text-xl` (18px), `--text-2xl` (21px), `--text-3xl` (30px)
- Spacing scale (4px base): `--space-1` (4px) … `--space-12` (48px)
- Radii: `--radius-sm` (7px), `--radius-md` (9px), `--radius-lg` (12px), `--radius-pill` (999px)
- Font stacks: `--font-head`, `--font-body`, `--font-mono`

### Semantic tokens (what components use)

Defined twice — under `[data-theme="dark"]` and `[data-theme="light"]` — with the locked values below:

| Token | Dark | Light |
|-------|------|-------|
| `--bg` | `#17171d` | `#f7f7f9` |
| `--surface` | `#1f1f27` | `#ffffff` |
| `--surface-2` | `#282831` | `#f0f0f4` |
| `--border` | `#353541` | `#e3e3e9` |
| `--text` | `#f4f4f8` | `#16161c` |
| `--text-2` | `#c1c1cd` | `#545462` |
| `--text-3` | `#9696a3` | `#83838f` |
| `--accent` | `#8f73ff` | `#6d4aff` |
| `--accent-fg` | `#ffffff` | `#ffffff` |
| `--accent-soft` | `rgba(143,115,255,.17)` | `rgba(109,74,255,.10)` |
| `--ok` | `#46d8a4` | `#0ea66e` |
| `--warn` | `#fbbf24` | `#c2830a` |

### Hard rules (encoded in spec, enforced in CI)

1. Component CSS references **only** semantic tokens (and the type/spacing/radius scales) — never raw hex, never primitive ramps directly.
2. Every text token must meet **WCAG AA contrast** against its background in both modes. (The locked values above satisfy this; any future change must preserve it.)
3. A CI guard (extend `scripts/check-frontend-deps.sh` or a sibling script) greps component CSS in `crates/app/src` for raw 3/6-digit hex and `rgb(`/`rgba(` literals outside the `theme/` module and fails on new occurrences.

## Component Library

New module `crates/app/src/components/ui/`. Each is a Dioxus component with typed props and scoped, tokenized CSS. The library is the only sanctioned way pages render these primitives.

| Component | Responsibility | Notable props |
|-----------|----------------|---------------|
| `Button` | All buttons/CTAs | `variant: primary\|ghost\|danger`, `size: sm\|md`, `disabled`, `loading` |
| `Card` | Surface container | `padding`, optional `accent_edge: bool` |
| `Input`, `Textarea`, `Select` | Form fields | `value`, `oninput`, `placeholder`, `invalid: bool`, `disabled` |
| `Pill` / `Badge` | Status chips | `tone: neutral\|accent\|ok\|warn\|danger` |
| `Label` | Mono uppercase field/section label | — |
| `SectionHeader` | Section title block (refactor existing) | `label`, `title`, `description` |
| `Modal` | Dialog (refactor existing) | existing API, retokenized |
| `Toast` | Notifications (refactor existing) | existing API, retokenized |
| `EmptyState` | Empty-list placeholder | `icon`, `title`, `message`, optional action |
| `Spinner` | Loading indicator | `size` |
| `PageShell` | Standard page padding/width container | `children` |

Each component gets a render smoke test (mounts, renders expected root, reflects key props). This library is what removes the ~600 lines of duplicated card/form/button CSS.

## Theming Engine + Dark/Light Toggle

- `ThemeProvider` mounted at the app root: sets `data-theme` on the document element, reads/writes choice in `localStorage` (key `sc-theme`), defaults to `prefers-color-scheme` when unset. Exposes current mode + a setter via context.
- `ThemeToggle` component placed in the nav/shell; flips light↔dark.
- **Brand seam:** the accent/brand values come from a single `BrandConfig` struct (or const) consumed by the token-emitting `<style>`. In SP1 this is hardcoded to Scuffed defaults. The indirection is the only thing SP2 changes — swapping the source from constant to `/api/settings` is a one-site change, not a per-page one. SP1 builds the seam; SP1 does not read settings.

## Migration Plan (full app)

- **Phase A — Foundation (no page changes):** build `theme/` token module, `components/ui/` library, `ThemeProvider`/`ThemeToggle`, the CI raw-hex guard, and load the three Google Fonts. Old `theme.rs` retired.
- **Phase B — Shell & layouts:** migrate `layouts/public.rs`, `layouts/admin.rs`, `layouts/strategy.rs` and the nav to the new tokens/components + theme toggle. Highest-visibility; proves the system end-to-end.
- **Phase C — Pages, in batches:** migrate every remaining page to UI components + semantic tokens, verifying both modes per batch:
  - C1: content/public pages (home, members, member_profile, news, blog, community, apply, events, polls, wiki, forum, forum_thread, tournaments + detail, scrims, stats, dm, identity).
  - C2: admin pages (dashboard, members, teams, games, matches, tournaments, schedules, announcements, articles, moderation, audit log, settings, relay).
  - C3: strategy editor — adopt the system, replace the placeholder panels (TeamPanel/PropertiesPanel/Timeline stubs at `pages/strategy/editor.rs`), and convert its orange to a legitimate per-section accent override (`[data-theme="dark"] [data-accent="strategy"]` style) rather than hardcoded `#ff6a00`.

## Testing

- Render smoke test per `components/ui/` component.
- CI raw-hex guard passes (no raw color literals in component CSS outside `theme/`).
- `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings` and `cargo fmt --check` green.
- Per page batch: manual visual pass in **both** light and dark, confirming no unreadable text and no broken layouts.
- Contrast is enforced at the token layer (verified once for the locked values), not re-checked per page.

## Risks / Notes

- **Large surface area:** ~25 pages. Mitigated by doing all the design thinking in Phase A so Phase B/C are mechanical, repetitive swaps against a fixed component API.
- **Strategy editor** is the most complex page (canvas + panels); it is sequenced last (C3) and includes finishing its placeholder UI.
- **Tailwind:** remains loaded for its reset/base only; no utility-class adoption in this project (authoring stays tokenized component CSS).
