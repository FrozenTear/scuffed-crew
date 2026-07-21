# Stat Tracker X11 Capture Backend — Implementation Spec (final)

> **For agentic workers:** This spec is executed under
> `2026-07-21-stat-tracker-x11-orchestration.md` (same dir): Claude
> orchestrates, lanes A–D run as separate worktree branches with symmetric
> claude/grok authorship and dual-agree merges. **A lane brief = that lane's
> tasks from this document, verbatim.** Use superpowers:executing-plans (or
> subagent-driven-development inside a lane) with checkbox tracking. Every
> decision in this spec is final (adjudicated over five review passes — see
> the Decision register); do not relitigate settled choices inside a lane.
> Deviations require a plan amendment posted to `fleet::x11-capture` BEFORE
> the code diverges.

**Goal:** Add a native X11 screen-capture backend to `scuffed-stat-tracker` so
the daemon works on pure X11 sessions (no Wayland), without changing OCR,
session FSM, storage, or sync.

**Architecture:** Capture is already pluggable via `CaptureBackend` +
`capture_screen_output`. Resolve one probed backend at startup and pass that
same backend to capture and output listing so the CLI, GUI, and daemon cannot
disagree. Add `CaptureBackend::X11` using `x11rb` (RandR enumeration + core
`GetImage`; no MIT-SHM, no connection cache in the MVP). Selection order:
usable Wayshot → usable X11 → Portal (weak availability, unchanged) → None.
OCR and Tab/evdev paths stay untouched.

**Constraint:** Author daily-drives Wayland and cannot fully live-test X11.
All lane work is compile/unit-testable on Wayland; pure-X11 correctness is
the deferred Task 8 checklist (USER-only).

**Status:** Local draft — gitignored; promoted to tracked `main` as
orchestration Phase 0 Step 4 before any lane starts. Review history (R1–R5
narrative) is preserved in `2026-07-21-stat-tracker-x11-review-archive.md`.

---

## Decision register (final — source of truth for "why")

Every entry was adjudicated across the five-pass review cycle (R1 blockers,
R2 Claude code-verified, R3 Grok adjudicated, R4 Claude consistency, R5 Codex
concur). IDs are kept so the orchestration plan's cross-references resolve.

| ID | Decision (final) |
|----|------------------|
| R1-1 | One backend decision per process path: `detect_backend()` owns env priority + probes; every caller passes its result to `list_outputs` / `capture_screen_output`. No caller re-reads env. |
| R1-2 | Probes are real: Wayshot and X11 must successfully connect and enumerate ≥1 output before selection. Env vars alone indicate intent, not availability. Failed automatic candidate falls through. |
| R1-3 | Force means force: unavailable/unknown `STAT_TRACKER_CAPTURE` value → `CaptureBackend::None` + clear diagnostic. Never silent fallback. |
| R1-4 | Dependency = `x11rb` (RustConnection, `randr` feature). `xcap` rejected: pulls PipeWire/Wayland/zbus/xcb with no X11-only feature. |
| R1-5 | Dependency choice must be reflected in stat-tracker CI + release jobs; release ELF closure must prove which X11 libs (if any) are host-provided. |
| R1-6 | Pure selection-policy + output-matching unit tests; full fmt/build/test/clippy gates; README says **experimental** until Task 8. |
| R2-2 | Portal `--list-outputs`: print explicit "portal backend does not support output selection", never an empty "Available outputs:" list. |
| R2-3 | Wayshot's connection cache is `thread_local!` on the blocking pool; `probe()` warming one thread's connection is fine and must not be "optimized" into cross-thread sharing. |
| R2-4 | Nested/virtual X servers reporting zero RandR outputs = probe policy correctly rejecting an unusable server, not a bug. Never weaken the probe to pass a smoke. |
| R3-1 | GUI backend is **root-owned**: `gui/main.rs` routes are siblings with no `#[layout]`; the only persistent ancestor is `app()` (line 133). Context/signal provided there, above `Router::<Route> {}`; panels only consume. |
| R3-2 | MVP has **no X11 connection cache**: connect-per-call. Thread-local cache only as a measured follow-up. |
| R3-3 | Task 0 evaluates x11rb's `image` feature (`PixelLayout` helpers, no native dep); enable if it reduces hand-rolled conversion risk. |
| R3-4 | Portal keeps its pre-existing weak `XDG_CURRENT_DESKTOP` check — hardening it is a non-goal; last resort only. |
| R3-5 | Empty RandR name → `x11-output-{xid}` fallback is **runtime-stable only** (XIDs change across server restarts). Never described as persistent. |
| R3-6 | Three promotion tiers (table in Task 8). "Experimental" ships on Tasks 0–7 alone; A–D adds a validated note; E–F unlocks "supported". |
| R4-1 | Signature convention: `capture_screen_output` **keeps `&CaptureBackend`** (existing signature, zero call-site churn); new `list_outputs` takes `CaptureBackend` **by value**. Mixed shapes are deliberate — do not harmonize. |
| R5-1 | `wayshot::probe()` is a new function with an explicit implementation step (Task 3 Step 2) — it did not previously exist. |

