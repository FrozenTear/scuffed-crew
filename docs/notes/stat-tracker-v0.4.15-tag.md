# Tag `stat-tracker-v0.4.15` (author tags after merge)

Operator runbook; it is not the GitHub Release body. This bump PR
covers work already on `main` (#122, #125, #126). HOLD-MERGE: do not
merge this PR, and do not create the tag from it. Tag
`stat-tracker-v0.4.15` only after a human merges it to `main` and CI
is green. Do not merge or tag if this bump PR is not fully green.

Proposed tag: **`stat-tracker-v0.4.15`**
Trigger: push of that tag runs `.github/workflows/stat-tracker-release.yml`
(builds daemon + Iced `stat-tracker-gui`, packages the tarball, creates the
GitHub Release).

This ships the feature PRs already on `main`:

- #122 Overview header: filter chips vs companion switch vs plain status (L25).
- #125 updater and Settings install command pinned to the release tag,
  optional minisign verification, failed install leaves daemon stopped,
  atomic binary replace, tar path/symlink hardening (M19).
- #126 sync token refused over non-loopback http, sync backoff,
  Retry-After on 429/503 (M20).

Prefer **merge the release PR to `main`, then tag that merge commit**.
That keeps `CARGO_PKG_VERSION` on `main` aligned with the tag. Same
pattern as v0.4.14 (`docs/notes/stat-tracker-v0.4.14-tag.md`).

## 0. Confirm

- [ ] Hold is lifted; a human will merge the release PR into `main`
- [ ] Author will tag `stat-tracker-v0.4.15` after this PR merges
- [ ] Release PR is against `main` and CI is green
- [ ] `git ls-remote --tags origin 'stat-tracker-v0.4.15'` is empty

## 1. Merge the release PR

Merge via GitHub when CI is green and the hold is lifted. Match #80 / #81 / #82 / #83 / #84 / #85 / #86 / #87 / #88 / #93:
a merge commit (`Merge pull request #N from FrozenTear/<branch>`), not
squash/rebase.

```sh
git fetch origin main
git log -1 --oneline origin/main
# expect the merge commit of the v0.4.15 bump PR
```

## 2. Tag that commit

From a throwaway clone or worktree — do not move HEAD in the shared checkout.

```sh
git fetch origin main
SHA="$(git rev-parse origin/main)"
git show -s --oneline "$SHA"

# Annotated tag on origin/main. Do not push until this looks right.
git tag -a stat-tracker-v0.4.15 "$SHA" -m "Stat Tracker v0.4.15"
git show --no-patch stat-tracker-v0.4.15
```

Check that the tagged tree has `crates/stat-tracker` **0.4.15**:

```sh
git show stat-tracker-v0.4.15:crates/stat-tracker/Cargo.toml | head -6
# version = "0.4.15"
```

If `stat-tracker-v0.4.15` already exists locally or on `origin`, **stop**.
Do not force-push or retag.

## 3. Push the tag (triggers artifacts)

```sh
git push origin stat-tracker-v0.4.15
```

Watch:

- Actions: https://github.com/FrozenTear/scuffed-crew/actions/workflows/stat-tracker-release.yml
- Release: https://github.com/FrozenTear/scuffed-crew/releases/tag/stat-tracker-v0.4.15

The job injects `SST_RELEASE_VERSION=0.4.15` so
`scuffed-stat-tracker --version` is `scuffed-stat-tracker 0.4.15`.

Release notes are `crates/stat-tracker/CHANGELOG.md` section `## 0.4.15`, then
the usual git-log since the previous `stat-tracker-v*` tag (0.4.14), then
install requirements.

## 4. Laptop install test (AerynOS)

After the GitHub Release has the tarball:

```sh
STAT_TRACKER_TAG=stat-tracker-v0.4.15 \
  bash -c 'curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash'
scuffed-stat-tracker --version   # expect: scuffed-stat-tracker 0.4.15
```

## Do not

- Do **not** use **workflow_dispatch** if you want the production tag name.
  Dispatch publishes a **draft** release tagged `stat-tracker-manual-<sha>`
  and can mint that tag itself (`--target`).
- Do **not** `gh release create` by hand — the workflow owns the tarball,
  sha256, and notes.
- Do **not** push `stat-tracker-v0.4.15` onto a commit that is not on `main`
  after the merge.
- Do **not** force-push or move the tag if the name already exists.
- Do **not** merge or tag unless this bump PR's CI is fully green.
- Do **not** create the tag from the bump PR — author tags after merge.
- Do **not** merge while this PR is HOLD-MERGE.
- Do **not** retag older `stat-tracker-v*` tags.
- Do **not** touch Contabo, Site/API deploy, or `scripts/update.sh`.

## If the tag was pushed by mistake

Human-only (agent-protocol §5): delete a published tag / GitHub Release only
with an explicit current instruction from the repo owner. Do not improvise.
