# Tag `stat-tracker-v0.4.0` (after Robert confirms)

**Do not create or push this tag until Robert confirms.** Never merge the
release PR without Robert. This file is the operator runbook; it is not the
GitHub Release body.

Proposed tag: **`stat-tracker-v0.4.0`**
Trigger: push of that tag runs `.github/workflows/stat-tracker-release.yml`
(builds daemon + Iced `stat-tracker-gui`, packages the tarball, creates the
GitHub Release).

Prefer **merge the release PR to `main`, then tag that merge commit**. That
keeps `CARGO_PKG_VERSION` on `main` aligned with the tag. The v0.3.4 cut tagged
the bump commit first and left `main` at 0.3.3 (PR #46 still open) — do not
repeat that.

## 0. Confirm

- [ ] Robert has said **yes** to tagging `stat-tracker-v0.4.0`
- [ ] Robert has said **yes** to merging the release PR into `main`
- [ ] Release PR is against `main` and CI is green
- [ ] `git tag --list 'stat-tracker-v0.4.0'` is empty on `origin`

## 1. Merge the release PR (Robert)

Merge via GitHub UI (or instruct an agent). Author / preparing agent does
**not** merge.

```sh
git fetch origin main
git log -1 --oneline origin/main
# expect the merge (or squash) commit of the v0.4.0 release PR
```

## 2. Tag that commit (only after confirmation)

From a throwaway clone or worktree — do not move HEAD in the shared checkout.

```sh
git fetch origin main
SHA="$(git rev-parse origin/main)"
git show -s --oneline "$SHA"

# Annotated tag on origin/main. Do not push until this looks right.
git tag -a stat-tracker-v0.4.0 "$SHA" -m "Stat Tracker v0.4.0"
git show --no-patch stat-tracker-v0.4.0
```

Check that the tagged tree has `crates/stat-tracker` **0.4.0**:

```sh
git show stat-tracker-v0.4.0:crates/stat-tracker/Cargo.toml | head -6
# version = "0.4.0"
```

## 3. Push the tag (triggers artifacts)

```sh
git push origin stat-tracker-v0.4.0
```

Watch:

- Actions: https://github.com/FrozenTear/scuffed-crew/actions/workflows/stat-tracker-release.yml
- Release: https://github.com/FrozenTear/scuffed-crew/releases/tag/stat-tracker-v0.4.0

The job injects `SST_RELEASE_VERSION=0.4.0` so
`scuffed-stat-tracker --version` is `scuffed-stat-tracker 0.4.0` even if
someone tags a tree whose Cargo.toml lagged (it should not on this cut).

Release notes are `crates/stat-tracker/CHANGELOG.md` section `## 0.4.0`, then
the usual git-log since the previous `stat-tracker-v*` tag, then install
requirements.

## 4. Laptop install test

After the GitHub Release has the tarball:

```sh
STAT_TRACKER_TAG=stat-tracker-v0.4.0 \
  bash -c 'curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash'
stat-tracker-gui --help
scuffed-stat-tracker --version   # expect: scuffed-stat-tracker 0.4.0
```

Reinstall replaces Dioxus `stat-tracker-gui` in place (same binary name).

## Do not

- Do **not** use **workflow_dispatch** if you want the production tag name.
  Dispatch publishes a **draft** release tagged `stat-tracker-manual-<sha>`
  and can mint that tag itself (`--target`).
- Do **not** `gh release create` by hand — the workflow owns the tarball,
  sha256, and notes.
- Do **not** push `stat-tracker-v0.4.0` onto a commit that is not on `main`
  after the merge.

## If the tag was pushed by mistake

Human-only (agent-protocol §5): delete a published tag / GitHub Release only
with an explicit current instruction from Robert. Do not improvise.