Verified call-site inventory (2026-07-21): `capture/mod.rs:22-23`
(`&CaptureBackend`); `capture_screen_output` callers `main.rs:1152`,
`main.rs:1549`, `gui/preview.rs:34`; hardcoded `wayshot::list_outputs()` at
`main.rs:232`, `main.rs:337`, `gui/status.rs:35`, `gui/settings.rs:8`
(synchronous inside `use_signal` — must-fix); independent `detect_backend`
calls at `gui/status.rs:42`, `gui/preview.rs:33`; help text `main.rs:202`.

---

## Lane map (who runs which tasks)

| Lane | Owner | Tasks | Branch (PR) | Depends on |
|------|-------|-------|-------------|------------|
| Track 1 | Claude subagents | Task 0 (spike) | no branch; decision post on `fleet::x11-capture` | — |
| **A** | claude | Task 1 | `x11-prep` (PR-1) | — (starts at t≈0) |
| **B** | grok | Tasks 2 + 4 | `x11-module` (PR-2) | Task 0 decision post |
| **C** | claude | Tasks 3 + 5 | `x11-wire` (PR-3) | PR-1 + PR-2 merged |
| **D** | grok | Task 6 | `x11-docs` (PR-4) | Task 0 evidence; `main.rs` help string only after PR-1 |
| Phase 4 | orchestrator + gate subagent | Task 7 | on merged `main` | PR-1..4 |
| USER | human only | Task 8 | — | Phase 4 green |

Merge order is strict: **PR-1 → PR-2 → PR-3 → PR-4.** The single sanctioned
file overlap between lanes is the `pub mod x11;` line in `capture/mod.rs`
(Lane B adds it; a one-line rebase for whoever lands second). Everything else
is disjoint by the orchestration plan's ownership table.

The tree builds at every merge point: PR-1 changes only existing backends;
PR-2's `x11.rs` is a complete, unit-tested, **unwired** module (no enum arm);
PR-3 wires it; PR-4 documents merged behavior.

---

## Context (current state)

### Works on any Linux display server already

| Layer | Location | Notes |
|-------|----------|--------|
| Tab hook | `src/detect/mod.rs` (`MultiKeyboardStream`, evdev) | Kernel `/dev/input` — not Wayland-specific |
| Game gate | `src/detect/game_running.rs` | `/proc/<pid>/comm` |
| OCR / parse / sessions / sync | `ocr/`, `parse.rs`, `storage/`, `sync/`, `main.rs` | Image in → stats out |
| GUI toolkit | Dioxus desktop + GTK + tray | Usually works under X11 |

### Wayland-only today

| Piece | File | Behavior |
|-------|------|----------|
| Preferred capture | `src/capture/wayshot.rs` | `libwayshot`, gated on `WAYLAND_DISPLAY` env only |
| Fallback | `src/capture/portal.rs` | `ashpd` screenshot; availability = `XDG_CURRENT_DESKTOP` env only |
| Backend enum | `src/capture/mod.rs` | `Wayshot \| Portal \| None` |
| Output listing | wayshot-hardcoded call sites (see Decision register inventory) | Pure X11 → empty list / misleading UI |

