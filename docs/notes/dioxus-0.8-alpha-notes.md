# Dioxus 0.8.0-alpha — notes for Scuffed Crew

**Status:** research only. Production stays on **Dioxus 0.7.9** until stable 0.8 docs exist.  
**Sources reviewed:** GitHub release `v0.8.0-alpha.0` (2026-05-19), shallow clone of that tag, package READMEs in-tree, grep of this monorepo (2026-07-10).

## Documentation gap

| Resource | Status |
|----------|--------|
| dioxuslabs.com `/learn/0.8/` | Missing (404) |
| Migration guide 0.7 → 0.8 | Missing |
| Migration guides on site | Through **0.6 → 0.7** only |
| Even 0.8 alpha README/CLI | Still links to **`/learn/0.7/`** |
| crates.io “latest” stable | **0.7.9** |
| Pre-release | **0.8.0-alpha.0** |

**Implication:** review source + release PRs; do not treat alpha as documented product upgrade.

Local clone used for review (disposable):

```text
/tmp/dioxus-research/dioxus   # tag v0.8.0-alpha.0
```

## What 0.8.0-alpha.0 is

- Workspace version **0.8.0-alpha.0**, Rust **edition 2024**.
- First cut of the 0.8 series; more alphas expected.
- Themes from release notes:
  - Breaking / behavior changes for **CLI**, **Signals**, some **internal** APIs
  - Large **dioxus-native / Blitz** quality upgrade
  - Install CLI from source with **`--locked`** for now
- No in-repo `notes/releases/0.8.*` product migration writeup comparable to a learn-site guide.

## Changes that may affect us

### Web app (`crates/app`) — primary product

| Change | Upstream signal | Likely impact |
|--------|-----------------|---------------|
| `#[component]` generated props are **`#[non_exhaustive]`** | `packages/core-macro` | Breaks only if code **constructs `XxxProps { .. }` by hand** or exhaustively matches; normal `rsx! { Foo { a: 1 } }` is fine |
| Template diffing: const hash vs pointer | core | Internal; no app API change expected |
| Signal / store coercion fixes | stores / signals | Correctness of edge reactivity; low code churn |
| External URL navigation opt-in | router | Optional; only if we customize external nav |
| `dx serve`: **hotpatch on by default** | CLI | Dev only; press `p` to change modes if flaky |

### Stat-tracker GUI (`crates/stat-tracker`, optional `gui`)

- Uses Dioxus **desktop** + a few manual `#[derive(Props)]` structs.
- Invocations are **rsx-style** (`HeroRowComponent { ... }`, `WinTrendChart { ... }`), not hand `HeroRowProps { ... }` literals in call sites.
- **No `#[derive(Store)]`** in this monorepo — store visibility enforcement is N/A for now.

### Mostly irrelevant to production web deploy

- objc2 macOS/iOS platform work  
- Blitz / dioxus-native rendering upgrades  
- macOS notarization, Alpine aarch64 bundle, Windows linker quirks  
- wry desktop WebView bumps  

## Repo audit (non_exhaustive risk) — 2026-07-10

Grep summary:

| Pattern | Result |
|---------|--------|
| `#[component]` | ~135 uses under `crates/app` + `stat-tracker` (normal) |
| `#[derive(Store)]` | **none** |
| Hand `FooProps { field: ... }` at call sites | **none found** |
| Manual `#[derive(..., Props)]` structs | **2** in `stat-tracker` GUI: `HeroRowProps`, `WinTrendChartProps` — used as `fn(props: HeroRowProps)` and called via rsx field syntax |

**Conclusion for a future 0.8 try:** this codebase looks **low-risk** for the non_exhaustive props change. Highest-touch areas are still “compile and fix,” not a rewrite. Re-check after large refactors that construct props structs in Rust (not rsx).

## How to experiment with alpha *locally* later

Do **not** change production `Containerfile` until stable 0.8.

```bash
# on a branch only
# Cargo.toml: dioxus = "0.8.0-alpha.0" (and lock update)
cargo install dioxus-cli --locked --version 0.8.0-alpha.0
# or install prebuilt dx from the GitHub release assets

cd crates/app && dx serve   # note hotpatch-default warning; press p if needed
cargo check -p scuffed-app --target wasm32-unknown-unknown
```

Checklist when trying:

1. [ ] Matching **crate + CLI** alpha versions  
2. [ ] App WASM check / `dx build`  
3. [ ] Optional: `stat-tracker` with `--features gui`  
4. [ ] Grep again for `Props {` hand construction  
5. [ ] Leave server/container on **0.7.9** until docs + stable  

## Production recommendation (current)

| Track | Version |
|-------|---------|
| VPS / Podman / `Containerfile` | **dioxus 0.7.9** + **dioxus-cli@0.7.9** |
| Local alpha experiments | separate branch, disposable |
| Upgrade to 0.8 stable | after **learn/0.8** + migration guide (or clear release notes) |

## Upstream links

- [v0.8.0-alpha.0 release](https://github.com/DioxusLabs/dioxus/releases/tag/v0.8.0-alpha.0)  
- [Compare v0.7.6…v0.8.0-alpha.0](https://github.com/DioxusLabs/dioxus/compare/v0.7.6...v0.8.0-alpha.0)  
- Stable docs (still): [learn/0.7](https://dioxuslabs.com/learn/0.7/)  
- Migration (still only through 0.7): [learn/0.7/migration](https://dioxuslabs.com/learn/0.7/migration/)  
