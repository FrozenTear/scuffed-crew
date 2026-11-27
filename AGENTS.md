# AGENTS.md

This file is the map for coding agents (Cursor cloud agents, Claude Code, any harness).

- Binding fleet protocol: `docs/agent-protocol.md` (IRON LAW, Evidence Law, review and
  merge gates). Host Memtrace ops: `docs/notes/memtrace-ops.md`. Open fleet work:
  `docs/notes/night-shift-backlog.md`. This file does not repeat the protocol.
- Architecture, SurrealDB gotchas, membership policy, brand: `CLAUDE.md`.
- If the protocol and a brief disagree, follow the protocol and report the conflict.

## App map

All crates are workspace members in the root `Cargo.toml`.

| Path | Package | Owns |
|---|---|---|
| `crates/app` | `scuffed-app` | Dioxus 0.7 WASM app: site, admin, strategy editor, chat. `dx build` output is staged into `dist/`. |
| `crates/server` | `scuffed-server` | Production Axum binary: site-server routes plus strategy collab WebSocket (`collab/`, `routes/ws.rs`) and chat routes. Serves `dist/`. |
| `crates/site-server` | `scuffed-site-server` | Core REST API library (sessions, members, tournaments, uploads, membership policy) plus its own `main`. |
| `crates/types` | `scuffed-types` | Types shared by the app, servers and tracker. |
| `crates/api-client` | `scuffed-api-client` | HTTP client; web impl by default, `native` feature for native. |
| `crates/db` | `scuffed-db` | SurrealDB client, migrations, queries. Binary `scuffed-rewrap`. |
| `crates/auth` | `scuffed-auth` | OAuth, sessions, crypto (feature-gated). |
| `crates/chat` | `scuffed-chat` | Nostr chat backend: relay client, NIP-42 auth, NIP-29 groups. |
| `crates/relay-policy` | `relay-policy` | strfry write-policy plugin plus `relay-read-gate` and `relay-chat-flow` binaries. Built by `relay/Containerfile`; optional `relay` compose profile. |
| `crates/map-pipeline` | `scuffed-map-pipeline` | CLI: 2D tactical map tiles from 3D Overwatch map models (glTF). |
| `crates/map-renderer` | `scuffed-map-renderer` | CLI: orthographic floor views from GLB models with Bevy. |
| `crates/stat-tracker` | `scuffed-stat-tracker` | Overwatch OCR stat tracker daemon (Linux): reads the Tab scoreboard, stores games locally, optional sync to the site. Also `scuffed-pack-cutter`. |
| `crates/stat-tracker-core` | `scuffed-stat-tracker-core` | Bottom layer of the tracker: config, text parsing, shared model types, perf and host probes. Re-exported by `scuffed-stat-tracker`. |
| `crates/stat-tracker-ui` | `scuffed-stat-tracker-ui` | Iced 0.14 desktop GUI, binary `stat-tracker-gui`, including the companion overlay. |

The tracker crates declare `rust-version = "1.90"`. Legacy Leptos frontends are gone;
`scripts/check-frontend-deps.sh` blocks new Leptos deps.

## Build and test

CI uses Rust `stable` and sets `RUSTFLAGS="-D warnings"`. Commands below are copied from
the workflows; run them from the repo root unless stated.

### Site and backend (`.github/workflows/ci.yml`)

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --exclude scuffed-app --exclude scuffed-stat-tracker \
  --exclude scuffed-stat-tracker-core --exclude scuffed-stat-tracker-ui -- -D warnings
cargo clippy --target wasm32-unknown-unknown --all-targets -p scuffed-app -- -D warnings
cargo clippy -p scuffed-api-client --no-default-features --features native --all-targets -- -D warnings
cargo build --workspace --exclude scuffed-app --exclude scuffed-stat-tracker \
  --exclude scuffed-stat-tracker-core --exclude scuffed-stat-tracker-ui
cargo test --workspace --exclude scuffed-stat-tracker --exclude scuffed-stat-tracker-core \
  --exclude scuffed-stat-tracker-ui
