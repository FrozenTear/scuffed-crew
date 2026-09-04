# Tag `stat-tracker-v0.4.6` (Robert authorized 2026-09-04)

Operator runbook; it is not the GitHub Release body. Robert authorized
the **full 0.4.6 cut**: bump, merge the release PR, annotated tag, and
`git push origin stat-tracker-v0.4.6`.

Proposed tag: **`stat-tracker-v0.4.6`**
Trigger: push of that tag runs `.github/workflows/stat-tracker-release.yml`
(builds daemon + Iced `stat-tracker-gui`, packages the tarball, creates the
GitHub Release).

This ships the feature PR already on `main`:

- #74 Maps-grammar Seasons grid + denser Settings cards (Companion hotkey
  setting unchanged)

Settings/Maps/Games polish (0.4.5), Companion overlay hotkey (0.4.4),
desktop-launcher absolute Exec (0.4.3), optional-tray (0.4.2), and
OpenSSL packaging (0.4.1) are unchanged. Daemon OCR / capture / sync /
store schema are unchanged.

Prefer **merge the release PR to `main`, then tag that merge commit**.
That keeps `CARGO_PKG_VERSION` on `main` aligned with the tag. Same
pattern as v0.4.5 (`docs/notes/stat-tracker-v0.4.5-tag.md`).

## 0. Confirm

- [x] Robert has said **yes** to tagging `stat-tracker-v0.4.6`
- [x] Robert has said **yes** to merging the release PR into `main`
- [ ] Release PR is against `main` and CI is green
- [ ] `git ls-remote --tags origin 'stat-tracker-v0.4.6'` is empty

## 1. Merge the release PR

Merge via GitHub when CI is green. Match #73 / #74: a merge
commit (`Merge pull request #N from FrozenTear/<branch>`), not squash/rebase.

```sh
git fetch origin main
git log -1 --oneline origin/main
# expect the merge commit of the v0.4.6 bump PR
```

## 2. Tag that commit

From a throwaway clone or worktree — do not move HEAD in the shared checkout.

```sh
git fetch origin main
SHA="$(git rev-parse origin/main)"
git show -s --oneline "$SHA"

# Annotated tag on origin/main. Do not push until this looks right.
git tag -a stat-tracker-v0.4.6 "$SHA" -m "Stat Tracker v0.4.6"
git show --no-patch stat-tracker-v0.4.6
```

Check that the tagged tree has `crates/stat-tracker` **0.4.6**:

```sh
git show stat-tracker-v0.4.6:crates/stat-tracker/Cargo.toml | head -6
# version = "0.4.6"
```

If `stat-tracker-v0.4.6` already exists locally or on `origin`, **stop**.
Do not force-push or retag.

## 3. Push the tag (triggers artifacts)

```sh
git push origin stat-tracker-v0.4.6
```

Watch:

- Actions: https://github.com/FrozenTear/scuffed-crew/actions/workflows/stat-tracker-release.yml
- Release: https://github.com/FrozenTear/scuffed-crew/releases/tag/stat-tracker-v0.4.6

The job injects `SST_RELEASE_VERSION=0.4.6` so
`scuffed-stat-tracker --version` is `scuffed-stat-tracker 0.4.6`.

Release notes are `crates/stat-tracker/CHANGELOG.md` section `## 0.4.6`, then
the usual git-log since the previous `stat-tracker-v*` tag (0.4.5), then
install requirements.

## 4. Laptop install test (AerynOS / Robert)

After the GitHub Release has the tarball:

```sh
STAT_TRACKER_TAG=stat-tracker-v0.4.6 \
  bash -c 'curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash'
scuffed-stat-tracker --version   # expect: scuffed-stat-tracker 0.4.6
# Seasons: Maps card grammar — 2–4 col grid, big WR, stripe, FillPortion bar.
# Settings: denser surface cards + 1–2 column field grid; Companion hotkey unchanged.
```

## Do not

- Do **not** use **workflow_dispatch** if you want the production tag name.
  Dispatch publishes a **draft** release tagged `stat-tracker-manual-<sha>`
  and can mint that tag itself (`--target`).
- Do **not** `gh release create` by hand — the workflow owns the tarball,
  sha256, and notes.
- Do **not** push `stat-tracker-v0.4.6` onto a commit that is not on `main`
  after the merge.
- Do **not** force-push or move the tag if the name already exists.

## If the tag was pushed by mistake

Human-only (agent-protocol §5): delete a published tag / GitHub Release only
with an explicit current instruction from Robert. Do not improvise.
