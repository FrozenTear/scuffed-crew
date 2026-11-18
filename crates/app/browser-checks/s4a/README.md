# Browser checks: public pages (PR 696)

Playwright scripts for the redesigned public pages (Home, Blog, News, Wiki,
Events, Apply, patch notes, 404 and the no-access wall). They run against a
local stack with fake data only. They are not part of CI.

| Script | Checks |
| --- | --- |
| `fix1.mjs` | The review fixes: Apply rules and error copy (normal and forced colours), the pager, patch notes marks and headings, wiki save errors and encoded slugs, Home apply actions and hero fill, the Events RSVP radio group, the News title, the trial withdraw |
| `keys.mjs` | Keyboard pass: the `/blog` pager moves focus to the list under the top bar, page titles, the withdraw confirm (focus on Cancel, Esc and Cancel return focus), wiki History and Edit focus, the Apply error summary and its links |
| `bars.mjs` | Keyboard focus clear of the shell bars (QA 696 nit 1): Tab and Shift+Tab through `/`, `/blog`, `/news`, `/wiki/tank-basics`, `/patch-notes` and `/changelog/site` at 1440x900, 390x844 and 360x844; fails on any stop in `main` or the footer that is not fully between the top bar and the phone tab bar (or the bottom of the view). Runs in Chromium and Firefox (`BROWSER=firefox`); `GUARD_OFF=1` blocks the shell's focus guard to show the same bundle failing without it |
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
panel. Both are safe to rerun. Right after the S2 seed the server
rate-limits sign-ins: every sign-in, setup and register call waits out a
429 (the server's `retry_after`, at most 10 s, up to 3 times). If a 401 or
429 is still left at the end, most writes were refused, so `seed.mjs`
prints `SEED FAILED` and exits 1: start again from an empty database. Seed output and saved sessions go to `.state/`
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
npm run bars                      # Chromium
BROWSER=firefox npm run bars      # Firefox (npx playwright install firefox once)
```

Settings come from the environment:

| Variable | Default |
| --- | --- |
| `BASE` | `http://127.0.0.1:3031` (loopback hosts only) |
| `ALLOW_REMOTE=1` | accept a `BASE` that is not loopback |
| `STATE_DIR` | `./.state` |
| `REPORT_DIR` | `./reports` |
| `SHOTS_DIR` | `./shots` |
| `HEADED=1` | show the browser |
| `BROWSER` | `chromium`, or `firefox` for `bars.mjs` |

The checks seed accounts, delete events and rewrite site settings, so every
script refuses a `BASE` whose host is not `localhost`, `127.x.x.x` or `[::1]`.
Set `ALLOW_REMOTE=1` only for a throwaway stack on another machine.
`npm test` checks that guard (no browser or stack needed).

Chromium only, apart from `bars.mjs`. Forced-colour shots in `fix1.mjs` are the viewport, not the
full page: Playwright's stitched full-page capture does not paint forced
colours.

## 4. Clean up

Stop the server and SurrealDB, then delete the stack's data folder and
`.state/`, `reports/` and `shots/` here.
