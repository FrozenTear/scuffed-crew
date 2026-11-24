# Scuffed Tracker 0.7 upgrade plan

Plan only. No code and no PRs come with this document. It is for Chief and FrozenTear.

- Written: Mon 23 Nov 2026 (Europe/Oslo).
- Base: `wip/0.7` at `632a17ad`, cut from the `stat-tracker-v0.6.0` tag commit `ce3ee9c7`, plus #886 and #885. Every path, name and line number below was checked at that SHA.
- Tracking issue: #881. Owner: Scuffed Tracker.
- Rules that do not change: small draft HOLD-MERGE PRs against `wip/0.7` (0.7.1 work on its own branch), a stamp from API or Site at the exact head SHA (Vision does no PR reviews), QA last, and FrozenTear's go for every merge, tag, deploy and AUR push.

## Summary and framing

This is an upgrade of the tracker app that shipped as 0.6.0. The screens, flows and patterns that work today stay. Each item below is a small change on top of the current UI, in its own small PR.

**0.7.0.** Freeze Thu 3 Dec, bump Fri 4 Dec, tag Sun 6 or Mon 7 Dec. Hard deadline Tue 8 Dec 20:00 Oslo (19:00 UTC), when Season 6 starts with 6v6 role queue.

- 6v6 role queue (2-2-2) in the tracker: queue detection and per-role rank (#654, tracker parts).
- Status pill in the top bar with Start, Restart and Stop (#775), then the pinned Settings bar (#776). The status wording part of #778 rides with them.
- Win rate per banned hero, as a When banned mode on the Heroes tab (#779).
- Rank as text in the Games table's Rank column (the "Rank text column" line on #881).
- Save corrections keeps the scroll position (#806).
- Report kit fixes: no orphan kits (#810) and no silent returns (#811).
- What leaves your PC: line ends no longer hidden under the scrollbar (#879).

**0.7.1.** The companion pages (companion-spec.md) and the tracker parts they need, built side by side on their own branch and merged only when they pass QA as a pair. Combined QA Wed 9 to Sat 12 Dec, tag about Mon 14 to Wed 16 Dec. The one switch control (#777) and the rest of #778 go here if they do not fit before the freeze.

**What stays as it is**

- The top tabs: Overview, Games, Heroes, Maps, Seasons, Settings. No new tab. The classic layout (`SCUFFED_TRACKER_LAYOUT=classic`) stays as a fallback.
- The Settings page and its instant writes ("Changes apply as you make them, except those tagged After restart."). No Save button comes back for switches.
- How the app starts and stops the tracker: `daemon::run_verb`, the systemd user unit, and the pid identity checks (`crates/stat-tracker-ui/src/daemon.rs`).
- The Games table, the open game panel (Edit, the map and hero picker, Delete, Retry upload) and the bans and picks chips. UX Spec's a3b pass kept the chips as the default, which is FrozenTear's choice.
- Heroes Cards and List, the Maps table with its Best and Worst cards, and the Seasons cards.
- The 10 decided games rule (`RATE_MIN` in `crates/stat-tracker/src/games/small_sample.rs`).
- The companion card: 360 px wide, click-through, top right, Super+Shift+C to hide it for a game. In 0.7.1 it becomes page 1, Tonight.
- The tray menu: Show window, Hide window, Hide / show overlay, Quit.

**Three findings that set the order**

1. **Stat Tracker CI does not run on `wip/0.7`.** `.github/workflows/stat-tracker.yml` lists `main` and `wip/0.6` for both `push` and `pull_request`. #886 added `wip/0.7` to `ci.yml` only, and `ci.yml` skips its site jobs when every changed path is tracker-only. So a tracker PR into `wip/0.7` gets no tracker build, clippy or tests today. PR T1 fixes this before anything else.
2. **The file-size guard leaves no room in `app.rs`.** `crates/stat-tracker/src/tests_file_size.rs` pins `crates/stat-tracker-ui/src/app.rs` at exactly 2911 lines, its length today. It may not grow, and a shrink must lower its `CAPS` entry in the same PR. Several other files the plan touches sit just under the 1500 line limit (see Rules for every PR). New code goes in new modules.
3. **#806 is very likely not a Games bug.** `chrome::top_tabs` (`crates/stat-tracker-ui/src/chrome.rs`, lines 118 to 121) returns the window column alone, or a stack of the window and the floating toast when a toast shows. iced 0.14 matches widget state by position in the tree (`iced_core` `Tree::diff`), so when the root changes shape the page scrollable very likely gets a fresh state and jumps to the top. If so, every toast on every tab does this, and so does a toast closing. Save corrections always shows a toast, on success and on a refused save, which fits what QA saw. The fix belongs in the chrome, and it should land before the pill and the bar add layers of their own. T4's first test confirms the cause.

A note on the toolkit: the tracker app (`crates/stat-tracker-ui`) is Iced 0.14, with `iced_layershell` 0.19 for the companion and `tray-icon` 0.23 for the tray. The Dioxus app is the website (`crates/app`).

## The app today

### Processes and how they talk

| Part | Binary or entry | Code | What it does |
| --- | --- | --- | --- |
| Tracker (daemon) | `scuffed-stat-tracker` | `crates/stat-tracker/src/main.rs`, `crates/stat-tracker/src/daemon/` | Captures, reads boards and screens, stores games in the local SurrealKV store, uploads. Writes the status file (`stat_tracker_core::ocr_status::Status`), `live_snapshot.json` (or answers on the query socket, `daemon/query_server.rs`) and `daemon.pid`. |
| Desktop app | `stat-tracker-gui` | `crates/stat-tracker-ui/src/main.rs` (`iced::daemon`), `crates/stat-tracker-ui/src/app.rs` (`TrackerApp`, `Message`) | One per session (`instance.rs`). Reads the status file and the games once a second (`Message::Tick`). Writes `config.toml` (`settings_save.rs`) and `StoreCommand` files under `<data_dir>/commands/` (`commands.rs`). |
| Companion | `stat-tracker-gui --companion` | `crates/stat-tracker-ui/src/overlay.rs`, `crates/stat-tracker-ui/src/overlay_shell.rs` | A child process of the desktop app, started and stopped by `overlay::reconcile_companion`. A layer-shell surface (`Layer::Overlay`, anchored top right, `KeyboardInteractivity::None`). Re-reads `ui_state.json`, the shortcut and the snapshot on a 1 s tick. |
| Shared bottom layer | library | `crates/stat-tracker-core/src/` | Config, shared model (`model.rs`), the status file (`ocr_status.rs`), process helpers (`replace.rs`). |

### Window chrome

- `chrome.rs`: `with_layers` stacks open dialogs over the window (What leaves your PC is the last layer). `top_tabs` draws the bar, the page title, `page_body` and the floating prompts. `floating_prompts` holds the toast and the app restart bar, bottom centre, at most 640 px (`FLOAT_MAX`). `page_body` wraps every top tabs screen, Settings included, in one scrollable with the id `focus_status::PANE_SCROLL_ID`.
- `nav.rs`: `top_bar`, `status_slot` (dot and words, or the amber focus pill that opens Settings through `nav::Msg::ShowFocusSetting`), `needs_look_chip`, `tab_strip`. `nav_fit.rs` measures the bar with the bundled Urbanist widths and picks the fullest form that fits. From the 720 px minimum to about 860 px the bar takes two lines.
- `tracker_status.rs`: `HeaderStatus` (Capturing, CapturingAnyWindow, Paused, FocusUnknown, Waiting, Down, TokenRejected), `short_label`, `long_label`, and `strip_label` ("Tracker running" or "Tracker stopped" on the Settings strip).
- Window size: 1280x860 preferred, 720x480 minimum (`layout.rs`: `WINDOW_PREF_W`, `WINDOW_PREF_H`, `WINDOW_MIN_W`, `WINDOW_MIN_H`).

### Screens

| Screen | Entry | Main files (all under `crates/stat-tracker-ui/src/`) | What it shows today |
| --- | --- | --- | --- |
| Overview | `overview::view` | `overview_page.rs`, `overview_main.rs`, `overview_side.rs`, `overview_heroes.rs`, `overview_chips.rs`, `stat_tiles.rs` | Session card, newest game with six tiles, heroes with 3 or more games, side panel (season win rate, last 20 games, needs a look, and a Companion card with a switch: an iced `toggler` in `overview_side.rs` `companion_card`). |
| Games | `games::view`, then `games_table::view` | `games_table.rs`, `games_rows.rs`, `games_sessions.rs`, `games_panel.rs`, `games_detail.rs`, `games_picker.rs`, `games_delete.rs`, `games_retry.rs`, `games_bans.rs`, `games_bans_view.rs`, `commands.rs` | Table by session or flat, with filter chips. The RANK and CREW columns draw "-" (`games_table.rs` line 718, `Col::Rank \| Col::Crew`), and `SORT_NOTE` in `games_rows.rs` says "Rank and crew are not recorded yet." The queue chip comes from `games_detail::queue_value`. The open row has map, heroes, result, time, six tiles, the upload box, Edit, Report misread, Scoreboard picture, Delete game, and the bans and picks chips. |
| Heroes | `heroes::view`, then `heroes_page::view` | `heroes_page.rs` | Cards or List (`HeroView`), sorted by Games, Win rate or Recent (`HeroSort`), with season, role and map filters (`heroes_page::stats_filter`). |
| Maps | `maps_tab::view` | `maps_tab.rs` | Table with sortable headers, Best and Worst cards, and a not counted card. |
| Seasons | `seasons_tab::view` | `seasons_tab.rs`, `seasons.rs` | Current season card and a table of seasons. |
| Settings | `settings_page::view` | `settings_page.rs`, `settings_cards.rs`, `settings_keys.rs`, `capture_reading.rs`, `settings_rows.rs`, `settings_model.rs`, `settings_update.rs`, `settings_save.rs`, `settings_open.rs`, `account_card.rs`, `training_card.rs` | The status strip (status words, Version, Last capture, the changes waiting count, Start tracker or Restart tracker), "Run setup again" with the legend, the Everyday cards (Player and sessions, Companion, Website account, Updates) and the Advanced rows (Capture and reading, Tracker service, Stored data, Website connection, Troubleshooting). |

### Dialogs, companion and tray

| Part | Files (under `crates/stat-tracker-ui/src/`) | Today |
| --- | --- | --- |
| What leaves your PC | `what_leaves.rs` (`layer`, `dialog`, `SECTIONS`) | Opened from Website account. A 560 px card with a scrolled body that leaves no room for the scrollbar, so line ends hide under it (#879). |
| Report a problem, Report misread | `report.rs`, `report_choices.rs`, `report_rank.rs` | Scrolled body in a dialog. |
| Release notes and About | `notes.rs` | Scrolled body padded 8 px on the right, under iced's default 10 px scrollbar, which floats over the content. |
| Uninstall | `uninstall.rs` | Scrolled body in a dialog. |
| Setup guide | `setup_guide.rs` | Opened by "Run setup again" on Settings. |
| Update banner, app restart bar | `update.rs`, `restart_prompt.rs` | The restart bar floats with the toast. |
| Companion overlay | `overlay.rs` (`build_model`, `view`, `last_game_card`, `tonight_strip`, `heroes_block`, `footer_row`), `overlay_shell.rs`, `companion_gate.rs`, `hotkey.rs` | One card: season line, last game, Tonight results as coloured squares with no letters, top heroes tonight, footer. Super+Shift+C (read from `/dev/input` with evdev, so it needs the input group) or the tray hides it for the game. On X11 the child exits at once, the app shows the toast "Companion overlay needs a Wayland compositor with layer-shell" and stops respawning it. |
| Tray | `tray.rs` | Show window, Hide window, Hide / show overlay, Quit. Needs an AppIndicator library; without it the app runs with no tray and says so in a toast. The desktop also needs a tray host to show the icon. |

### Flows today

1. **Start, Stop, Restart.** Start tracker or Restart tracker is the button on the Settings strip (`settings_page::strip_actions`). Stop is only inside Advanced, Tracker service (`settings_cards::tracker_service`). Both send `Message::Daemon(verb)`. In `app.rs` (line 1134) that sets `daemon_busy`, a plain bool, and runs `daemon::run_verb` (`daemon.rs` line 592). `Message::DaemonDone` clears it and shows a toast. There is no confirm, even mid-game: `stop_running` says an explicit Stop does not wait for a game. The top bar does not change while busy. A stop can take up to 90 s while the tracker drains its Tab reads (`daemon::STOP_WAIT_TIMEOUT`). Only the update path (`Message::RestartGui`, `app.rs` line 1607) waits for a game to be saved (`stat_tracker::replace::game_file_in_progress`). `Message::ToggleAutostart` reuses `daemon_busy` and `DaemonDone`.
2. **Settings apply.** Switches, chips, dropdowns and radios write `config.toml` at once (`settings_update::Msg::Apply` with `settings_save::Commit::Control`). Typed text waits for Enter or Apply (`Commit::Timings`, `Commit::Sync`, `Commit::Shortcut`). Saved keys the running tracker has not loaded are counted by `TrackerApp::changes_waiting` (`settings_update.rs`) and shown on the strip next to Restart tracker. Leaving Settings with typed drafts drops nothing and asks nothing.
3. **Game correction save.** Games, open a game, Edit, Save corrections. `Message::SaveEdit` (`app.rs` line 987) calls `commands::save_edit_for`, then `commands::show_saved` (a toast such as "Stats corrected" or "Elims confirmed") and `refresh_game`. A refused save toasts the reason ("No changes to save") and keeps the form open. Either way the page jumps to the top (#806).
4. **Report kit.** Each accepted Tab calls `record_report_kit` (`crates/stat-tracker/src/daemon/debug_dumps.rs` line 157) from `daemon/tab/grab.rs` (line 1355). It returns with no log when `report::blank::TeamLayout::from_team_size` refuses the size, and logs a write failure at debug level only (#811). Rank screen crops are written by `report::kit::record_rank_screen_pngs` (`report/kit/rank.rs` line 90) from `daemon/event_loop.rs` (lines 793 to 802) whenever the active game was active within `RANK_OUTCOME_MAX_IDLE` (10 minutes), even when that game has no stored board (#810).
5. **Queue and rank.** With Read my rank after Comp games on (off by default), the queue title reader reads OPEN QUEUE or ROLE QUEUE on the pre-game screens (`crates/stat-tracker/src/shadow/queue_title/`, `daemon/queue_title.rs`, stored by `storage/queue_title.rs` `attach_queue_title`), and the rank reader reads the rank screen (`shadow/rank/`, `daemon/rank.rs`, stored as `PersonalMatch::rank_update`). The upload sends both in the `rank` object (`sync/wire.rs` `upload_rank`, site type `scuffed_types::api::MatchRank`). `daemon::rank::queue_hint` drops a `5V5` or `6V6` word after `COMPETITIVE`, so the rank banner hint for the next game says only ROLE QUEUE or OPEN QUEUE, and the queue title pack knows only those two titles. Its `queue_key` (`stat_tracker_core::model::queue_key`) keeps every word but the last QUEUE, so the same title in another word order gives another key. The board reader knows the team size (5 or 6 per team), but the game record does not store it. So nothing on a game tells 6v6 role queue from 5v5 role queue, and its rank would be filed with the 5v5 role ranks.
6. **Companion.** Shows during a game when Show Companion during games is on (`companion_gate.rs`). One card, no pages. The shortcut and the tray item hide it for one game.

## Screens: what stays and what changes

| Screen | Stays as is | 0.7.0 change | 0.7.1 change |
| --- | --- | --- | --- |
| Top bar | Brand, tabs, the needs a look chip, the focus pill behaviour, the `nav_fit` breakpoints | The status slot becomes a pill with a menu: Start, Restart, Stop. It says Starting..., Restarting... or Stopping... while busy, keeps an Error state with the reason, a Troubleshooting link and Retry, and asks to confirm Restart and Stop only mid-game (#775). | None planned. |
| Overview | Everything | None. | The switch in the side panel's Companion card takes the new switch look only if Chief says so (Q7 under Overlap with Site / UX Spec). |
| Games table | Everything | The page keeps its scroll after a save and after any toast (#806). The Rank column shows the rank as text. The queue chip names 6v6 role queue (#654). `SORT_NOTE` no longer says rank is not recorded. | None. |
| Games open row | Everything, including the a3b bans and picks chips | Only the queue chip and the "queue not known" warning wording (#654). No rank line in the panel unless Chief asks (Q11). | None. |
| Heroes | Cards, List, sort, filters, the Other heroes fold | A Played or When banned segment on the heading. When banned shows the per banned hero table from UX Spec's 779 spec (#779). | None. The companion reuses the same counts. |
| Maps | Everything | None. | None. |
| Seasons | Everything | None. | None. |
| Settings | Two columns, Everyday cards, Advanced rows, instant writes, the legend | A pinned bar at the bottom for typed drafts and restart-needed changes (#776). The status strip goes once the pill and the bar ship. One set of status words (no "Tracker running" for an idle tracker). If it fits: the Wayland notice moves into the Companion card, Run setup again moves to Troubleshooting, chevron icons on closed Advanced rows, one-line Capture and reading summary (#778). | One switch control for every on/off setting, the Companion shortcut included (#777). Companion card: page keys and a "Hold the stream page" switch. |
| What leaves your PC | Copy and layout | Text clear of the scrollbar (#879). A line for the queue value only if API adds an upload field (Q4). | None. The companion sends nothing. |
| Other dialogs | Everything | The same scrollbar check as #879 on Report a problem, Uninstall, release notes and the setup guide. | None. |
| Companion overlay | Size, position, click-through, hide key, Tonight content | None. | Pages: Tonight (letters W, L or D inside the result squares), Map vote, Picks and bans with the ban hint card, Matchup, Tip. Page dots with the page name in words, auto-advance and auto-return, a safe page key. |
| Tray | Everything | None. | None. |

## Flows after the upgrade

### Start, Stop and Restart (0.7.0, #775)

1. The pill shows the state in words, with the same breakpoints as today. Words per UX Spec's spec 1.2: Stopped / Tracker stopped, Starting..., Ready / Tracker ready, In game / Watching Overwatch, In game / Capturing in any window, Paused, the focus unknown pill, Restarting..., Stopping..., Couldn't start / Couldn't start the tracker, Token rejected / Website token rejected.
2. Click, Space or Enter opens the menu. Stopped: Start tracker. Error: the reason, a Troubleshooting link (`nav::Msg::ShowAdvancedSection(AdvancedSection::Troubleshooting)`), then Retry. Any running state: Restart tracker and Stop tracker. While a verb runs there is no menu.
3. Restart or Stop while a match is in progress asks first: "Stop the tracker?" or "Restart the tracker?", body "The current game won't be recorded.", with Keep tracking as the default. "In progress" means `stat_tracker::replace::game_file_in_progress` or a present `live_game_session`, not just Overwatch being open. No confirm when idle, for Start or Retry, or in demo mode.
4. If the pinned bar holds typed drafts, the verbs follow spec 1.5: Start, Retry and Restart save first; Stop asks "Save changes first?" with Save and stop, Discard and stop, and Cancel (the default).
5. The app records the verb in flight and runs `daemon::run_verb` as today. The pill says Starting..., Restarting... or Stopping... next to an amber dot. A stop can take up to 90 s; clicks are ignored while busy, as today.
6. On `DaemonDone` a success refreshes the view and shows today's toast. A failure keeps the Error state until the next verb, with the error text from `run_verb` (a failed direct start already returns the last error line of the tracker log, `start_daemon_checked`).
7. The focus unknown pill still opens Settings at the focus row. A chevron part of it opens the menu.
8. `Message::ToggleAutostart` and `Message::RestartGui` keep a busy mark of their own, so the pill only names the verbs it ran, and no verb starts while either runs.

### Settings save and restart needed (0.7.0, #776, with the wording part of #778)

1. The bar shows only on Settings and only when something is pending. It counts typed draft groups (timings as one, sync as one, shortcut as one) plus `changes_waiting`. Copy per spec 2.2, for example "1 unsaved change", "3 changes waiting", "2 unsaved changes, 1 waiting for restart".
2. Discard (drafts only) puts the typed fields back to the saved values. It never undoes a switch.
3. Save commits through the same paths as Enter and Apply (`Commit::Timings`, `Commit::Sync`, `Commit::Shortcut`) with the same checks. An unsafe website URL blocks it. Errors stay on the bar: `SETTINGS_SAVE_REFUSED`, `PAGE_RELOADED_FROM_DISK` or the URL problem line.
4. Save and restart (or Restart tracker, or Save and start when the tracker is stopped): commit, then the mid-game confirm from the Start, Stop and Restart flow (step 3), then `run_verb`. The waiting count clears through `PageState::restart_started`, `daemon_done` and `observe_tracker`, as today.
5. Leaving Settings with drafts (another tab, a dialog that covers Settings, Hide window, Quit) asks "You have unsaved changes" with Discard and leave, Save (or Save and restart), and Cancel as the default. Saved restart-needed changes never block leaving.
6. Switches stay instant (spec 2.1). Switch-spec's "flipping marks the form dirty" rule is an open question (Q7).
7. When the pill and the bar have both shipped, the status strip goes. Version stays on the Updates card and in About. The Tracker service row uses the pill's words, not "Tracker running".

### Game correction save (0.7.0, #806)

1. Edit, change fields, Save corrections: the same save path and the same toasts as today.
2. The page keeps its scroll offset. The open game and its form stay where they were. After a refused save, the form stays open in the same place.
3. The same holds for every other toast (Map saved, Hero saved, Upload queued again, Game deleted), on every tab, and when a toast closes.

### Report kit (0.7.0, #810 and #811)

1. An accepted Tab writes the kit as today. If `TeamLayout::from_team_size` refuses the team size, the tracker logs a warning with the size and still writes `reads.json` (with an unknown layout). Every kit write failure logs a warning with the session id. The game record is never touched by a kit error, as today.
2. Rank and outcome crops are written only to a game that has a stored board, or to a start screen game still waiting for its first board. Otherwise the tracker logs that it left them out.
3. A rank or end screen alone does not open a fresh session. This step changes session rules, so it lands only with a replay test of the 4f17bcff orphan timeline that fails before the change.

### Role queue detection and per-role rank (0.7.0, #654 tracker parts)

1. Inputs per game: the queue title (when its reader runs), the team size from the board (5 or 6 per team), and the rank screen's role line (Tank, Damage, Support, or Open when no role line shows).
2. A local queue value per game, from these rules. The names here are working names only; API owns the wire names and values (Q4).
   - Title names ROLE and the board has 6 per team: role queue 6v6. With 5 per team: role queue 5v5. `6V6 ROLE QUEUE`, `ROLE QUEUE 6V6` and `6v6 Role Queue` all count, so no exact string is needed.
   - Title names OPEN and the board has 6 per team: open queue 6v6. With 5 per team: open queue 5v5.
   - No confident title and 6 per team: a rank screen role line of Tank, Damage or Support means role queue 6v6, and Open means open queue 6v6.
   - Anything else: queue unknown. The game is recorded as normal and shows the existing warning (`QUEUE_TITLE_WARNING`). A game is never dropped.
3. The value is stored once per game as a new optional field, the same way `storage/queue_title.rs` stores the queue title. It goes into `matches.jsonl` too.
4. Per-role rank: the rank stays on its own game, and that game carries the queue value from step 2, so a 6v6 tank rank is never filed with the 5v5 tank ranks. `daemon::rank::queue_hint` keeps the `6V6` word for a role queue banner, so the next game's hint from a 6v6 role queue rank screen never agrees with a 5v5 ROLE QUEUE title.
5. Games shows the queue in words on the chip and the open row (wording in Q2 and Q4).
6. Upload: nothing new goes on the wire until API's contract PR has merged into `wip/0.7`. Until then the upload sends what it sends today.
7. Synthetic data until Season 6 starts: 12-player boards, title strings and rank banner strings in tests. After 8 Dec, Vision's real frames give a `6V6 ROLE QUEUE` title class. The queue title templates ship as the server pack `queue-title-v1.tar` (`docs/tracker-packs.md`), so a new class can reach trackers that sync packs as a new pack version, on their next pack sync, with no new build, as long as step 2 already accepts that class name. The pack must pass `eval` against the published one, and publishing (`scripts/publish-packs.sh`) needs FrozenTear's go.
8. Default setting: the queue title reader runs only with Read my rank after Comp games on, which is off by default. With it off, a 12-player game is "6v6, queue unknown". See Q5.

### Companion page switching (0.7.1)

This follows companion-spec.md option 3, which it marks as recommended and the 0.7 default.

1. Pages: Tonight (the stream page and the default), Map vote, Picks and bans, Matchup, Tip.
2. Auto-advance, on by default: Map vote when the three maps were read, then Picks and bans when the map locks. Auto-return to Tonight at match start (the loading screen or the first live Tab), always on.
3. Keys: next page and previous page, plus a safe page key, Super+Shift+T, that jumps to Tonight and holds it until the next next-page press. Super+Shift+C still hides the whole overlay. All binds can be changed in Settings.
4. Indicator: four dots and the page name in words on the in-match pages. "Held on the stream page" when the hold is on, and "Safe page with Super+Shift+T" in the footer. The Tip page shows "Tip" instead of the dots.
5. How it runs: the desktop app reads the keys (`hotkey.rs`, evdev, as today) and writes the page state to a small file in the data dir. The companion reads it on its tick (1 s today, which may need to be shorter for this file). The tracker publishes the pre-game stage (map vote with its maps, hero select, hero ban, match start) as a new optional field in the status file, from `detect::GamePhase`.
6. Data: the Picks and bans card uses the 779 counts filtered to the map, with a floor of 5 decided games on that map and the all-maps line from 10.
7. Fallbacks: if the vote maps or the locked map are not read, the companion stays on Tonight. On X11 there is no companion, and the notice says so in the Companion card.
8. The new settings (Hold the stream page, the page binds) are app-only keys, so they never count as changes waiting.

## What changes in code

### Rules for every PR

- **Size guard.** `crates/stat-tracker/src/tests_file_size.rs` fails any tracker file over 1500 lines unless it is in `CAPS`, and fails a `CAPS` file that is over or under its cap. It runs in the `scuffed-stat-tracker` lib tests, so a PR that changes only `stat-tracker-ui` still trips it in CI. Run that test before asking for review. Files this plan touches that are at or near the limit:

  | File | Lines | Note |
  | --- | --- | --- |
  | `crates/stat-tracker-ui/src/app.rs` | 2911 | In `CAPS` at exactly 2911. No growth. A shrink lowers the cap in the same PR. |
  | `crates/stat-tracker-ui/src/settings.rs` | 3791 | In `CAPS` at exactly 3791 (classic page). |
  | `crates/stat-tracker-ui/src/settings_model.rs` | 1499 | One line of room. |
  | `crates/stat-tracker/src/daemon/tab/grab.rs` | 1498 | Two lines of room. |
  | `crates/stat-tracker-core/src/ocr_status.rs` | 1476 | |
  | `crates/stat-tracker-ui/src/games_detail.rs` | 1450 | |
  | `crates/stat-tracker/src/daemon/event_loop.rs` | 1448 | |
  | `crates/stat-tracker-ui/src/games_table.rs` | 1442 | |

- **Tests.** The tracker's daemon tests live in its bin target, so authors run both `--lib` and `--bin` for `scuffed-stat-tracker`, plus `-p scuffed-stat-tracker-ui`. CI on the head SHA is the bar.
- **CHANGELOG.** Each user-facing PR adds its lines under a new `## Unreleased` heading in `crates/stat-tracker/CHANGELOG.md` (there is none after the 0.6.0 bump). The bump PR renames it.
- **What leaves your PC.** Any PR that adds an upload field or kind updates `what_leaves.rs` in the same PR.
- **Repo text.** Copied specs name the owner FrozenTear, call the game Overwatch, and use no em or en dashes.
- **Private frames.** Real Season 6 frames never go into the repo, and nothing goes under `crates/stat-tracker/test-data/`.

### CI for `wip/0.7` (T1)

- `.github/workflows/stat-tracker.yml`: add `"wip/0.7"` to `on.push.branches` and `on.pull_request.branches`. Later the 0.7.1 branch goes into the same lists and into `ci.yml` (C1).
- This is a CI workflow edit, so T1 and C1 need FrozenTear's go before anyone opens them.

### What leaves your PC and the other dialogs (#879, T3)

- `what_leaves.rs` `dialog`: keep the scrolled body clear of the scrollbar. iced 0.14's default bar is 10 px wide and floats over the content. Either pad the body on the right by `layout::SETTINGS_SCROLL_GUTTER` (the 10 px bar plus a gap), or embed the bar with `Scrollable::spacing` so it takes its own space. `settings.rs` `scroll_frame` pins the bar width for the classic page and explains why it keeps a gutter instead.
- Check the same pattern in `notes.rs` (body padded 8 px, bar 10 px), `report.rs` (the Report body scrollable), `uninstall.rs` (`dialog_shell` body) and `setup_guide.rs` (the guide body). Fix only the ones that clip.
- Test: lay the dialog out at 1280x860 with the headless renderer, as the `settings_page.rs` tests do (`headless`, `lay_out`), and check that every text block ends left of the scrollbar.

### Scroll kept after a toast (#806, T4)

- `chrome.rs` `top_tabs`: always build `stack![window, prompts]`, with an empty layer when `floating_prompts` returns `None`, so the tree keeps its shape when a toast or the restart bar comes and goes.
- Review `chrome::with_layers` the same way: each open dialog adds a stack level, which may also reset the page scroll when a dialog opens. Change it only if QA or a test shows a reset.
- Test: build the window with and without a toast and check that the path to the `PANE_SCROLL_ID` scrollable has the same widget tags (iced `advanced` tree). QA repeats the #806 steps: the 8th of 14 games at 1920x1080, a successful save and a refused one, then the toast timing out.

### Rank text column (T5)

- `crates/stat-tracker/src/games/model.rs`: `Game` gains the rank read from `PersonalMatch::rank_update` in the conversion that already copies `queue_title`.
- New `crates/stat-tracker-ui/src/games_rank.rs`: the rank text, progress and change, and a one-sentence tooltip. The words wait on Q2; the site's badge ("Diamond 2", "45%", "+23%") is the reference.
- `games_table.rs` (the `Col::Rank` cell, line 718), `games_rows.rs` (`SORT_NOTE`; Rank stays not sortable in 0.7.0), `fixtures.rs` (some fixture games get a rank).

### Win rate when a hero was banned (#779, T9 and T10)

- New `crates/stat-tracker/src/games/bans.rs` (T9), shared by the app and the 0.7.1 companion. It parses the stored `bans_picks` JSON with one parser; today the parser is private to the app (`crates/stat-tracker-ui/src/games_bans.rs`, `fn parse`), so it moves or the app calls the shared one. Per banned hero: games, `aggregate::Record`, the Your ban, Enemy and Side not read split, a lobby ban only when it is not one of the four slots, votes never counted, the left-out count, and `small_sample::RATE_MIN`. A per-map query for the companion.
- `heroes_page.rs` (T10): `HeroesPage` gains a mode (Played or When banned) and `heroes_page::Msg` a message for it. Cards | List hides in When banned. The file is 926 lines, so the new view goes in a new file.
- New `crates/stat-tracker-ui/src/heroes_banned.rs` (T10): the table, the caption, the stats line, sortable headers and the Games | Win rate | Recent segment like `maps_tab.rs` (`header`, `sort_line`, `empty_card`), the under 10 rule per hero, and the empty, loading and filter states from the 779 spec.

### Status pill (#775, T2 and T6)

- T2 copies UX Spec's `spec.md` and Design System's `switch-spec.md` into a new `docs/design/` folder, as the issues ask.
- New `crates/stat-tracker-ui/src/status_pill.rs`: pill state (verb in flight, last error, menu open, confirm), the pill and menu views, and the update code for `Message::Daemon` and `Message::DaemonDone`, moved out of `app.rs`. That move frees the lines the new fields need, so `app.rs` ends at or under 2911.
- `nav.rs` `status_slot`: draws the pill. `nav_fit.rs` `status_slot_width`: counts the pill border, padding and chevron.
- `tracker_status.rs`: words for Starting..., Restarting..., Stopping... and Couldn't start.
- `chrome.rs`: the menu and the confirm as layers, on top of the T4 stack shape.
- `settings_cards.rs` `tracker_service`: the state row uses the pill's words and points to the pill.
- `daemon.rs`: unchanged.

### Pinned Settings bar (#776, T7)

- New `crates/stat-tracker-ui/src/settings_bar.rs`: counts, copy, Discard, Save, Save and restart, Save and start, and the leave confirm.
- `settings_save.rs`: a `pub(crate)` helper that lists the dirty draft groups (`Draft` and `DRAFTS` are private today).
- `settings_update.rs`: a message for the bar and Discard.
- `chrome.rs` `top_tabs`: the bar as a row under `page_body` on Settings only, under the floating toast.
- `app.rs`: `Message::Navigate`, the tray Hide and Quit, and `Message::WindowClosed` go through the leave confirm, with no net growth.
- `settings_page.rs`: remove `status_strip`, `strip_status`, `strip_actions` and `strip_action` and their tests. `tracker_status::strip_label` goes with them.

### Settings cleanup (#778 rest, T8)

- `settings_cards.rs` `companion`: the Wayland notice as a notice in the card (icon, `ACCENT_SOFT` tint). The message from `overlay::reconcile_companion` goes to page state instead of a toast, and the no-respawn rule stays.
- `settings_page.rs` `setup_and_legend`: Run setup again moves into `settings_cards.rs` `troubleshooting`.
- `settings_page.rs` `fold_header`: the pill's chevron icon instead of a ">" character.
- `settings_model.rs` `AdvancedSection::summary`: one line each. The file is at 1499 lines, so move its tests to a new file first, or keep the change line-neutral.

### One switch control (#777, C9)

- New `crates/stat-tracker-ui/src/switch.rs`: a small custom widget (iced `advanced` widget) with a 40x24 track, a 20x20 thumb, a focus ring, Space to toggle and a 120 ms slide. It is the first custom widget in this crate.
- `settings_rows.rs` `switch` and `switch_style` use it. `theme::companion_toggle` is restyled or retired.
- `settings_cards.rs` `companion`: the shortcut checkbox becomes a switch.
- `settings.rs` (classic): the checkboxes at lines 1354, 1430, 1512, 1589, 1604, 1618 and 1654 become switches with no line growth (cap 3791).
- `form_widgets.rs` `checkbox_style` stays for Uninstall and Report a problem.

### Report kit (#811 T11, #810 T12)

- `crates/stat-tracker/src/daemon/debug_dumps.rs` `record_report_kit` (T11): a warning when `TeamLayout::from_team_size` refuses the size, then still write the reads with an unknown layout. A warning for each kit write error, in place of the debug line. `daemon/tab/grab.rs` (1498 lines) does not grow at the call site.
- `daemon/event_loop.rs` lines 793 to 802 (T12): the crop gate calls a small helper (for example in `debug_dumps.rs`) that also asks whether the game has a stored board or is a start screen game waiting for its first board, so `event_loop.rs` stays under the limit. `report/kit/rank.rs` `record_rank_screen_pngs` stays the writer.
- `daemon/session/transitions.rs` `should_start_fresh_session` (T12, last): no fresh session from a rank or end screen alone, next to `POST_MATCH_GRACE` and `is_empty_tab_game`.
- Tests: a replay of the 4f17bcff orphan, accepted Tabs that leave a kit when the team size is refused, and a logged write error. New tests go in new files (`daemon/tests_rank.rs` is already 1428 lines).

### 6v6 role queue (#654, T13, T14, T15)

- `crates/stat-tracker-core/src/model.rs` (T13): the local queue value and the rules in the role queue flow, with a new helper that reads the title's words in any order, case and punctuation (ROLE or OPEN, and 5V5 or 6V6). `queue_key` stays as it is, because `hint_agrees` compares keys and must stay strict. `QueueTitle::is_open_queue` keeps its meaning.
- `crates/stat-tracker/src/daemon/rank.rs` `queue_hint` (T13): keep `6V6` for role queue. `daemon/queue_title.rs` (`hint_agrees`, `settle_hint`): tests for the new spellings.
- `crates/stat-tracker/src/storage/` (T13): a new optional field, stored once per game in the `storage/queue_title.rs` pattern, with the team size passed from the Tab path through a helper (`grab.rs` has no room).
- Tests (T13): synthetic 12-player boards (as `shadow/reader.rs` builds them with `tab_stand_in` and `PITCH_6V6`), title strings and rank banner strings. No real frames.
- `crates/stat-tracker-ui/src/games_detail.rs` `queue_value` and `map_chips` (T14): the chip names the queue value. `filter_chips.rs` and its disabled "All queues" chip are left as they are until Q10 is answered.
- `crates/stat-tracker/src/sync/wire.rs` `upload_rank` (T15): after API's contract PR (#889) merges. Field: `rank.queue` as `{value, confidence}` (confidence may be null). Values: `role_5v5`, `open_5v5`, `role_6v6`, `open_6v6`, `unknown`. Competitive only; Quick Play, Stadium and arcade omit the rank object. A Competitive game with no rank read still sends a rank object holding only queue. `unknown` is a Competitive game whose queue could not be read. OPEN with 5 per team maps to `open_5v5`. `rank.queue_title` stays the raw title text. `what_leaves.rs` `RANK_LINE` names the new field.

### Companion pages (0.7.1)

- `crates/stat-tracker-ui/src/overlay.rs`: `tonight_strip` draws W, L or D in each square; the view picks the page; the footer shows the page and the hold.
- New `crates/stat-tracker-ui/src/overlay_pages.rs`: the page model, auto-advance, auto-return and the hold rules, as plain functions with unit tests.
- `crates/stat-tracker-ui/src/hotkey.rs`: more than one bind (next, previous, safe page), plus file triggers like `companion_toggle` for compositor binds.
- `settings_cards.rs` `companion` and `settings_model.rs` `APP_ONLY_KEYS`: the binds and Hold the stream page.
- `crates/stat-tracker-core/src/ocr_status.rs` `Status`: a new optional stage field, its type in a new file (the file is 1476 lines). `crates/stat-tracker/src/daemon/event_loop.rs`: writes it from `detect::GamePhase` (`MapVote { maps }`, `HeroBan`, `HeroSelect`, `InGame`) through a helper.
- `crates/stat-tracker/src/games/bans.rs`: the per-map query with the floor of 5.

## Order of PRs

Sizes count changed lines with tests: S under about 150, M 150 to 500, L over 500. Every PR is a draft titled `HOLD-MERGE: ...`. "Window" is when it should be merge-ready. QA batches are a proposal, so QA can cover several heads in one run (at most one run a day).

### 0.7.0, base `wip/0.7`

| # | PR | Depends on | Size | Stamp | Window | QA batch |
| --- | --- | --- | --- | --- | --- | --- |
| T1 | ci: run Stat Tracker CI on wip/0.7 | none | S | Site | Tue 24 Nov | CI only |
| T2 | docs: copy the tracker status pill, Settings bar and switch specs into docs/design/ | none | S | Site | Wed 25 Nov | none |
| T3 | stat-tracker-ui: What leaves your PC text clear of the scrollbar (#879) | T1 | S | Site | Thu 26 Nov | A |
| T4 | stat-tracker-ui: keep the page scroll when a toast shows (#806) | T1 | S | Site | Fri 27 Nov | A |
| T5 | stat-tracker-ui: rank text in the Games Rank column | T1, after T4 | M | Site | Fri 27 Nov | A |
| T6 | stat-tracker-ui: status pill with Start, Restart and Stop (#775) | T1, T2, T4 | L | Site | Mon 30 Nov | B |
| T7 | stat-tracker-ui: pinned Settings bar, strip removed (#776, #778 wording) | T6 | L | Site | Tue 1 Dec | B |
| T8 | stat-tracker-ui: Settings cleanup (#778 rest) | T7 | M | Site | Tue 1 Dec, stretch | B |
| T9 | stat-tracker: ban counts per hero for the When banned view (#779 part 1) | T1 | M | API | Fri 27 Nov | C |
| T10 | stat-tracker-ui: When banned mode on Heroes (#779 part 2) | T9, after T4 | L | Site | Mon 30 Nov | C |
| T11 | stat-tracker: report kit logs refused sizes and write errors (#811) | T1 | S | API | Thu 26 Nov | D |
| T12 | stat-tracker: no orphan report kit from a rank or end screen (#810) | T11 | M | API | Mon 30 Nov | D |
| T13 | stat-tracker: 6v6 role queue value and per-queue rank, local only (#654 part 1) | T1 | M to L | API | draft Sat 28 Nov, ready Wed 2 Dec | D |
| T14 | stat-tracker-ui: show the queue value on Games (#654 part 2) | T13, T5 | S | Site | Wed 2 Dec | D |
| T15 | stat-tracker: send the queue value in API's upload shape (#654 part 3) | T13, API's contract PR | S to M | API | Wed 2 Dec | D |
| T16 | stat-tracker: bump to 0.7.0 | all merged items | S | Site | Fri 4 Dec | release check |

Notes:

- T1 goes first. Without it no tracker PR on `wip/0.7` can show the Stat Tracker CI jobs at its head.
- T4 lands before T6 and T7, because all three change `chrome.rs` and the pill and the bar add layers on top of it. One author owns `chrome.rs` in that order.
- On Games and the stats views, #806 (T4) lands first, then the rank column (T5) and #779 (T10).
- #810 and #811 go together: two small stacked PRs (T11, then T12) on the report kit path, reviewed by API and QA'd as a pair.
- #775 lands before #776. T8 is a stretch. If it is not ready by Tue 1 Dec it moves to 0.7.1 with #777.
- T13 and T14 do not depend on API's answer and can merge either way. T15 waits for API's contract (wire value agreed by Thu 26 Nov, per #654). If the contract PR has not merged by Tue 1 Dec, Chief decides whether T15 slips (Q4).
- Wed 2 Dec: every 0.7.0 head merge-ready, with QA at that head. Thu 3 Dec: freeze. After the freeze only release fixes.
- Proposed QA runs, one a day: A on Fri 27 Nov, C on Mon 30 Nov, B on Tue 1 Dec, D on Wed 2 Dec, then the release check from Fri 4 Dec. A batch whose heads are not ready drops the late PR, never the day.

### 0.7.1, base the 0.7.1 branch

This plan calls it `wip/0.7.1`; the name is Chief's call. Cut it from `wip/0.7` when 0.7.1 work starts, and merge `wip/0.7` into it after each 0.7.0 merge (merge commits, no force-push). After the 0.7.0 tag, check its CHANGELOG against that tag.

| # | PR | Depends on | Size | Stamp | Window |
| --- | --- | --- | --- | --- | --- |
| C1 | ci: run CI on the 0.7.1 branch (`ci.yml` and `stat-tracker.yml`) | branch cut | S | Site | the day the branch is cut |
| C2 | docs: copy the companion spec into docs/design/ | none | S | Site | early |
| C3 | stat-tracker: publish the pre-game stage for the companion | C1 | M | API | Fri 4 Dec |
| C4 | stat-tracker-ui: companion page model, page indicator and Tonight letters | C1, C2 | M | Site | Fri 4 Dec |
| C5 | stat-tracker-ui: companion page keys and Hold the stream page | C4 | M | Site | Mon 7 Dec |
| C6 | stat-tracker-ui: Picks and bans and Matchup pages | C3, C4, T9 | L | Site | Tue 8 Dec |
| C7 | stat-tracker-ui: Map vote page | C6 and a Vision read of the vote maps | M | Site | stretch |
| C8 | stat-tracker-ui: Tip page (no death-rate slot) | C6 | M | Site | stretch |
| C9 | stat-tracker-ui: one switch control (#777), plus T8 if it moved | T7 | L | Site | Tue 8 Dec |
| C10 | stat-tracker: bump to 0.7.1 | all merged items | S | Site | after combined QA, Mon 14 Dec |

Combined QA runs Wed 9 to Sat 12 Dec, at most one a day, plus one live session per release candidate.

## Risks

The top three are marked.

| # | Risk | Kind | Mitigation |
| --- | --- | --- | --- |
| 1 | **Top.** Role queue support ships blind. No real Season 6 frames exist before Tue 8 Dec 20:00. The pre-game title may not read as ROLE QUEUE. The queue title reader is off by default. | Technical and schedule | Rules that use team size and the rank role line, a title helper that reads the words in any order, "queue unknown" with a warning and never a dropped game. A day-one title class as a `queue-title-v1.tar` pack update, with no new build. QA checks the first real games on 8 and 9 Dec. The day-one path is agreed before the tag (Q14). |
| 2 | **Top.** Stat Tracker CI does not run on `wip/0.7`, so tracker PRs could look green with no tracker tests. | Process | T1 first. No tracker PR is merge-ready without the Stat Tracker jobs green at its head. The same for the 0.7.1 branch (C1). |
| 3 | **Top.** Nine days to the freeze for about fifteen PRs, with review stamps and QA at most once a day. The API contract for the queue field is also on the clock (agreed by Thu 26 Nov). | Schedule | Small PRs, QA batches, stretch items named up front (T8, #777). T13 and T14 do not wait for API. A stop-the-line check on Tue 1 Dec: anything not ready moves to 0.7.1 with Chief's say. |
| 4 | The size guard turns CI red late in review. `app.rs` cannot grow at all. | Technical | New modules, moves out of `app.rs` with the cap lowered in the same PR, and the guard run before every review request. |
| 5 | Shared chrome changes (#806 fix, pill menu, pinned bar) change scroll or focus on every tab and dialog. | Technical | One owner for `chrome.rs` in the order T4, T6, T7. A tree shape test. QA walks every tab and dialog. |
| 6 | The mid-game confirm misses a real match, or asks on a menu. | Technical | Use both `game_file_in_progress` and `live_game_session`. Unit tests on the state table. QA with a live game. |
| 7 | Switch-spec and UX Spec disagree on instant switches. Building the wrong one costs a rework. | Spec | Keep today's instant writes until Chief answers Q7. #777 sits in 0.7.1. |
| 8 | The #810 session change splits or merges games wrongly. | Technical | Crop gate first. The session change last, only with a replay test that fails first. API review. |
| 9 | A new local field breaks old rows or an older build. | Technical | An optional field with a serde default, written like `queue_title`. Never a field old rows lack with a non-optional type. |
| 10 | 0.7.1 needs reads that do not exist: the vote maps (the new reader's map vote names no maps) and the locked map on hero select and ban. | Dependency (Vision) | Ship the page framework, Tonight, Matchup and Picks and bans with a Tonight fallback. Map vote is a stretch. Chief confirms with Vision what is in 0.7.1 (Q6). |
| 11 | A day-one 6v6 fix and the 0.7.1 combined QA want the same days (9 to 12 Dec). | Schedule | Prefer the pack update path. Chief picks the QA slot and the version (Q14). |
| 12 | Page keys clash with compositor binds, need the input group, or switch pages a second late. | Technical | Binds can be changed. File triggers for compositor binds, as for the hide key. A shorter companion tick for the page file. QA on each desktop. |
| 13 | A hint page stays up on a stream. | Product | Auto-return at match start is always on. The safe page key. Hold the stream page. |
| 14 | CHANGELOG lines land under `## 0.7.0` on the 0.7.1 branch after the bump, with no conflict. | Process | Diff the CHANGELOG against the latest tag on each merge, and dry-run `crates/stat-tracker/dist/release-notes.sh`. |
| 15 | The pill's pulse ignores a reduced motion setting. The app reads no such setting today. | Accessibility | A static amber dot in 0.7.0 unless Q12 says otherwise. |
| 16 | The first custom iced widget (#777) is harder than it looks: focus, Space and the slide. | Technical | 0.7.1, behind `settings_rows::switch` so call sites do not change. |

## Setups to test

QA checks the upgrade on three setups: niri, Linux Mint with Cinnamon on X11, and KDE Plasma on Wayland. Each QA batch covers at least one of them, and the release checks for 0.7.0 and 0.7.1 cover all three. Use the PR head build (or the release candidate), the default 1280x860 window, and 1000 px and 720 px wide windows where noted.

| Check | niri | Mint X11 (Cinnamon) | KDE Plasma Wayland |
| --- | --- | --- | --- |
| Setup facts | Focus from `NIRI_SOCKET`. No tray host of its own. No GlobalShortcuts portal, so keys come from evdev or a compositor bind on the file trigger. | Focus from `_NET_ACTIVE_WINDOW`. Tray built into Cinnamon. No layer-shell. | Focus from the small KWin script. Tray built into Plasma. Layer-shell supported. |
| Status pill (#775) | Words at 720, 1000, 1280 and 1440 px. Menu by mouse and keys (Space, Enter, arrows, Escape). Mid-game confirm. Error state (unit stopped, or binary missing). Stopping... for a slow stop. | Same. With and without the systemd user unit. | Same. |
| Pinned bar (#776) | The bar never covers the last row. The toast sits above it. The leave confirm on a tab change and on Quit. | Same, plus Hide window from the tray. | Same, plus Hide window from the tray. |
| Companion | 0.7.0: no change. 0.7.1: overlay top right, click-through, never takes focus, pages switch by key, auto-return at match start. Check niri's own binds for a Super+Shift+T clash. | No overlay. The notice shows in the Companion card (and compactly on Overview after T8). The app does not keep respawning the companion. | As niri, over a fullscreen game. Check Plasma's shortcuts for a clash. |
| Tray | Shows only in a bar with a tray (StatusNotifierItem) module. Without one, check what Hide window and closing the window do: the app may think it has a tray, and a second launch brings the window back (`instance::request_show`). The "No system tray" toast shows only when the AppIndicator library is missing. | Show window, Hide window, Hide / show overlay, Quit. | Same as Mint. |
| Scrollbar and dialog clipping (#879) | What leaves your PC, Report a problem, release notes, Uninstall and the setup guide at 1280x860 and 1000x768: no text under the scrollbar. | Same. | Same. |
| Font and scale | Output scale 1, 1.25 and 2. | User interface scaling 1x and 2x, and a text scaling factor of 1.25. | Display scale 100, 125 and 150 percent. |
| Games (#806, rank column, #654) | The #806 steps. Rank text with and without a rank read. The queue chip and warning. | Same. | Same. |
| When banned (#779) | 1280 and 1000 px: every column stays, the Your ban tag stays, the stats line stays on one line. | Same. | Same. |

On scale: iced draws its own bundled Urbanist font and follows the window scale factor only, so check the text at each scale. A higher scale also shrinks the width in logical pixels (a 1920 px screen at 150 percent gives 1280), which moves the pill, the bar and the When banned table into their narrow forms. On X11 the scale may come from the X server settings rather than Cinnamon's text scaling setting; check which one moves the app.

**Screenshots for FrozenTear.** FrozenTear reviews mockups and screenshots on a phone. For each UI PR, attach a full window shot at 1280x860, one at 1000 px, a crop of the changed area at 2x, and one sheet 1080 px wide and under 2400 px tall with every state of the change, text big enough to read without zooming. Fixture runs (`stat-tracker-gui --fixture sample`, `games` or `full`, and `--companion --fixture sample`) cover the static pages. Fixture mode always shows Ready and turns the pill menu off, so pill and bar states come from a real tracker run.

## Overlap with Site / UX Spec

Open questions for Chief, numbered Q1 to Q14. This plan does not decide them. Where a PR depends on an answer, it says so.

1. **#783 rank graph range (UX Spec designs, Site builds, API adds a date filter).** #783 offers first page, last week, last month and last 3 months, with the site's `RankWindow` captions. The tracker's progress view, parked for 0.9, has its own range picker (a month, a season or all games). Should the tracker use the same ranges and words, and does #783 stay 0.7 or move to 0.8?
2. **Rank wording reuse from the site.** T5 shows the rank as text. The site words it in `crates/app/src/pages/stats/rank.rs` (`RankShown::rank_text` "Diamond 2", `badge_parts` with "45%" and "+23%", `context_label` "Support, Role Queue", `sentence` "Rank: ..."). Copy those words into the tracker, or share one helper through `crates/types` (API and Site own that crate)? Who words "6v6 Role Queue" for both?
3. **Stats privacy (#781, a draft HOLD-MERGE docs PR on `wip/0.6`).** When banned counts both teams' bans on this PC, and the 0.7.1 companion shows per-map ban hints from the member's own games. Does #781 need to land, or be retargeted to `wip/0.7`, before #779 or 0.7.1 ship? Note that `bans_picks.picks` already holds teammates' hero select and is uploaded today.
4. **API upload contract for role queue (#654 item 3).** Settled 2026-11-24 and merged as #889. Field: `rank.queue`, `{value, confidence}` (`scuffed_types::api::RankQueue`). Values: `role_5v5`, `open_5v5`, `role_6v6`, `open_6v6`, `unknown`. Competitive only; Quick Play, Stadium and arcade omit `rank` entirely. `unknown` is a Competitive game whose queue could not be read. `rank.queue_title` stays the raw title text. OPEN with 5 per team maps to `open_5v5`. T15 sends this shape and names the field on the What leaves your PC line.
5. **Queue title reader default.** It runs only with Read my rank after Comp games, which is off by default. So by default the tracker cannot tell 6v6 role queue from 6v6 open queue. Keep that, or let the queue title reader run on its own? That changes the upload and the What leaves your PC copy (API and UX Spec).
6. **Companion spec ownership.** companion-spec.md (UX Spec) and the companion design notes (Chief) live outside the repo. Who owns the copy in `docs/design/`, and who signs off changes during the 0.7.1 build? Still open: default binds for next and previous page (only Super+Shift+T is specified), the "Hold the stream page" wording, and which Vision reads (vote maps, locked map) are in 0.7.1.
7. **Switch and Settings specs.** UX Spec's spec.md 2.1 keeps switches instant and says not to bring back a Save. Design System's switch-spec.md says flipping a switch marks the form dirty and waits for the pinned save bar, Start on login included. Which holds for 0.7? Also: do the Companion switches outside Settings take the new look? Those are the switch in the Overview Companion card (`overview_side.rs` `companion_card`) and `widgets::overlay_toggle`, drawn on the Games control row (`games_table.rs`) and in the classic header. They are not on the switch-spec list.
8. **Bans and picks chip style.** FrozenTear prefers chips for ban and pick hero names (a3b). Should the hero names in the When banned table and in the companion's ban hint rows be chips in the Games panel look (`games_bans_view::chip_look`), or plain names with the "Your ban" tag as the 779 spec draws them? Does the site's match page bans section follow the same chip style?
9. **Sample floors.** The tracker uses 10 decided games (`RATE_MIN`), the companion spec uses 5 per map, and the site stats notes (`docs/notes/stats-ui-redesign.md`) use a 3 game gate for top win rate callouts. Keep the three numbers on purpose?
10. **Queue filter.** Site adds role queue 6v6 to its stats filters (#654). The tracker's "All queues" chip is disabled until the queue title reader is on (`filter_chips.rs`), and the 779 view inherits a future queue filter. Turn on a tracker queue filter in 0.7.0, or wait?
11. **Rank text column scope.** Games table only, or also the open game panel (the games expanded spec left rank out because it was not stored then) and Seasons? Should the column sort?
12. **Pill motion.** The spec asks for a pulsing dot with no pulse under reduced motion. The app has no reduced motion source today. Ship a static amber dot in 0.7.0? And the words for known start failures (no unit, binary missing, tracker exited at once, a systemd error), which GUI Review asked to name.
13. **The a-sidebar-0.7 frames** (`A-settings-everyday`, `A-settings-advanced`, `A-optin`, `A-report` and the `variant-a-flows` sheet under `/workspace/scuffed/0.6/redesign/ux-spec/a-sidebar-0.7/`). This plan keeps the top tabs and today's Settings page. Are any of these meant for 0.7?
14. **Day-one 6v6 fix.** A pack update only, or a tracker build too? Which QA slot, given 0.7.1's combined QA on 9 to 12 Dec? Which version number, if a build is needed?

## Out of scope

- **Parked for 0.9 (#881):** the rank progress screen in the app (rank up, down or same, from local `rank_update` data, an empty state that points to Read my rank, the site's wording) and its range picker (a month, a season or all games). The website's month range is not part of it.
- **0.8:** #780 enemy heroes per game, #782 teammate hero pairings, the Matchup sections that need them, #772 per-hero SR from the rank screen, the session and tilt view, a review queue for unsure stats, and patch-aware stats.
- **Not on #881's 0.7 lists:** #791 window capture (feasibility notes only; FrozenTear asked for it in 0.7 on 19 Nov, so Chief should confirm it stays out) and #344 the display portrait pack (Site, API and Vision; the tracker's pack path does not change).
- The sidebar layout, and classic layout changes beyond keeping its words in line.
- A global Start or Stop shortcut, Start or Restart in the tray, and any change to `run_verb` or systemd.
- An X11 companion overlay.
- The death-rate line on the Tip page (it waits on Blizzard's OK).
- Ban aggregates on the site or the API, and any "best ban" wording.
- #807 (four vision tests fail on Tesseract 5.5.4), unless Vision or QA takes it. #774 and #749, older drafts on `wip/0.6` (Chief decides 0.6 or a retarget).

## Appendix A. Dates (Europe/Oslo)

| Date | What |
| --- | --- |
| Mon 23 Nov | `wip/0.7` cut at the 0.6.0 tag. This plan. |
| Thu 26 Nov | API wire value for role queue agreed (#654). UX Spec's 779 placement is already posted on #779. |
| Sat 28 Nov | #654 tracker draft PR (T13). |
| Wed 2 Dec | Every 0.7.0 head merge-ready, with QA at that head. |
| Thu 3 Dec | Freeze. |
| Fri 4 Dec | Bump PR (T16). |
| Sun 6 or Mon 7 Dec | Tag, on FrozenTear's go. |
| Tue 8 Dec 20:00 (19:00 UTC) | Hard deadline. Season 6 starts with 6v6 role queue. |
| Wed 9 to Sat 12 Dec | 0.7.1 combined QA. |
| Mon 14 to Wed 16 Dec | 0.7.1 tag. |

## Appendix B. Inputs read

- #881 (body and comments), #775, #776, #777, #778, #779 (UX Spec comment), #806, #810, #811, #654, #879, #780, #783, #791, #344, and the titles of #807, #774, #749, #781, #782 and #772.
- UX Spec and Design System: the tracker spec and audit (status pill, Settings bar, switch control, Settings notes), switch-spec.md, ban-winrate.md and its #779 comment, focus-status.md, settings-stats-visibility.md, games-expanded.md, games-expanded-v2.md, games-delete.md, games-picker.md, bans-picks.md, bans-picks-a3.md (with a3b).
- Companion: companion-spec.md, the companion design notes, and the briefs (gap check, held 783, ban hint card).
- Roadmap: roadmap-0.6-0.8-consolidated.md and the Season 6 note (s6-rq6.md).
- Repo: `crates/stat-tracker-ui`, `crates/stat-tracker`, `crates/stat-tracker-core`, `crates/types/src/api/stats.rs`, `crates/app/src/pages/stats/rank.rs`, `crates/stat-tracker/README.md`, `crates/stat-tracker/CHANGELOG.md`, `docs/tracker-packs.md`, `docs/notes/stats-ui-redesign.md`, `.github/workflows/stat-tracker.yml` and `.github/workflows/ci.yml`.
