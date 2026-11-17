# Browser checks: public pages (PR 696)

Playwright scripts for the redesigned public pages (Home, Blog, News, Wiki,
Events, Apply, patch notes, 404 and the no-access wall). They run against a
local stack with fake data only. They are not part of CI.

| Script | Checks |
| --- | --- |
| `fix1.mjs` | The review fixes: Apply rules and error copy (normal and forced colours), the pager, patch notes marks and headings, wiki save errors and encoded slugs, Home apply actions and hero fill, the Events RSVP radio group, the News title, the trial withdraw |
| `keys.mjs` | Keyboard pass: the `/blog` pager moves focus to the list under the top bar, page titles, the withdraw confirm (focus on Cancel, Esc and Cancel return focus), wiki History and Edit focus, the Apply error summary and its links |
| `shots.mjs` | Full-page screenshots and an in-page audit (contrast, overflow, console errors, failed requests) for guest, member, visitor and applicant at 1440 and 390, dark and light. Ends by running `summary.mjs` |
| `summary.mjs` | Groups `report-audit.json` failures by page, without the expected 401, 404, 500 and 502 answers |

`audit.mjs` is the in-page audit used by `fix1.mjs` and `shots.mjs`.
`crop.mjs SRC DST [HEIGHT]` crops the top of a full-page shot so it can be
viewed at full size.

`fix1.mjs` and `keys.mjs` print `PASS` and `FAIL` lines, write
`reports/report-<name>.json` and exit 1 on any failure. Both take an optional
filter: `node fix1.mjs apply` runs only the groups whose name contains
`apply`, `node shots.mjs guest-390` only the captures whose
`role-width-theme` key contains `guest-390`.

## 1. Start a local stack

Same stack as the shell checks: follow section 1 of `../README.md`. Use a
fresh, empty data folder for these checks; the two seeds are separate.

## 2. Install and seed

```bash
cd crates/app/browser-checks/s4a
npm ci
npx playwright install chromium   # once per machine
npm run seed                      # once per fresh database
```

`seed.mjs` creates TestAdmin (first-run setup, org "Test Crew"), Player01 and
Player02 (accepted members), Applicant01 (pending application) and Visitor01
(an account with no application, the only viewer who gets the Apply form),
all with the password `Test-password-12345`. It also adds two games, a long
blog post with an uploaded fake image, news, wiki pages, a patch note, Home
settings and one public event. `seed2.mjs` adds two teams with a roster and a
tournament in registration so Home shows its Teams table and Tournaments
panel. Both are safe to rerun. Seed output and saved sessions go to `.state/`
(gitignored).

The `withdraw` group in `fix1.mjs` withdraws Applicant01's application.
`npm run reapply` gives Applicant01 a pending application again; run it after
every `fix1` run, before `keys` (which checks the withdraw trigger) or another
`fix1`.

## 3. Run

```bash
npm run fix1
npm run reapply   # fix1 withdrew Applicant01; keys needs the pending application
npm run keys
npm run shots
```

Settings come from the environment:

| Variable | Default |
| --- | --- |
| `BASE` | `http://127.0.0.1:3031` |
| `STATE_DIR` | `./.state` |
| `REPORT_DIR` | `./reports` |
| `SHOTS_DIR` | `./shots` |
| `HEADED=1` | show the browser |

Chromium only. Forced-colour shots in `fix1.mjs` are the viewport, not the
full page: Playwright's stitched full-page capture does not paint forced
colours.

## 4. Clean up

Stop the server and SurrealDB, then delete the stack's data folder and
`.state/`, `reports/` and `shots/` here.