Portal *may* work on some X11 DEs but is slow and not gaming-safe for the 4s
poller (`performance-review-2026-07-13.md` M5), and ignores `capture_output`.

### Selection: today → target

```text
TODAY:  WAYLAND_DISPLAY set? → Wayshot | else XDG_CURRENT_DESKTOP set? → Portal | else None

TARGET: wayshot::probe() ok?  → Wayshot   (WAYLAND_DISPLAY + real connect + ≥1 output)
        else x11::probe() ok? → X11       (DISPLAY + real connect + ≥1 active RandR output)
        else portal weak-available? → Portal   (unchanged weak check — R3-4)
        else → None
Force (fail-closed, R1-3): STAT_TRACKER_CAPTURE=wayshot|x11|portal
```

### Final API

```rust
pub async fn detect_backend() -> CaptureBackend;

pub async fn list_outputs(              // NEW — by value (R4-1)
    backend: CaptureBackend,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>>;

pub async fn capture_screen_output(     // EXISTING signature kept (R4-1)
    backend: &CaptureBackend,
    output_name: Option<&str>,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>>;
```

All connection work stays off async/UI threads via `spawn_blocking`. Detect
once per process path (R1-1): daemon = once at startup onto `DaemonCtx`; CLI
one-shot = once in the handler; GUI = once in root `app()` (R3-1). Hotplug
re-enumerates outputs for the already-chosen backend only — never re-runs
selection.

---

## Non-goals (binding)

- Windows / macOS capture
- Replacing libwayshot on Wayland
- Portal multi-output selection or availability hardening (R3-4)
- Full OCR re-validation on X11 (fixtures cover vision offline)
- Changing Tab detection, session FSM, or sync contracts
- XWayland special cases (Wayshot already captures XWayland windows on Wayland)
- MIT-SHM / X11 connection caching (R3-2 — measurement-gated follow-ups only)

---

## Task 0: Dependency and packaging spike — Track 1 (Claude subagents)

Read-only investigation; no branch. Result = a decision post on
`fleet::x11-capture` (grok ACKs before Lane B starts) + pasted into PR-2's body.

- [ ] **Step 1: Verify x11rb APIs** — current release (0.14.0 confirmed on
  crates.io 2026-07-21, docs at docs.rs/crate/x11rb/0.14.0), `RustConnection`,
  RandR monitor/output enumeration, core `GetImage`, server image byte
  order/visual masks, `shm` feature surface, and the `image` feature
  (`PixelLayout`, byte order, bpp, scanline pad — R3-3). Recommend whether
  `image` joins the feature set (it does not replace the explicit mask→RGBA
  policy in Task 4).
- [ ] **Step 2: Confirm the xcap rejection** — inspect the current pinned
  release's Linux dependency closure (PipeWire/Wayland/zbus/xcb, no X11-only
  feature expected — R1-4). Overturning this requires evidence it compiles
  X11-only AND a list of every CI package/workflow change it forces.
- [ ] **Step 3: Build + runtime closure check** (throwaway worktree):

```bash
cargo tree -p scuffed-stat-tracker
cargo check -p scuffed-stat-tracker
cargo build -p scuffed-stat-tracker --bin scuffed-stat-tracker
ldd target/debug/scuffed-stat-tracker
```

Expected for `x11rb` + `RustConnection`: **no new** libX11/libxcb/PipeWire/
DBus dynamic dependency. If false, the decision post must flag Task 6 + release
workflow updates before Lane B implements.

- [ ] **Step 4: Post the decision** — crate, exact pin, features, observed
  native deps, reasoning. Default:

```toml
x11rb = { version = "0.14", features = ["randr"] }          # baseline
# or, if Step 1 shows PixelLayout helpers reduce conversion risk:
x11rb = { version = "0.14", features = ["randr", "image"] }
```

---

## Task 1: Unified output-listing API + root-owned GUI backend — Lane A (PR-1)

**Files:** `capture/mod.rs` (new `list_outputs` fn only), `capture/wayshot.rs`
(Send-safe list wrapper), `main.rs` (info flags, `log_selected_output`),
`gui/main.rs`, `gui/status.rs`, `gui/settings.rs`, `gui/preview.rs`.