cargo test -p scuffed-api-client --no-default-features --features native
(cd crates/app && dx build --release --debug-symbols false)   # dioxus-cli 0.7.9
```

The `Dependency Guardrails` job runs these scripts with `bash`:
`check-frontend-deps.sh`, `check-design-tokens.sh`, `check-compose-interpolation.sh`,
`check-surrealdb-version.sh`, `check-compose-env.sh`, `test-surreal-guard.sh`,
`test-backup-secrets.sh`, `test-restic-access.sh`, `test-publish-packs.sh` (all in `scripts/`).
On a pull request whose changed paths are all tracker-only (`crates/stat-tracker*/**`,
`docs/notes/stat-tracker-*`, `docs/superpowers/plans/*stat-tracker*`), Build & Test and
App Build are skipped.

Production build: `scripts/build.sh` runs the `dx build` above, copies
`target/dx/scuffed-app/release/web/public` to `dist/`, then `cargo build --release -p scuffed-server`.

### Stat tracker (`.github/workflows/stat-tracker.yml`)

Default `working-directory` is `crates/stat-tracker`, and
`TRACKER_PACKAGES="-p scuffed-stat-tracker -p scuffed-stat-tracker-core"` (every tracker
crate but the GUI). From `crates/stat-tracker`:

```sh
cargo fmt --check $TRACKER_PACKAGES
cargo build --locked $TRACKER_PACKAGES
cargo test --locked -p scuffed-stat-tracker --bin scuffed-stat-tracker \
  -- --exact daemon::poll::tests_cadence::capture_futures_stay_small   # stack guard, opt-level 0
cargo test --locked $TRACKER_PACKAGES --lib --bins --tests \
  --config "profile.test.package.scuffed-stat-tracker.opt-level=2" \
  --config "profile.test.package.scuffed-stat-tracker.debug-assertions=true" \
  --config "profile.test.package.scuffed-stat-tracker.overflow-checks=true"
cargo clippy --locked $TRACKER_PACKAGES --all-targets -- -D warnings
cargo clippy --locked -p scuffed-stat-tracker-core --all-targets -- -D warnings
```

Build & Test also runs the packaging scripts (`bash dist/test-*.sh`) and
`bash tests/check-no-game-art.sh` before the cargo steps.

GUI, separate job (`ui-overview`), from the repo root:

```sh
cargo fmt -p scuffed-stat-tracker-ui --check
cargo clippy --locked -p scuffed-stat-tracker-ui --all-targets -- -D warnings
cargo test --locked -p scuffed-stat-tracker-ui
```

### Relay (`.github/workflows/relay.yml`)

`bash relay/ci-chat-flow.sh` builds the relay image and walks the chat path. It triggers
only on pushes and PRs to `main` that touch `relay/`, `relay-policy`, `db`, `auth`,
`types` or the workspace manifests, so it does not run on PRs into `wip/0.7`.

## File-size guard (tracker crates)

Defined in `crates/stat-tracker/src/tests_file_size.rs` and run by the tracker test suite.
Every `.rs` file under `src/`, `tests/` and `examples/` of `stat-tracker`,
`stat-tracker-core` and `stat-tracker-ui` must stay at or under 1,500 lines. Files that
were over the limit when the guard landed are listed in `CAPS` and may only shrink:
never raise a cap, lower it when the file shrinks (the test fails if a file is under its
cap), and drop a file from `CAPS` once it is at or under 1,500 lines. New files start
under the limit; split instead of growing.

Read `CAPS` in the file for current values. No equivalent guard exists for the other
crates; this one covers the three tracker crates only.

## Conventions

- Branches: open PRs against the active dev branch, today `wip/0.7` (not `main`).
  `wip/0.7.1` carries tracker companion work; target it only when the brief says so.
- PRs are drafts with a HOLD-MERGE body: what changed, how it was tested, risks.
- Merges are squash merges, done only with the repo owner's go. Agents never merge.
- Fix review nits in the same PR, not in a follow-up PR.
- Commit and open PRs as the configured `FrozenTear` identity. Do not put personal names
  in commits, PRs or docs.
- Call the game Overwatch. No em dashes in docs, commit messages or PR bodies.
- Privacy: crops, screenshots and anything under `crates/stat-tracker/test-data/` stay
  local and are never committed (copyrighted game captures). `.gitignore` ignores `*.png`
  outside a short allowlist; do not add exceptions for captures.
- More project rules (SurrealDB binds, auth extractors, toasts, audit log): `CLAUDE.md`.

## Cursor Cloud specific instructions

**Env.** To build or test, a cloud VM needs what CI installs:

- Rust `stable` with `rustfmt`, `clippy` and the `wasm32-unknown-unknown` target.
- `dioxus-cli` 0.7.9 (`dx`) for the app build only.
- Site jobs: `libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libvulkan-dev pkg-config`.
- Tracker jobs: `libwayland-dev libxkbcommon-dev libegl-dev libgbm-dev tesseract-ocr
  libtesseract-dev libleptonica-dev libevdev-dev libudev-dev pkg-config minisign`.
- GUI job adds: `libasound2-dev libvulkan-dev libssl-dev libgtk-3-dev
  libayatana-appindicator3-dev libxdo-dev`.

A cloud agent already has its own checkout. The protocol's IRON LAW (work only in
`.claude/worktrees/<agent>-<topic>`) protects the shared host checkout; it does not
require a worktree inside a Cursor cloud VM.

**Do not:**

- start Memtrace or any fleet stack, or commit `test-data/`, crops or screenshots;
- edit `.github/workflows/` unless the brief says to;
- merge, tag, force-push or publish a release;
- run the full tracker test suite or a release build unless the brief needs it
  (stat-tracker CI is heavy; its Build & Test job allows 105 minutes);
- run cargo at all for a docs-only change.

**Scope.** Touch only the files the brief names. Do not tidy unrelated code. If you are
blocked, stop and report instead of widening the change.
