# G1 Version Integrity Implementation Plan

> **For Hermes:** subagent-driven-development.

**Goal:** Tagged releases produce a binary whose `--version` matches the tag (e.g. `stat-tracker-v0.1.2` → `scuffed-stat-tracker 0.1.2`), and CI enforces that. Local/dev builds still use `CARGO_PKG_VERSION`.

**Architecture:**
1. Compile-time override `SST_RELEASE_VERSION` (preferred over Cargo when set).
2. Release workflow exports `SST_RELEASE_VERSION` from the git tag before both cargo builds.
3. Clean-room smoke test requires exact version match (not just prefix).
4. Bump `crates/stat-tracker/Cargo.toml` to `0.1.2` so default matches latest published tag.
5. Optional tiny unit/doc test for version helper.

**Out of scope:** Bumping every workspace crate; monorepo site version; G3/DOC-1.

**Worktree:** `.claude/worktrees/grok-g1-version`  
**Branch:** `fix/g1-version-integrity`

---

### Task 1: Version helper in main.rs

**Files:** Modify `crates/stat-tracker/src/main.rs`

Add near top of main module (or just above main):

```rust
/// Runtime package version for --version/--help.
/// Release CI sets `SST_RELEASE_VERSION` from the git tag so the binary matches
/// the release (Cargo.toml can lag between tags). Local builds use CARGO_PKG_VERSION.
fn package_version() -> &'static str {
    option_env!("SST_RELEASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}
```

Replace both `env!("CARGO_PKG_VERSION")` in --version and --help with `package_version()`.

**Verify:** `rg package_version crates/stat-tracker/src/main.rs`  
**Commit:** `fix(stat-tracker): --version prefers SST_RELEASE_VERSION from release tags`

---

### Task 2: Bump Cargo.toml to 0.1.2

**Files:** `crates/stat-tracker/Cargo.toml` version = "0.1.2"

**Commit:** `chore(stat-tracker): bump package version to 0.1.2`

---

### Task 3: Release workflow inject + assert

**Files:** `.github/workflows/stat-tracker-release.yml`

Before each `cargo build` (daemon ~line 56 and gui ~line 113), add a step or prefix the build run with:

```bash
# Derive release version from tag (stat-tracker-v0.1.2 → 0.1.2)
if [[ "${GITHUB_REF_NAME}" == stat-tracker-v* ]]; then
  export SST_RELEASE_VERSION="${GITHUB_REF_NAME#stat-tracker-v}"
else
  export SST_RELEASE_VERSION="dev-${GITHUB_SHA::8}"
fi
echo "SST_RELEASE_VERSION=${SST_RELEASE_VERSION}"
echo "SST_RELEASE_VERSION=${SST_RELEASE_VERSION}" >> "$GITHUB_ENV"
cargo build ...
```

Update clean-room smoke (daemon) after --version:

```bash
out="$(./out/bin/scuffed-stat-tracker --version)"
echo "smoke: ${out}"
echo "${out}" | grep -qx "scuffed-stat-tracker ${SST_RELEASE_VERSION}" \
  || { echo "version mismatch: got '${out}', want scuffed-stat-tracker ${SST_RELEASE_VERSION}" >&2; exit 1; }
```

Ensure `SST_RELEASE_VERSION` is in env for smoke step (via GITHUB_ENV from build job — same job so OK).

For GUI job, set same env before build (consistency if GUI ever prints version).

**Commit:** `ci(stat-tracker): inject SST_RELEASE_VERSION from tag; assert --version`

---

### Task 4: Local compile check

```bash
cd worktree
cargo build -p scuffed-stat-tracker --bin scuffed-stat-tracker
./target/debug/scuffed-stat-tracker --version   # expect 0.1.2
SST_RELEASE_VERSION=9.9.9 cargo build -p scuffed-stat-tracker --bin scuffed-stat-tracker
# note: need rebuild with env - cargo clean -p or touch main.rs
```

Actually option_env is compile-time:  
`SST_RELEASE_VERSION=9.9.9 cargo build -p scuffed-stat-tracker && ./target/debug/scuffed-stat-tracker --version` → 9.9.9

**Commit** only if needed.

---

### Task 5: Push + fleet REVIEW

`git push -u origin fix/g1-version-integrity`  
Fleet REVIEW REQUEST G1 for Claude.

---

## Acceptance

- [ ] `--version` uses package_version()
- [ ] Cargo.toml 0.1.2
- [ ] Release workflow sets SST_RELEASE_VERSION from tag
- [ ] Smoke requires exact match
- [ ] Local build prints 0.1.2
- [ ] Override build prints override
- [ ] No unrelated files
