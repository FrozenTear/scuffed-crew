# Tag `stat-tracker-v0.4.13` (author tags after merge)

Operator runbook; it is not the GitHub Release body. This bump PR
covers #91 already on `main`. Author will tag
`stat-tracker-v0.4.13` after merge. Do not create the tag from this PR.
Merge + tag only if CI is fully green. Do not merge or tag if this
bump PR is not fully green.

Proposed tag: **`stat-tracker-v0.4.13`**
Trigger: push of that tag runs `.github/workflows/stat-tracker-release.yml`
(builds daemon + Iced `stat-tracker-gui`, packages the tarball, creates the
GitHub Release).

This ships the feature PR already on `main`:

- #91 bucket Neon Junction, Paraiso, and Esperanca by game mode so
  Maps leave OTHER. Neon Junction → Hybrid. Paraiso / Esperanca
  accept both plain and accented live names and classify as Hybrid /
  Push. Touched types, site stats maps, tracker parse, tracker-ui.

0.4.12 already shipped #87 (cheap outcome-only poll during
`end_reel_wake` while Tab is busy). Still on prior polish /
packaging from 0.4.1–0.4.12: In-app Update now (0.4.8), Settings
Maps-level polish (0.4.7), Maps-grammar Seasons grid (0.4.6),
Settings/Maps/Games polish (0.4.5), Companion overlay hotkey
(0.4.4), desktop-launcher absolute Exec (0.4.3), optional-tray
(0.4.2), and OpenSSL packaging (0.4.1) are unchanged. Daemon OCR /
capture / sync / store schema are unchanged.

Prefer **merge the release PR to `main`, then tag that merge commit**.
That keeps `CARGO_PKG_VERSION` on `main` aligned with the tag. Same
pattern as v0.4.12 (`docs/notes/stat-tracker-v0.4.12-tag.md`).

## 0. Confirm

- [ ] Author will tag `stat-tracker-v0.4.13` after this PR merges
- [ ] Author will merge the release PR into `main`
- [ ] Release PR is against `main` and CI is green
- [ ] `git ls-remote --tags origin 'stat-tracker-v0.4.13'` is empty

## 1. Merge the release PR

Merge via GitHub when CI is green. Match #80 / #81 / #82 / #83 / #84 / #85 / #86 / #87 / #88:
a merge commit (`Merge pull request #N from FrozenTear/<branch>`), not
squash/rebase.

```sh
git fetch origin main
git log -1 --oneline origin/main
# expect the merge commit of the v0.4.13 bump PR
```

## 2. Tag that commit

From a throwaway clone or worktree — do not move HEAD in the shared checkout.

```sh
git fetch origin main
SHA="$(git rev-parse origin/main)"
git show -s --oneline "$SHA"

# Annotated tag on origin/main. Do not push until this looks right.
git tag -a stat-tracker-v0.4.13 "$SHA" -m "Stat Tracker v0.4.13"
git show --no-patch stat-tracker-v0.4.13
```

Check that the tagged tree has `crates/stat-tracker` **0.4.13**:

```sh
git show stat-tracker-v0.4.13:crates/stat-tracker/Cargo.toml | head -6
# version = "0.4.13"
```

If `stat-tracker-v0.4.13` already exists locally or on `origin`, **stop**.
Do not force-push or retag.

## 3. Push the tag (triggers artifacts)

```sh
git push origin stat-tracker-v0.4.13
```

Watch:

- Actions: https://github.com/FrozenTear/scuffed-crew/actions/workflows/stat-tracker-release.yml
- Release: https://github.com/FrozenTear/scuffed-crew/releases/tag/stat-tracker-v0.4.13

The job injects `SST_RELEASE_VERSION=0.4.13` so
`scuffed-stat-tracker --version` is `scuffed-stat-tracker 0.4.13`.

Release notes are `crates/stat-tracker/CHANGELOG.md` section `## 0.4.13`, then
the usual git-log since the previous `stat-tracker-v*` tag (0.4.12), then
install requirements.

## 4. Laptop install test (AerynOS / the streamer)

After the GitHub Release has the tarball:

```sh
STAT_TRACKER_TAG=stat-tracker-v0.4.13 \
  bash -c 'curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash'
scuffed-stat-tracker --version   # expect: scuffed-stat-tracker 0.4.13
# Neon Junction buckets as Hybrid (not OTHER)
# Paraiso / Paraíso bucket as Hybrid; Esperanca / Esperança as Push
# Maps leave OTHER for those live names
```

## Do not

- Do **not** use **workflow_dispatch** if you want the production tag name.
  Dispatch publishes a **draft** release tagged `stat-tracker-manual-<sha>`
  and can mint that tag itself (`--target`).
- Do **not** `gh release create` by hand — the workflow owns the tarball,
  sha256, and notes.
- Do **not** push `stat-tracker-v0.4.13` onto a commit that is not on `main`
  after the merge.
- Do **not** force-push or move the tag if the name already exists.
- Do **not** merge or tag unless this bump PR's CI is fully green.
- Do **not** create the tag from the bump PR — author tags after merge.

## If the tag was pushed by mistake

Human-only (agent-protocol §5): delete a published tag / GitHub Release only
with an explicit current instruction from the repo owner. Do not improvise.
