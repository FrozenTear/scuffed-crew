# Review Fixes + Legacy Leptos Removal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply the three verified high-priority review fixes, delete the legacy Leptos stack (`crates/site`, `crates/admin`, `crates/ui`), and update `CLAUDE.md`.

**Architecture:** The Dioxus app (`crates/app`) served by `crates/server` (which reuses `crates/site-server`'s router) is a feature-complete superset of the legacy Leptos site/admin. The Nostr identity page — the one feature flagged as possibly unported — is confirmed present and enhanced in `crates/app/src/pages/identity.rs`. Removing the Leptos crates therefore loses no functionality. The static-serving fallback in `site-server/src/lib.rs:402` serves `dist/`, which the Dioxus build (not trunk) populates.

**Tech Stack:** Rust, Dioxus 0.7 (WASM), Axum 0.8, SurrealDB v3, tower-http, tower_governor (new).

**Scope corrections discovered during planning:**
- Pagination limits are **already clamped** (`types/src/api/mod.rs:64` clamps 1–100; `strategy.rs:104` does `.min(100)`). The only custom-query routes (forum/articles/wiki/moderation/scrims) pass `query.limit`/`query.offset` straight to the DB **unclamped** — these are the real gaps, narrower than the review claimed.
- `undo.rs:254/263` and `tools.rs:243` `unwrap()`s are inside `#[cfg(test)]` — not production panics. Excluded.
- The `auth` crate's `client` feature (leptos) becomes dead after removal but is harmless; flagged as a follow-up, not touched here.

---

## Task 1: Fix SurrealQL injection in public strategy listing

**Files:**
- Modify: `crates/db/src/queries/strategies.rs:280-331`

The `game_mode` value is interpolated raw into the WHERE clause (`format!("game_mode = '{gm}'")`), and `search` uses manual quote-escaping. Replace both with bound parameters using placeholder names in the dynamically-built clause.

- [ ] **Step 1: Replace the dynamic WHERE construction with bound placeholders**

In `get_public_strategies`, change the condition-building block (lines ~288-300) and thread the binds through both the count and data queries:

```rust
// Build WHERE clause with bound placeholders (no raw interpolation).
let mut conditions = vec!["visibility = 'public'".to_string()];
if game_mode.is_some() {
    conditions.push("game_mode = $game_mode".to_string());
}
if search.is_some() {
    conditions.push(
        "string::lowercase(name) CONTAINS string::lowercase($search)".to_string(),
    );
}
let where_clause = conditions.join(" AND ");

// Count total
let count_query =
    format!("SELECT count() as total FROM strategy WHERE {where_clause} GROUP ALL");
let mut count_q = self.client.query(&count_query);
if let Some(gm) = game_mode {
    count_q = count_q.bind(("game_mode", gm.to_string()));
}
if let Some(s) = search {
    count_q = count_q.bind(("search", s.to_string()));
}
let mut count_result = count_q.await?;

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    total: i64,
}
let count_rows: Vec<CountRow> = count_result.take(0)?;
let total = count_rows.first().map(|r| r.total as u64).unwrap_or(0);

// Fetch page
let data_query = format!(
    "SELECT *, array::len(elements) as element_count \
     FROM strategy WHERE {where_clause} \
     ORDER BY updated_at DESC LIMIT $lim START $off"
);
let mut data_q = self
    .client
    .query(&data_query)
    .bind(("lim", limit as i64))
    .bind(("off", offset as i64));
if let Some(gm) = game_mode {
    data_q = data_q.bind(("game_mode", gm.to_string()));
}
if let Some(s) = search {
    data_q = data_q.bind(("search", s.to_string()));
}
let mut data_result = data_q.await?;
let rows: Vec<DbStrategySummary> = data_result.take(0)?;
```

Note: `where_clause` still uses `format!`, but only with fixed `$placeholder` strings — no user data is interpolated. The `q.replace('\'', "''")` escaping is removed entirely.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scuffed-db`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/db/src/queries/strategies.rs
git commit -m "fix(db): parameterize public strategy filter to close SurrealQL injection"
```

---

## Task 2: Stop swallowing DB errors in DM latest-message query

**Files:**
- Modify: `crates/db/src/queries/dms.rs:196`

- [ ] **Step 1: Propagate the error instead of defaulting to empty**

```rust
let rows: Vec<DbDmMessage> = result.take(0)?;
```
(was `result.take(0).unwrap_or_default();`)

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scuffed-db`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/db/src/queries/dms.rs
git commit -m "fix(db): propagate query errors in dm latest-message lookup"
```

---

## Task 3: Remove panic-prone unwraps in the canvas renderer

**Files:**
- Modify: `crates/app/src/canvas/renderer.rs:69,70,109,181`

Canvas 2D context calls almost never fail, but `unwrap()` on them panics the whole WASM app. These are inside non-`Result` drawing methods, and a failed draw is non-fatal, so discard the result.

- [ ] **Step 1: Replace the four unwraps with discards**

Line 69-70 (`begin_frame`):
```rust
let _ = self.ctx.translate(self.pan.x, self.pan.y);
let _ = self.ctx.scale(self.zoom, self.zoom);
```

Line 109 (`draw_circle`, the `.arc(...)` call):
```rust
let _ = self
    .ctx
    .arc(pos.x, pos.y, radius, 0.0, std::f64::consts::PI * 2.0);
```

Line 181 (`fill_text`):
```rust
let _ = self.ctx.fill_text(text, pos.x, pos.y);
```

- [ ] **Step 2: Verify it compiles for wasm**

Run: `cargo check -p scuffed-app --target wasm32-unknown-unknown`
Expected: clean compile (clippy `-D warnings` tolerates `let _ =`).

- [ ] **Step 3: Commit**

```bash
git add crates/app/src/canvas/renderer.rs
git commit -m "fix(app): avoid panicking on canvas context errors in renderer"
```

---

## Task 4: Remove panic-prone unwraps in the avatar upload flow

**Files:**
- Modify: `crates/app/src/pages/admin/members.rs:340-423`

The avatar upload chains six unwraps on `web_sys`. The `resp.text().unwrap()` at line 388 in particular makes the existing error toast at line 412 unreachable. Make each fallible call degrade to a toast instead of a panic.

- [ ] **Step 1: Guard the file-input DOM access (`on_avatar_file_change`, lines 340-351)**

```rust
let on_avatar_file_change = move |_e: Event<FormData>| {
    // Access the file input via DOM query to get the web_sys::File
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Some(el) = document.get_element_by_id("avatar-file-input")
        && let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>()
        && let Some(file_list) = input.files()
        && let Some(file) = file_list.get(0)
    {
        avatar_file.set(Some(file));
    }
};
```

- [ ] **Step 2: Guard FormData / Request / window in `on_avatar_submit` (lines 367-388)**

Replace the `unwrap()`s on `FormData::new()`, `Request::new_with_str_and_init`, `web_sys::window()`, and `resp.text()` with early-return-plus-toast:

```rust
spawn(async move {
    // Upload via FormData
    let Ok(form_data) = web_sys::FormData::new() else {
        toast.show(Toast::error("Could not prepare upload."));
        avatar_uploading.set(false);
        return;
    };
    let _ = form_data.append_with_blob("file", &file);

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_body(&form_data.into());
    opts.set_credentials(web_sys::RequestCredentials::SameOrigin);

    let Ok(request) =
        web_sys::Request::new_with_str_and_init("/api/upload/avatar", &opts)
    else {
        toast.show(Toast::error("Could not build upload request."));
        avatar_uploading.set(false);
        return;
    };

    let Some(window) = web_sys::window() else {
        toast.show(Toast::error("Upload failed: no browser window."));
        avatar_uploading.set(false);
        return;
    };
    let resp_val =
        wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request)).await;

    match resp_val {
        Ok(resp_val) => {
            let resp: web_sys::Response = resp_val.unchecked_into();
            if resp.ok() {
                let text_promise = match resp.text() {
                    Ok(p) => p,
                    Err(_) => {
                        toast.show(Toast::error("Failed to read upload response."));
                        avatar_uploading.set(false);
                        return;
                    }
                };
                let text = wasm_bindgen_futures::JsFuture::from(text_promise).await;
                // ... existing body unchanged from `if let Ok(text) = text {` onward
```

The rest of the match arm (the `if let Ok(text) = text` block, lines 389-419) stays as-is.

- [ ] **Step 3: Verify it compiles for wasm**

Run: `cargo check -p scuffed-app --target wasm32-unknown-unknown`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/pages/admin/members.rs
git commit -m "fix(app): surface avatar upload failures as toasts instead of panics"
```

---

## Task 5: Add HTTP timeout to the native api-client

**Files:**
- Modify: `crates/api-client/src/native_impl.rs:1-139`

Every function builds `reqwest::Client::new()` with no timeout. Introduce a shared, timeout-configured client builder and use it everywhere.

- [ ] **Step 1: Add a client constructor and replace the four `Client::new()` sites**

At the top of the file, after the `use`:
```rust
use std::time::Duration;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
```

Then replace each `let client = reqwest::Client::new();` (in `json_request`, `get`, `post_empty`, `delete`) with:
```rust
let client = client();
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scuffed-api-client`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/api-client/src/native_impl.rs
git commit -m "fix(api-client): add 30s request timeout to native http client"
```

---

## Task 6: Gate the dev-login route to dev mode only

**Files:**
- Modify: `crates/site-server/src/lib.rs:43-49`

`/api/dev/login` is registered unconditionally. The matching dev session only exists when seeded in in-memory mode, so it's inert in production — but the route should not exist there at all. Gate registration on the same `SURREALDB_URL`-unset signal `main.rs` already uses for `is_dev`.

- [ ] **Step 1: Conditionally add the route**

In `create_router`, replace the unconditional `.route("/api/dev/login", ...)` with a conditional builder. Build the base router, then add the dev route only in dev mode:

```rust
    let dev_mode = std::env::var("SURREALDB_URL").is_err();

    let mut router = Router::new()
        // NIP-05 Nostr identity verification (must be before fallback)
        .route("/.well-known/nostr.json", get(routes::nostr::nostr_json))
        // Health check
        .route("/api/health", get(routes::health::health));

    if dev_mode {
        // Dev login (sets session cookie for in-memory dev mode) — never registered in prod.
        router = router.route("/api/dev/login", get(routes::dev::dev_login));
    }

    router
        // Auth routes
        .route("/api/auth/{provider}/login", get(routes::auth::login))
        // ... rest of the chain unchanged
```

Remove the old `// Dev login` comment + `.route("/api/dev/login", ...)` line from the main chain (lines 48-49).

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scuffed-site-server`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/site-server/src/lib.rs
git commit -m "fix(site-server): only register /api/dev/login in dev mode"
```

---

## Task 7: Clamp limits on the custom-query list routes

**Files:**
- Modify: `crates/site-server/src/routes/forum.rs` (2 handlers), `articles.rs` (2 handlers), `wiki.rs`, `moderation.rs`, `audit_log.rs`

These routes use their own query structs with `default_limit()` and pass `query.limit` straight to the DB without an upper bound (unlike the shared `PaginationParams::resolve()` which clamps to 100). Clamp each at the call site.

- [ ] **Step 1: Add `.min(100)` at each DB call site**

For each handler, change `query.limit` (or `q.limit`) in the DB call to `query.limit.min(100)`:

- `forum.rs:101`: `.list_forum_threads(query.category.as_deref(), query.limit.min(100), query.offset)`
- `forum.rs:145`: `.list_forum_replies(&id, query.limit.min(100), query.offset)`
- `articles.rs:34`: `.list_published_articles(query.limit.min(100), query.offset)`
- `articles.rs:55`: `.list_all_articles(query.limit.min(100), query.offset)`
- `wiki.rs:40`: `.list_wiki_pages(query.q.as_deref(), query.limit.min(100), query.offset)`
- `moderation.rs:85`: `.list_all_moderation(q.limit.min(100), q.offset)`
- `audit_log.rs:36`: `.list_audit_log(query.limit.min(100), query.offset)`

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scuffed-site-server`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add crates/site-server/src/routes/forum.rs crates/site-server/src/routes/articles.rs crates/site-server/src/routes/wiki.rs crates/site-server/src/routes/moderation.rs crates/site-server/src/routes/audit_log.rs
git commit -m "fix(site-server): clamp page size to 100 on custom-query list routes"
```

---

## Task 8: Rate-limit the OAuth login & callback routes

**Files:**
- Modify: `crates/site-server/Cargo.toml` (add dep)
- Modify: `crates/site-server/src/lib.rs` (apply limiter layer)

Add a per-IP rate limit to `/api/auth/{provider}/login` and `/api/auth/{provider}/callback` using `tower_governor`.

- [ ] **Step 1: Add the dependency**

In `crates/site-server/Cargo.toml` `[dependencies]`:
```toml
tower_governor = "0.4"
```

- [ ] **Step 2: Build a limiter and apply it to the auth routes**

In `lib.rs`, before constructing the router:
```rust
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

let governor_conf = Arc::new(
    GovernorConfigBuilder::default()
        .per_second(2)
        .burst_size(5)
        .finish()
        .expect("valid governor config"),
);
let auth_limit = GovernorLayer { config: governor_conf };
```

Apply the layer to the two auth routes by splitting them into a sub-router that carries the layer, then merging:
```rust
let auth_routes = Router::new()
    .route("/api/auth/{provider}/login", get(routes::auth::login))
    .route("/api/auth/{provider}/callback", get(routes::auth::callback))
    .layer(auth_limit);
```
Remove those two `.route(...)` lines from the main chain and `.merge(auth_routes)` into the final router before `.with_state(state)`.

(If `GovernorLayer`/`GovernorConfigBuilder` field names differ in the resolved 0.4.x, adapt to that version's API — the intent is 5-burst / 2-per-second per IP on those two routes.)

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p scuffed-site-server`
Expected: clean compile. If `tower_governor` 0.4 is incompatible with axum 0.8, fall back to the latest 0.x that is, or a minimal hand-rolled `Mutex<HashMap<IpAddr, (Instant, u32)>>` middleware.

- [ ] **Step 4: Commit**

```bash
git add crates/site-server/Cargo.toml crates/site-server/src/lib.rs Cargo.lock
git commit -m "feat(site-server): rate-limit oauth login and callback per IP"
```

---

## Task 9: Delete the legacy Leptos crates

**Files:**
- Delete: `crates/site/`, `crates/admin/`, `crates/ui/`
- Modify: root `Cargo.toml` members list

- [ ] **Step 1: Remove the directories**

```bash
git rm -r crates/site crates/admin crates/ui
```

- [ ] **Step 2: Remove them from the workspace members**

In root `Cargo.toml`, delete the three lines `"crates/ui",`, `"crates/site",`, `"crates/admin",`, and update the leading comment block that calls them an "approved exception" to note the migration is complete. Resulting `members` keeps `auth, site-server, db, types, app, server, api-client, map-pipeline, map-renderer, relay-policy, chat, stat-tracker`.

- [ ] **Step 3: Verify the workspace still resolves**

Run: `cargo metadata --no-deps --format-version 1 > /dev/null`
Expected: no error about missing/duplicate members.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: remove legacy Leptos site/admin/ui crates (Dioxus migration complete)"
```

---

## Task 10: Purge Leptos references from build/CI tooling

**Files:**
- Modify: `scripts/build.sh`, `scripts/check-frontend-deps.sh`, `Containerfile`, `.github/workflows/ci.yml`

- [ ] **Step 1: Rewrite `scripts/build.sh`**

Replace the trunk-based site/admin builds. The Dioxus app builds into `dist/` via `dx build`; the server binary builds with cargo:
```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> Building Dioxus app (crates/app -> dist/)"
cd "$ROOT/crates/app"
dx build --release

echo "==> Building server (scuffed-server)"
cd "$ROOT"
cargo build --release -p scuffed-server

echo "==> Done"
echo "    dist/index.html — Dioxus app"
echo "    target/release/scuffed-server"
```

- [ ] **Step 2: Trim `scripts/check-frontend-deps.sh`**

Remove `"crates/site"`, `"crates/admin"`, `"crates/ui"` from `ALLOWED_CRATES` (leaving only `"crates/auth"`), and update the header comment to drop those crates from the "approved exceptions" list.

- [ ] **Step 3: Update `Containerfile`**

- Remove the `cargo install ... trunk` line (keep `rustup target add wasm32-unknown-unknown`).
- Remove the `COPY crates/ui/Cargo.toml`, `crates/site/Cargo.toml`, `crates/admin/Cargo.toml` lines.
- Remove `crates/ui/src`, `crates/site/src`, `crates/admin/src` from the dummy `mkdir`/`echo` block.
- Remove the two `RUN cd crates/site && trunk build --release` / `crates/admin` steps.
- The `dist/` copy and SPA serving stay, but the Dioxus app must now produce `dist/`. Add a Dioxus build step before the server binary build, or document that CI builds `dist/` separately. Minimal change: replace the two trunk steps with a `dx build --release` step in `crates/app` (requires installing `dioxus-cli` in the builder stage — add `cargo install dioxus-cli --locked` alongside the wasm target).

- [ ] **Step 4: Update `.github/workflows/ci.yml`**

- In the `clippy` job: remove `--exclude scuffed-site` and `--exclude scuffed-admin` from the native step, and remove `-p scuffed-site -p scuffed-admin` from the WASM step (keep `scuffed-app`).
- In `build-and-test`: remove `--exclude scuffed-site --exclude scuffed-admin` from both the build and test commands.
- Replace the `wasm-build` (Trunk) job with a Dioxus build job, or delete it if the WASM clippy step provides sufficient coverage. Remove the `TRUNK_VERSION` env var.

- [ ] **Step 5: Verify scripts are valid**

Run: `bash -n scripts/build.sh && bash scripts/check-frontend-deps.sh`
Expected: no syntax error; guardrail prints "OK".

- [ ] **Step 6: Commit**

```bash
git add scripts/build.sh scripts/check-frontend-deps.sh Containerfile .github/workflows/ci.yml
git commit -m "chore: remove trunk/leptos build steps from scripts, Containerfile, CI"
```

---

## Task 11: Update CLAUDE.md (and note relay-policy as future work)

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Fix the crate map**

In the Architecture block: remove the `site/`, `admin/`, and `ui/` lines. Add the five currently-undocumented crates so the list matches the workspace: `chat/`, `stat-tracker/`, `map-pipeline/`, `map-renderer/`, `relay-policy/` (one-line descriptions each). Mark `relay-policy/` as "Nostr relay policy enforcement — standalone, not yet wired into deploy (future work)."

- [ ] **Step 2: Fix the Dev Mode section**

Remove the legacy admin trunk instructions ("Build admin: `cd crates/admin && trunk build --features dev-noauth`" and the `dev-noauth` bullet). Replace with the Dioxus app run path (`dx serve` in `crates/app`, plus the existing `/api/dev/login` flow which still applies).

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md crate map and dev-mode for post-Leptos layout"
```

---

## Task 12: Full-workspace verification

- [ ] **Step 1: Format**

Run: `cargo fmt --all`

- [ ] **Step 2: Clippy (native)**

Run: `cargo clippy --workspace --exclude scuffed-app --exclude scuffed-stat-tracker -- -D warnings`
Expected: clean.

- [ ] **Step 3: Clippy (wasm app)**

Run: `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings`
Expected: clean.

- [ ] **Step 4: Tests**

Run: `cargo test --workspace --exclude scuffed-app --exclude scuffed-stat-tracker`
Expected: pass.

- [ ] **Step 5: Final commit if fmt changed anything**

```bash
git add -A && git commit -m "style: cargo fmt after review fixes and leptos removal" || true
```