**Why first:** every caller hardcodes `wayshot::list_outputs()` today; X11
needs one entry point that uses the exact backend `detect_backend` chose and
preserves errors. This lane touches no X11 code and starts immediately.

- [ ] **Step 1: Add `capture::list_outputs`**

```rust
// capture/mod.rs
/// List capture targets for the backend already selected by detect_backend.
/// Connection work must stay off the async/UI thread.
pub async fn list_outputs(
    backend: CaptureBackend,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Wayshot => {
            let result = tokio::task::spawn_blocking(|| {
                wayshot::list_outputs().map_err(|e| e.to_string())
            })
            .await?;
            result.map_err(Into::into)
        }
        // Lane C (Task 3) adds the X11 arm when the variant exists.
        CaptureBackend::Portal | CaptureBackend::None => Ok(Vec::new()),
    }
}
```

The enum has no `X11` variant yet — Lane C adds it, and the exhaustive match
here will force the new arm at that point. Do NOT use `unwrap_or_default()`
in the public API: an unreachable display must not look like a valid backend
with zero outputs.

- [ ] **Step 2: Route CLI through the unified API**

Make `handle_info_flags` async. `--list-outputs`: `detect_backend().await` →
`list_outputs(backend).await`; print an error and exit non-zero when detection
returns `None` or the backend cannot enumerate. If the resolved backend is
Portal, print "portal backend does not support output selection" instead of an
empty list (R2-2). Change `log_selected_output` to accept the already-resolved
`backend`, call the same API, and rename the log field `"wayland outputs"` →
`"capture outputs"` (keep available/selected fields). The startup backend log
in `main.rs` already exists — do not duplicate it.

- [ ] **Step 3: Root-owned GUI backend (R3-1)**

`gui/main.rs` mounts `Router::<Route> {}` with sibling route components and no
`#[layout]` — the only persistent shared ancestor is `app()`. Therefore:

1. In `app()` (or a thin wrapper it mounts above `Router`), resolve
   `detect_backend()` **once** via `use_resource`/`spawn` into a context /
   root `Signal<Option<CaptureBackend>>`.
2. `status`, `settings`, and `preview` panels **read that context only** and
   render a "detecting…" state while it is `None`-pending. They never call
   `detect_backend()` themselves (removes `status.rs:42`, `preview.rs:33`).
3. Output lists: `list_outputs(backend)` inside a resource/future keyed on the
   shared backend. This removes `settings.rs:8`'s synchronous
   `wayshot::list_outputs().unwrap_or_default()` inside `use_signal` — a
   blocking display connect at component construction, the worst pre-existing
   offender. Never enumerate synchronously during construction.
4. On failure: empty dropdown + visible diagnostic state; no panics.

Optional later (not this PR): a "Refresh capture backend" button that re-runs
detect once — never on a timer, never on route change.

- [ ] **Step 4: Gates** (full lane matrix, see Task 7 Step 1 — minimum here):

```bash
cargo check -p scuffed-stat-tracker
cargo check -p scuffed-stat-tracker --features gui
```

Wayland listing behavior unchanged; failures stay visible.

- [ ] **Step 5: Commit**

```bash
git add crates/stat-tracker/src/capture/mod.rs crates/stat-tracker/src/main.rs \
  crates/stat-tracker/src/capture/wayshot.rs \
  crates/stat-tracker/src/gui/main.rs crates/stat-tracker/src/gui/status.rs \
  crates/stat-tracker/src/gui/settings.rs crates/stat-tracker/src/gui/preview.rs
git commit -m "refactor(stat-tracker): unify capture output listing API"
```

---

## Task 2: X11 module skeleton + dependency — Lane B (PR-2, first half)

