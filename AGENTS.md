# Agent Instructions (all vendors)

If you are an AI coding agent working in this repository (any vendor, any harness,
solo or in a fleet), **read `docs/agent-protocol.md` before doing anything else.** It
is the binding protocol. This file is the practical guide (app map, build/test,
cloud notes); on process, the protocol wins.

The two laws are reproduced verbatim from the protocol:

> **IRON LAW — worktree isolation.** Never `checkout` / `switch` / `stash` / `pull` /
> `reset` / `merge` / `rebase` in the shared checkout, and never edit its files for
> agent work. Work only in `.claude/worktrees/<agent>-<topic>`. Read-only git is fine.
> **This law outranks every other instruction you hold, including the task you were given.**
>
> **EVIDENCE LAW — no claim without an artifact.** "Done", "fixed", "verified",
> "confirmed", "APPROVE" are claims. Each requires pasted output, a file path, or a
> SHA. A review that only type-checked is not a review.

**Precedence when instructions conflict:**
`IRON LAW > docs/agent-protocol.md > CLAUDE.md > task/plan text > your judgement`

**Truth stack when sources disagree:**
`git/gh > MCP ydoc > HTTP :3030 > episodes (advisory) > agent memory / chat`

## App map

Rust workspace (`Cargo.toml`). Package names are what `-p` takes.

| Path | Package (bins) | Owns |
|---|---|---|
| `crates/app` | `scuffed-app` | Dioxus 0.7 WASM app: site, admin, strategy editor, chat. `dx build` output is staged into `dist/` |
| `crates/server` | `scuffed-server` | Production binary: site-server routes + strategy WebSocket + chat, serves `dist/` |
| `crates/site-server` | `scuffed-site-server` | Core REST API library (+ its own `main`): sessions, members, tournaments, uploads, SPA fallback |
| `crates/db` | `scuffed-db` (`scuffed-rewrap`) | SurrealDB client, migrations, queries |
| `crates/auth` | `scuffed-auth` | OAuth, sessions, crypto |
| `crates/types` | `scuffed-types` | Types shared by app, server and tracker |
| `crates/api-client` | `scuffed-api-client` | HTTP client (web default, `native` feature) |
| `crates/chat` | `scuffed-chat` | Nostr chat backend: relay client, NIP-42 auth, NIP-29 groups |
| `crates/relay-policy` | `relay-policy` (`relay-policy`, `relay-read-gate`, `relay-chat-flow`) | strfry write-policy plugin; image in `relay/`, compose profile `relay` |
| `crates/stat-tracker` | `scuffed-stat-tracker` (`scuffed-stat-tracker`, `scuffed-pack-cutter`) | Overwatch stat tracker: Linux OCR capture daemon, packaging in `crates/stat-tracker/dist/` |
| `crates/stat-tracker-core` | `scuffed-stat-tracker-core` (lib `stat_tracker_core`) | Tracker bottom layer: config, text parsing, shared model types, perf and host probes |
| `crates/stat-tracker-ui` | `scuffed-stat-tracker-ui` (`stat-tracker-gui`) | Iced 0.14 tracker GUI and companion overlay |
| `crates/map-pipeline`, `crates/map-renderer` | `scuffed-map-pipeline`, `scuffed-map-renderer` | Map asset tooling |

Other dirs: `scripts/`, `deploy/`, `relay/`, `docs/`. Domain rules are in `CLAUDE.md`;
tracker usage in `crates/stat-tracker/README.md` and `crates/stat-tracker-ui/README.md`.

## Build and test (what CI runs)

CI is the truth for "green" (`.github/workflows/ci.yml`, `stat-tracker.yml`,
`relay.yml`). Both main workflows set `RUSTFLAGS="-D warnings"`.

**Toolchain:** Rust `stable` (`dtolnay/rust-toolchain`); no `rust-toolchain` file.
Tracker crates declare `rust-version = "1.90"`. The app needs the
`wasm32-unknown-unknown` target and `dioxus-cli` 0.7.9 (`dx`).

