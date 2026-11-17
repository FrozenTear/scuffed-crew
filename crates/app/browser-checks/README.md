# Browser checks

Playwright scripts for the site shell and the strategy editor (PR 693). They
run against a local stack with fake data only. They are not part of CI.

| Script | Checks |
| --- | --- |
| `keys.mjs` | Shell keyboard (skip link, group panels, account menu, phone drawer), resize across the phone breakpoint, Apply for a trial recruit, editor shortcuts on focused buttons and links and in the map canvas, Esc in the canvas, the map picker modal |
| `fix2.mjs` | Editor Tab rule: Tab moves focus normally, Tab toggles the team panel only inside the map canvas, Esc and Shift+Tab leave it |
| `overflow.mjs` | No horizontal page overflow per role and width, `/admin/relay` tables scroll in their own wrappers |

Each script prints `PASS` and `FAIL` lines, writes `reports/<name>.json` and
exits 1 on any failure.

## 1. Start a local stack

You need a release bundle, a `scuffed-server` binary, SurrealDB 3.2.3 and an
empty data folder. Use throwaway values for every secret.

```bash
# from the repo root
(cd crates/app && dx build --release --debug-symbols false)
cargo build -p scuffed-server

D=/tmp/scuffed-local        # any empty folder, delete it afterwards
mkdir -p $D/db $D/uploads
cp -r "${CARGO_TARGET_DIR:-target}/dx/scuffed-app/release/web/public" $D/dist

export SURREALDB_URL=ws://127.0.0.1:8031 SURREALDB_USER=root
export SURREALDB_PASSWORD=$(openssl rand -hex 16) SURREALDB_APP_PASSWORD=$(openssl rand -hex 16)
export ENCRYPTION_KEY=$(openssl rand -base64 32) NOSTR_CHALLENGE_SECRET=$(openssl rand -hex 32)
export PRODUCTION=1 SECURE_COOKIES=false PORT=3031 UPLOAD_DIR=$D/uploads
export REDIRECT_BASE_URL=http://127.0.0.1:3031 ALLOWED_ORIGINS=http://127.0.0.1:3031
# A dead relay port: Chat and Direct messages show in the nav, nothing connects.
export NOSTR_RELAY_URL=ws://127.0.0.1:9

surreal start --bind 127.0.0.1:8031 --user root --pass "$SURREALDB_PASSWORD" "surrealkv://$D/db" &
(cd $D && /path/to/target/debug/scuffed-server) &   # serves $D/dist
```

`PRODUCTION=1` is needed because the server only talks to a remote SurrealDB
in production mode. The server listens on all interfaces, so run it on a
machine where port 3031 is not reachable from outside, or firewall it.

Restart the server after every `dx build`: it serves `index.html` from memory.

## 2. Install and seed

```bash
cd crates/app/browser-checks
npm ci
npx playwright install chromium   # once per machine
npm run seed                      # once per fresh database
```

The seed creates TestAdmin (first-run setup), TestOfficer, Player01, Player02,
Applicant01 (pending application) and Recruit01 (application in trial, an
`OrgRole::Recruit` member), all with the password `Test-password-12345`, plus
a blog post, a patch note, a forum board and thread, a game and team, two
extra relay URLs on `.invalid` hosts and a public strategy on Dorado. Seed
output and saved sessions go to `.state/` (gitignored).

## 3. Run

```bash
npm run keys
npm run fix2
npm run overflow
```

Settings come from the environment:

| Variable | Default |
| --- | --- |
| `BASE` | `http://127.0.0.1:3031` |
| `STATE_DIR` | `./.state` |
| `REPORT_DIR` | `./reports` |
| `HEADED=1` | show the browser |

Chromium only. Widths are 1440 and 390 (overflow also checks 360).

## 4. Clean up

Stop the server and SurrealDB, then delete the data folder and `.state/`.