**Files:** create `capture/x11.rs`; `capture/mod.rs` (`pub mod x11;` — the one
sanctioned cross-lane line); `Cargo.toml` (Task 0's exact pin).

- [ ] **Step 1: Add the dependency** exactly as the Task 0 decision post
  specifies (pin + features). No `shm`.

- [ ] **Step 2: Skeleton**

```rust
//! Native X11 screen capture (pure X11 sessions).
//!
//! Selected only after a real X connection/output probe succeeds. Connection
//! work runs inside spawn_blocking to match wayshot. MVP: connect-per-call,
//! no connection cache (decision R3-2).

use image::DynamicImage;

pub async fn probe() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tokio::task::spawn_blocking(probe_blocking).await?
}

pub async fn list_outputs(
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    tokio::task::spawn_blocking(list_outputs_blocking).await?
}

pub async fn capture_with_output(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let target = output_name.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || capture_blocking(target.as_deref()))
        .await?
}
```

`probe_blocking` must connect and find ≥1 active RandR output (R1-2). A
non-empty `DISPLAY` alone is not availability. `DISPLAY` unset / connection
refused / no outputs are all clean errors, never panics — this module compiles
and unit-tests on hosts with no X server.

- [ ] **Step 3: Wire `pub mod x11;`** in `capture/mod.rs`. No enum arm — that
  is Lane C. The module is complete-but-unwired at PR-2; that is the intended
  merge state.

- [ ] **Step 4: `cargo check -p scuffed-stat-tracker`**, commit
  `feat(stat-tracker): add native X11 capture module skeleton`.

---

## Task 3: Enum + probed selection + dispatch — Lane C (PR-3, first half)

**Files:** `capture/mod.rs`, `capture/wayshot.rs` (new `probe()`),
`gui/status.rs` (label arm). Branch from `main` after PR-1 AND PR-2 merged.

- [ ] **Step 1: Extend enum**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    Wayshot,
    X11,
    Portal,
    None,
}
```

- [ ] **Step 2: Implement `wayshot::probe()` (R5-1 — new function)**

`pub async fn probe()` in `wayshot.rs`: cheap `WAYLAND_DISPLAY` check first,
then `spawn_blocking` + the existing `with_wayshot` wrapper requiring ≥1
output, preserving the underlying error (Send-safe; same shape as
`x11::probe`). Note (R2-3): the thread-local connection this warms belongs to
one blocking-pool thread; later captures on other threads reconnect lazily —
correct, leave it alone.

- [ ] **Step 3: Probed `detect_backend`**

```rust
pub async fn detect_backend() -> CaptureBackend {
    // Task 5 adds the STAT_TRACKER_CAPTURE force block above this.
    if wayshot::probe().await.is_ok() {
        return CaptureBackend::Wayshot;
    }
    if x11::probe().await.is_ok() {
        return CaptureBackend::X11;
    }
    if portal::is_available().await {
        return CaptureBackend::Portal;
    }
    tracing::warn!("no capture backend available");
    CaptureBackend::None
}
```

Behavior notes: Wayland session with XWayland → usable Wayshot still wins.
Pure X11 → only `DISPLAY` → X11. Dual-env nested setups → Wayshot unless
forced. Automatic probe failures log at debug/warn and fall through (R1-2).

- [ ] **Step 4: Dispatch** — keep the existing signature (R4-1, final):

```rust
pub async fn capture_screen_output(
    backend: &CaptureBackend,
    output_name: Option<&str>,
) -> Result<image::DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    match backend {
        CaptureBackend::Wayshot => wayshot::capture_with_output(output_name).await,
        CaptureBackend::X11 => x11::capture_with_output(output_name).await,
        CaptureBackend::Portal => portal::capture().await,
        CaptureBackend::None => Err("no capture backend available".into()),
    }
}
```

- [ ] **Step 5: X11 arm in `list_outputs`** — `CaptureBackend::X11 =>
  x11::list_outputs().await,`. No environment logic here; `detect_backend`
  already decided (R1-1).

- [ ] **Step 6: GUI label arm** in `gui/status.rs`:
  `CaptureBackend::X11 => "X11 (native)".to_string(),` (existing labels:
  wayshot = "libwayshot (Wayland)", portal = "XDG Desktop Portal", none =
  "none available").

- [ ] **Step 7: Sweep for non-exhaustive matches**

```bash
rg 'CaptureBackend::' crates/stat-tracker
cargo check -p scuffed-stat-tracker --features gui
```

- [ ] **Step 8: Commit**
  `feat(stat-tracker): select X11 capture backend when not on Wayland`.

---

## Task 4: `x11.rs` capture implementation — Lane B (PR-2, second half)

**Files:** `capture/x11.rs` only.

- [ ] **Step 1: Normalized output descriptor**

```rust
struct X11Output {
    name: String,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    primary: bool,
}
```

Enumerate connected outputs with an active CRTC. The RandR output **name** is
the persisted `capture_output` when non-empty; empty name → `x11-output-{xid}`
label, which is **runtime-stable only** (R3-5 — XIDs change across server
restarts; never call it persistent in docs or user-facing strings). Never use
vector index as a name. Stable order: primary first, then name/geometry.

- [ ] **Step 2: Selection as a pure helper** — given descriptors + optional
  configured name: (1) exact match, else error listing available names;
  (2) unconfigured → primary active; (3) no primary → first active in stable
  order. **Unit-test without an X server:** missing configured output,
  no-primary fallback, duplicate/empty-name fallback, stable ordering.

- [ ] **Step 3: Core `GetImage` capture**

Connect `x11rb::connect(None)`, select descriptor, `GetImage` the descriptor
rectangle from the root window, convert to `image::RgbaImage`, end at
`DynamicImage::ImageRgba8` so OCR is unchanged.

**Pixel conversion policy (TrueColor-only MVP):** Support only TrueColor
visuals at root depths **24 or 32**, applying the server's image byte order
and per-channel RGB masks from the setup/visual (LSBFirst vs MSBFirst;
red/green/blue masks; scanline pad). Expand packed pixels into independent
RGBA8 with alpha fixed `0xFF`. Do **not** assume "BGRA on little-endian"
without checking masks. Reject everything else — PseudoColor, DirectColor,
depth 16/15/8, unexpected bpp, unmappable mask combinations — with an error
that includes `depth`, `bits_per_pixel`, `byte_order`, and the three masks.
Never silently swap channels or stretch unknown formats. If a real DE smoke
later needs another format, that is an explicit follow-up conversion path,
not MVP guesswork. (If Task 0 enabled x11rb's `image` feature, its
`PixelLayout` helpers may implement this policy — they do not replace it.)

- [ ] **Step 4: Connection lifecycle — MVP has NO cache (R3-2).** Each
  probe/list/capture blocking call connects, works, drops. If Phase 4
  measurement shows connect+enumerate is a material fraction of the 4s poll
  budget, a follow-up adds a thread-local cache + stale-connection recovery
  (same shape as wayshot's — per-thread, never cross-thread). Measurement
  first; cache second.

- [ ] **Step 5: Error quality** — match wayshot style: missing output lists
  available names; connection/auth failures and empty output lists are clear
  errors; `.first()` over `[0]`; no panics anywhere.

- [ ] **Step 6: Gates + commit**
  `feat(stat-tracker): implement X11 monitor list and screenshot capture`.
  Lane B posts PR-2 only after Tasks 2+4 are both green (module is complete,
  unit-tested, unwired).

---

## Task 5: Fail-closed force env + policy tests — Lane C (PR-3, second half)

**Files:** `capture/mod.rs`, `main.rs` (verify only).

- [ ] **Step 1: `STAT_TRACKER_CAPTURE` override** — prepended inside
  `detect_backend` (R1-3, fail-closed):

```rust
if let Ok(force) = std::env::var("STAT_TRACKER_CAPTURE") {
    match force.to_ascii_lowercase().as_str() {
        "wayshot" | "wayland" => {
            if wayshot::probe().await.is_ok() { return CaptureBackend::Wayshot; }
            tracing::warn!("STAT_TRACKER_CAPTURE=wayshot is not usable");
            return CaptureBackend::None;
        }
        "x11" => {
            if x11::probe().await.is_ok() { return CaptureBackend::X11; }
            tracing::warn!("STAT_TRACKER_CAPTURE=x11 is not usable");
            return CaptureBackend::None;
        }
        "portal" => {
            if portal::is_available().await { return CaptureBackend::Portal; }
            tracing::warn!("STAT_TRACKER_CAPTURE=portal not available");
            return CaptureBackend::None;
        }
        other => {
            tracing::warn!(%other, "unknown STAT_TRACKER_CAPTURE value");
            return CaptureBackend::None;
        }
    }
}
// … automatic probe order (Task 3) …
```

A forced backend never silently falls back — that is the point (dual-session
X11 testing must mean what it says).

- [ ] **Step 2: Pure selection-policy tests (R1-6).** Env reads and live
  probes stay in the async wrapper; a pure helper takes their normalized
  results. Cover at minimum: automatic Wayshot→X11→Portal→None priority;
  failed Wayshot → X11; failed X11 → Portal; each valid forced backend;
  forced-unavailable → None with no fallback; unknown force value → None.
  **No `std::env::set_var` in tests** — process-global env mutation races
  parallel test threads; test with explicit string/probe inputs.

- [ ] **Step 3: Verify startup logging** — `main.rs` already logs
  `tracing::info!(?backend, "capture backend selected")` exactly once; keep it
  so, and confirm `--list-outputs` prints/logs its resolved backend.

- [ ] **Step 4: Commit**
  `feat(stat-tracker): STAT_TRACKER_CAPTURE override and backend log`.
  Lane C posts PR-3 after Tasks 3+5 are both green.

---

## Task 6: Docs + packaging — Lane D (PR-4)

**Files:** `README.md`, `dist/install.sh` + `dist/bundle-native-libs.sh`
(check), `.github/workflows/stat-tracker*.yml` (check), `main.rs` help string
(**only after PR-1 is merged** — same-file sequencing with Lane A).

- [ ] **Step 1: README platform section** (tier 1 wording — R3-6):

```markdown
- **Linux + Wayland; experimental X11 capture.**
  - Wayland: libwayshot on wlr-screencopy compositors (Sway, Hyprland, …),
    with XDG Desktop Portal fallback.
  - X11 (experimental): native capture when a usable X server is detected and
    Wayland capture is unavailable.
  - Portal remains last-resort on either stack (slower; not ideal for the poller).