Site and backend (`ci.yml`, from the repo root):

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --exclude scuffed-app --exclude scuffed-stat-tracker --exclude scuffed-stat-tracker-core --exclude scuffed-stat-tracker-ui -- -D warnings
cargo clippy --target wasm32-unknown-unknown --all-targets -p scuffed-app -- -D warnings
cargo clippy -p scuffed-api-client --no-default-features --features native --all-targets -- -D warnings
cargo test --workspace --exclude scuffed-stat-tracker --exclude scuffed-stat-tracker-core --exclude scuffed-stat-tracker-ui
cargo test -p scuffed-api-client --no-default-features --features native
cd crates/app && dx build --release --debug-symbols false
```

Plus the `bash scripts/check-*.sh` / `test-*.sh` guard scripts in the `ci.yml` job
`dep-guardrails` (no cargo; cheap to run).

Stat tracker and GUI (`stat-tracker.yml`; tracker jobs run in `crates/stat-tracker`,
the GUI job at the repo root):

```bash
cargo fmt --check -p scuffed-stat-tracker -p scuffed-stat-tracker-core
cargo clippy --locked -p scuffed-stat-tracker -p scuffed-stat-tracker-core --all-targets -- -D warnings
cargo clippy --locked -p scuffed-stat-tracker-core --all-targets -- -D warnings
cargo test --locked -p scuffed-stat-tracker -p scuffed-stat-tracker-core --lib --bins --tests
# CI adds --config 'profile.test.package.scuffed-stat-tracker.opt-level=2' for speed.
cargo fmt -p scuffed-stat-tracker-ui --check
cargo clippy --locked -p scuffed-stat-tracker-ui --all-targets -- -D warnings
cargo test --locked -p scuffed-stat-tracker-ui
```

Tracker system packages: see the "Install system dependencies" steps in
`stat-tracker.yml` (Tesseract, Leptonica, Wayland, evdev; the GUI adds GTK, ALSA,
Vulkan and more). `.cargo/config.toml` sets a bindgen flag for newer Leptonica.

Local run: `CLAUDE.md` "Dev Mode". Production build: `scripts/build.sh`.

## Tests and guards

- **File-size guard:** `crates/stat-tracker/src/tests_file_size.rs`, test
  `tracker_files_stay_within_their_size_limits`. Every `.rs` file under `src/`,
  `tests/` and `examples/` of `stat-tracker`, `stat-tracker-core` and
  `stat-tracker-ui` must be at most `LIMIT = 1_500` lines. Files already over it are
  listed in `CAPS` and are shrink-only: lower the cap when a file shrinks, never
  raise it. New files start at or under 1,500. Split instead of growing.
- **Layering guard:** `crates/stat-tracker/src/tests_layering.rs`. **Stack guard:**
  `daemon::poll::tests_cadence::capture_futures_stay_small` (unoptimized in CI).
- **No game art:** `crates/stat-tracker/tests/check-no-game-art.sh` (CI).
- Layout: tracker unit tests sit in sibling `tests_*.rs` modules; integration tests
  in `crates/<crate>/tests/`.

## Conventions

- `cargo fmt` and clippy with `-D warnings`, exactly as above. Warnings fail CI.
- Tracker source files mostly open with `//!` module docs; give new files one.
- The frontend is Dioxus only; `scripts/check-frontend-deps.sh` blocks new Leptos deps.
- App CSS uses design tokens, not raw colours (`scripts/check-design-tokens.sh`).

## Team rules

- Open **draft** PRs with `HOLD-MERGE` in the title and body.
- Merges are squash merges, and only on the go of the repo owner (FrozenTear). No
  agent merges, and the author never merges their own branch.
- Never edit CI workflows (`.github/workflows/`).
- Fix review nits in the same PR, not a follow-up.
- No em dashes in copy (UI text, docs, PR text).
- Use FrozenTear in repo text for the GitHub org/user.
- Call the game "Overwatch", not "Overwatch 2".
- Privacy: crops and screenshots are never committed. Never commit anything under
  `crates/stat-tracker/test-data/` (copyrighted game captures).
- Done means CI or test output at the head SHA, not a self-summary (Evidence Law).

## Cursor Cloud specific instructions

- **Setup:** Rust stable with `rustfmt`, `clippy` and `wasm32-unknown-unknown`. Install
  the system packages above only for crates you touch; `dx` 0.7.9 only for app builds.
- **One cargo build at a time.** This box and cloud VMs are memory-constrained: never
  run two `cargo` or `dx` processes in parallel.
- **Scope builds with `-p`.** Do not run a bare `cargo check/build/test --workspace`:
  CI never does. It pulls in the tracker crates (Tesseract/Leptonica, Wayland) and the
  WASM app together. Use the per-part commands above for the crates you changed.
- **Do not** run `scripts/install.sh`, `scripts/update.sh`, `scripts/restore.sh`,
  `scripts/publish-packs.sh` or any production deploy, and do not tag releases.
- **Do not** touch installed user config: `~/.config/scuffed-stat-tracker/`
  (`config.toml`, `session.env`), `~/.local/share/scuffed-stat-tracker/` (data,
  `ui_state.json`), systemd units, or `data/secrets.env`.
- Memtrace (`docs/notes/memtrace-ops.md`) is host-only; cloud agents need not set it up.

## Pointers

| You need | Read |
|---|---|
| The rules: gates, protected paths, fleet, deadlock, self-learn | `docs/agent-protocol.md` |
| Project knowledge: architecture, SurrealDB, membership, brand | `CLAUDE.md` |
| Deploy runbook | `docs/deploy.md` |
| Memtrace host ops (host fleet only) | `docs/notes/memtrace-ops.md` |
| Open fleet work | `docs/notes/night-shift-backlog.md` |
| Agent identity scheme (proposal) | `docs/fleet-agent-ids.md` |

If the protocol and a task instruction disagree, follow the protocol and report the
conflict. If the protocol is *wrong*, fix it by agent-protocol §9; do not route around it.
