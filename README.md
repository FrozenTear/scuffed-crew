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
| `crates/stat-tracker` | Overwatch 2 stat tracker — OCR capture daemon |
| `crates/stat-tracker-ui` | Iced 0.14 desktop GUI (`stat-tracker-gui`) |
| `crates/stat-tracker-core` | Tracker base layer shared by the daemon and GUI: config, parsing, shared model types |
| `crates/map-pipeline`, `crates/map-renderer` | Map asset tooling for the strategy editor |
| `crates/relay-policy` | Nostr relay policy plugin (NIP-42/29, rate limits) |

Production is `scuffed-server` serving the `dx build` output — see
`scripts/build.sh` and the `Containerfile`.

### Site stats pages

The site shows the stats the tracker uploads. Win rates follow one small-sample
rule everywhere: a win rate shows from 10 decided games, and below that the page
shows the record (wins, losses, draws) under "Too few games".

- `/stats` is My Stats for the signed-in member, and `/stats/match/:id` is one of
  their own games: result, map, mode, hero, role, date and all six stat lines
  (eliminations, assists, deaths, damage, healing, mitigation). Values the reader
  was unsure about carry a mark, and a game corrected in the tracker shows an
  "Edited" badge. A game that is missing or belongs to someone else shows "not in
  your match history".
- `/matches/:id` is the public page for a team fixture: score, map, mode, date,
  and the VOD link and replay code when there are any.
- `/leaderboards` has Win rate, K/D and Games boards, with hero and season
  filters. The server only sends players with 5 or more games on the Win rate and
  K/D boards. The Win rate board ranks players with 10 or more decided games and
  lists the rest below it under "Too few games" with their record. The K/D and
  Games boards are not split that way.

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
