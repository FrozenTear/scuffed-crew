# Spike plan: Dioxus 0.8 alpha + Blitz for desktop GUI

**For agents / future session:** User will point you at this note. Follow it.

**When:** Tonight / tomorrow (operator schedule).  
**Goal:** Try **Dioxus 0.8.0-alpha** so the **desktop GUI** uses the **Blitz / dioxus-native** path (lighter native HTML/CSS+GPU rendering, not WebView).  
**Out of scope:** Production website, VPS deploy, `Containerfile` / `scuffed-app` WASM — those stay on **0.7.9**.

---

## Context (why)

- Main product site = WASM + Axum → **not** Blitz.
- **stat-tracker GUI** (`crates/stat-tracker`, feature `gui`) is the desktop UI candidate for Blitz.
- 0.7 already had experimental native; **0.8 alpha** is where Blitz/native quality is being pushed hard. Operator wants to **experiment on 0.8 alpha**, not wait for stable docs.

Prior research: `docs/notes/dioxus-0.8-alpha-notes.md` (Props audit, release themes, low non_exhaustive risk in this repo).

---

## Required approach

1. **Create a git branch** (do not land on `main` until operator says so), e.g.:
   ```bash
   git checkout -b spike/dioxus-0.8-native-gui
   ```
2. Pin workspace / GUI path to **`dioxus = "0.8.0-alpha.0"`** (or latest 0.8 alpha if a newer one exists by then — check crates.io / GitHub releases).
3. Install matching CLI for local work only:
   ```bash
   cargo install dioxus-cli --locked --version 0.8.0-alpha.0
   ```
   (or prebuilt `dx` from the release assets)
4. Target **native / Blitz**, not WebView:
   - Prefer `dioxus` feature **`native`** / `dioxus-native` per 0.8 crate docs
   - Adjust `crates/stat-tracker` `gui` feature deps so the desktop GUI launches via **native** backend
5. Get GUI building and opening a window; note breakage, missing CSS/HTML features, GPU requirements.
6. Optional compare: memory / binary size vs previous WebView desktop build (rough is fine).
7. **Do not** change production:
   - `Containerfile` dioxus-cli pin
   - Site deploy docs assuming 0.7.9
   - Merge to `main` without explicit operator approval

---

## Acceptance for the spike (minimum)

- [ ] Branch exists and is used for all 0.8 work  
- [ ] stat-tracker GUI compiles against 0.8 alpha with native/Blitz path  
- [ ] GUI runs enough to open and show core screens (or document exact blockers)  
- [ ] Short note of findings (what broke, what’s lighter, whether to continue on alpha)  
- [ ] `main` / VPS path still 0.7.9  

---

## Starting points in this repo

| Path | Role |
|------|------|
| `crates/stat-tracker/` | Desktop GUI + daemon |
| `crates/stat-tracker/Cargo.toml` | `gui` feature, dioxus desktop deps today |
| `crates/stat-tracker/src/gui/` | Dioxus UI |
| `docs/notes/dioxus-0.8-alpha-notes.md` | 0.8 research + Props audit |

---

## Operator one-liner (paste to agent)

> Follow `docs/notes/dioxus-0.8-desktop-blitz-spike.md`: branch, Dioxus **0.8 alpha**, Blitz/native for **stat-tracker GUI** only; leave site/VPS on 0.7.9.
