# The Scuffed Crew

Community site and tooling for the Scuffed Crew gaming org. One Rust monorepo:
the public site and admin panel, the backend that serves them, and the desktop
tooling we use day to day — including an OCR stat tracker for Overwatch 2
scoreboards.

Built with [Dioxus](https://dioxuslabs.com) (WASM frontend), [Axum](https://github.com/tokio-rs/axum)
(backend), and [SurrealDB](https://surrealdb.com) (storage).

## What's here

| Crate | What it is |
|---|---|
| `crates/app` | Dioxus 0.7 WASM app — site, admin panel, strategy editor, chat |
| `crates/server` | Unified Axum binary (`scuffed-server`) — REST API, strategy WebSocket, chat |
| `crates/site-server` | Core REST API library — sessions, members, tournaments, uploads |
| `crates/types` | Shared types between app and server |
| `crates/api-client` | HTTP client (web + native) |
| `crates/db` | SurrealDB client, migrations, queries |
| `crates/auth` | OAuth, sessions, crypto |
| `crates/chat` | Nostr chat backend (relay client, NIP-44/59) |
| `crates/stat-tracker` | Overwatch 2 stat tracker — OCR capture daemon + desktop GUI |
| `crates/map-pipeline`, `crates/map-renderer` | Map asset tooling for the strategy editor |
| `crates/relay-policy` | Nostr relay policy plugin (NIP-42/29, rate limits) |

Production is `scuffed-server` serving the `dx build` output — see
`scripts/build.sh` and the `Containerfile`.

## Run it locally

Prereqs: stable Rust, the [Dioxus CLI](https://dioxuslabs.com/learn/0.7/getting_started/) (`dx`).

```sh
# backend on :3030 — no database needed: dev mode runs an in-memory
# SurrealDB with seeded data (user "devadmin", admin role)
PORT=3030 cargo run -p scuffed-server

# frontend with hot reload
cd crates/app && dx serve
```

Hit `/api/dev/login` once to get a session cookie, then `/admin/` is yours.
The dev login route only exists in dev mode.

## Self-hosting

`./scripts/install.sh` does the full setup on a fresh box: generates secrets
into `data/secrets.env`, picks a free port, and brings everything up with
Podman Compose. First visit in the browser creates the admin account.

The full runbook — updates, backups, restore drills, systemd — is in
[`docs/deploy.md`](docs/deploy.md). Environment reference for power users:
[`.env.example`](.env.example).

Production refuses to start with default database credentials and requires an
encryption key (`PRODUCTION=1` gate).

## Stat tracker

A desktop daemon that watches Overwatch 2, OCRs the scoreboard when you hit
Tab, and keeps per-game stats locally (optionally syncing to your org's site).
Ships as a portable tarball per release — see the
[releases page](https://github.com/FrozenTear/scuffed-crew/releases) and
[`crates/stat-tracker/README.md`](crates/stat-tracker/README.md). Linux-first;
capture via the desktop portal or wlroots screencopy.

## How this repo is built

A large share of this codebase is written and reviewed by a small fleet of AI
coding agents working under human direction, with every change peer-reviewed
before merge. The coordination rules live in
[`docs/fleet-protocol.md`](docs/fleet-protocol.md) — the short version: agents
work in isolated worktrees, nobody merges their own branch, and all findings
land on a shared log. Design and review notes accumulate in
[`docs/notes/`](docs/notes/).

## Community

Scuffed Crew is a no-drama org: direct communication, no politics, 16+.
The site is the front door — if you're reading this because you want in,
that's where applications happen.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in the work by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.

The code is yours to reuse; the **Scuffed Crew** name and identity are not —
don't present a fork as us.