```

Host-needs row: "Wayland **or** X11 + `input` group + tessdata". The word
"experimental" is removed only by the Task 8 promotion commit — never in this
PR.

- [ ] **Step 2: Config table** — `capture_output`: "Display/output name to
  capture (`--list-outputs`)"; drop "Wayland-only" wording.

- [ ] **Step 3: Help string** (`main.rs:202`):
  `--list-outputs        list capture outputs and exit`.

- [ ] **Step 4: Packaging verification against Task 0's closure evidence.**
  For `x11rb::RustConnection`: confirm `ldd` on staged daemon + GUI shows no
  new X11 client library; do NOT add `^libX|^libxcb` bundler exclusions to
  hide an unexpected dependency — explain any surprise first (R1-5). Update
  `dist/install.sh` missing-library wording "wayland/evdev" → "display
  stack/evdev". CI/release workflows: only touch if Task 0 evidence demands
  build packages (expected: no changes for x11rb).

- [ ] **Step 5: Commit** `docs(stat-tracker): document X11 capture support`.

---

## Task 7: Verification train — Phase 4 (merged `main`)

- [ ] **Step 1: Full gates** (gate-runner subagent, fresh worktree off
  `origin/main`). CI sets `working-directory: crates/stat-tracker`
  (`stat-tracker.yml:23`), hence the parenthesized `cd` for fmt parity:

```bash
(cd crates/stat-tracker && cargo fmt --check)
cargo build -p scuffed-stat-tracker
cargo build -p scuffed-stat-tracker --features gui
cargo test -p scuffed-stat-tracker
cargo clippy -p scuffed-stat-tracker -- -D warnings
cargo clippy -p scuffed-stat-tracker --features gui --all-targets -- -D warnings
bash scripts/check-design-tokens.sh
# + remaining repo gate per docs/fleet-protocol.md Appendix A
```

- [ ] **Step 2: Wayland regression** (orchestrator, real session):
  `cargo run -p scuffed-stat-tracker -- --list-outputs` → backend = Wayshot,
  outputs unchanged; daemon still logs wayshot and captures on Tab.

- [ ] **Step 3: Forced X11 fail-closed check:**
  `STAT_TRACKER_CAPTURE=x11 cargo run -p scuffed-stat-tracker -- --list-outputs`
  → override honored. `DISPLAY` absent/probe fails → clear non-zero failure,
  NOT wayshot's list. `DISPLAY` = XWayland → success is possible; document it,
  but it is not the pure-X11 smoke.

- [ ] **Step 4: Xephyr nested smoke (recommended once):**

```bash
Xephyr :2 -screen 1280x720 &      # pkg: xorg-server-xephyr
DISPLAY=:2 STAT_TRACKER_CAPTURE=x11 \
  cargo run -p scuffed-stat-tracker -- --list-outputs
```

One PNG dump via the GUI preview (no unplanned source edits); confirm non-zero
dimensions, non-black content. Zero RandR outputs on a nested server = probe
policy working, not a bug (R2-4) — do not weaken the probe. Record median and
worst-of-20 capture latency and post to `fleet::x11-capture` — this is the
go/no-go data for any future SHM/cache follow-up (R3-2).

---

## Task 8: Live X11 smoke — USER only (blocks promotion)

Run on a pure X11 session (second TTY "Plasma (X11)" / "GNOME on Xorg", or a
volunteer's box). Agents never simulate, Xephyr-substitute, or claim these.

| # | Check | Pass criteria |
|---|--------|----------------|
| A | `scuffed-stat-tracker --list-outputs` | ≥1 output name printed |
| B | Daemon start log | `capture backend selected X11` (or equivalent) |
| C | GUI status | Label "X11 (native)", not Portal/None |
| D | One capture dump | Non-black PNG, correct resolution for selected monitor |
| E | Tab + scoreboard (OW or synthetic fullscreen matching geometry) | ≥1 accepted OCR path or deliberate "not scoreboard" reject — proves real pixels |
| F | Poll tick (auto_detect on) | No portal dialog spam; CPU acceptable; no crash over ~2 min |

Record: dependency/version, DE, GPU, single/multi monitor, latency, X11 errors.

**Promotion tiers (R3-6, canonical):**

| Checks passed | README claim |
|---------------|--------------|
| Tasks 0–7 only | **experimental** (not yet live-validated) — this is the merge state |
| A–D on pure X11 | still **experimental**, + "validated on $DE/$GPU" note |
| E–F | **supported** — remove "experimental" in a separate, reviewed commit |

The A–D tier never blocks the Tasks 0–7 merge train. The promotion commit is
drafted during Phase 5 handoff and lands only after USER posts results.

---

## Risk follow-ups (post-MVP, only if smoke fails)

| Symptom | Likely cause | Follow-up |
|---------|--------------|-----------|
| Black frames / zero size | Pixel-format/visual conversion or wrong root geometry | Fix conversion first; SHM does not fix wrong pixels |
| Black/empty only while game is fullscreen | Unredirected fullscreen / compositor / driver (NVIDIA) hiding pixels from root `GetImage` | Document limit; try windowed/borderless; compositor-specific capture research; portal last resort |
| Wrong monitor | Name mismatch / Xinerama vs RandR | Stabilize naming (`name` + runtime id + geometry in list) |
| High CPU / lag every 4s | Connect-per-call or full framebuffer read | Measure first; then TLS cache and/or SHM follow-up; or raise poll interval on X11 |
| Capture works, OCR fails | Geometry/resolution scaling | Not X11-specific; existing fixture tools |
| Probe fails headless CI | No DISPLAY/X server | Expected; unit tests never touch live X |

---

## Effort (per lane, wall-clock under the orchestration plan)

| Lane | Content | Est. | Parallel with |
|------|---------|------|---------------|
| Track 1 | Task 0 spike | 1–2 h | Lane A |
| A | Task 1 | 3–5 h | Track 1, B |
| B | Tasks 2+4 | 4–6 h | A (after decision post) |
| C | Tasks 3+5 | 2–4 h | D |
| D | Task 6 | 1–2 h | C |
| Phase 4 | Task 7 | 1–2 h | — |
| **Wall-clock total** | | **~1 working day** | vs 2–3.5 days serial |
