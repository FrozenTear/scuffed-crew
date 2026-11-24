# Scuffed Tracker: status pill and pinned Settings bar

Spec for two 0.6 interactions, against `wip/0.6` at `9ec2bb662cbd3894d4e399006d3958351474e820`. Frames are in `/workspace/scuffed/tracker-ux/frames/`. Phone review sheets are in `/workspace/scuffed/tracker-ux/robert/`.

This does not change the daemon. Start, Stop and Restart stay as `daemon::run_verb` (`crates/stat-tracker-ui/src/daemon.rs:592`): the GUI stays open, and only the tracker process is started, stopped or restarted. Instant Settings writes stay instant. The pinned bar is for uncommitted typed drafts and for already saved changes that need a tracker restart.

No em or en dashes in UI copy. Call the game Overwatch.

## 1. Status pill in the top bar

### 1.1 Where it lives

The pill replaces the current status slot at the right of the top bar, before the "needs a look" chip when that chip is shown (`nav.rs:350`, `status_slot`). It uses the same breakpoints as today:

- Window under 1200 px: short words, no time (`short_label`). Capturing states use short words under 1100 px, and their long words with a time from 1100 px, adding "last game" from 1440 px.
- Window 1200 px and up (non-capturing): long words, and if there is a last game, " · " plus the time (`bar_status`).

The pill is a control on every top tabs screen, not only Settings. The classic sidebar header is out of scope for this pass.

The Settings status strip (`settings_page.rs` `status_strip`) is removed once this pill and the pinned bar ship. Version stays on the Updates card and in About. Last game time stays on the pill.

### 1.2 Status wording (one set of words)

Use the current 0.6 bar words, not the Settings strip's "Tracker running" for an idle daemon.

| State | Short (narrow) | Long (wide) | Mark | Time |
| --- | --- | --- | --- | --- |
| Stopped | Stopped | Tracker stopped | Red dot | Last game, if any, on the long form |
| Starting | Starting... | Starting... | Amber or neutral pulsing dot (not Ready green). No pulse when reduced motion is on. | No |
| Ready (daemon up, Overwatch not open) | Ready | Tracker ready | Green dot | Last game, if any, on the long form |
| Live, Overwatch in front | In game | Watching Overwatch | Green dot | Last game, if any, on the medium and long forms |
| Live, capture any window | In game | Capturing in any window | Green dot | Same as Live |
| Paused (Overwatch not in front) | Paused | Paused while Overwatch isn't in front | Grey ring | No |
| Focus unknown | ? (round button under 1100 px) | Can't tell which window is in front | Amber pill, as today | No |
| Restarting | Restarting... | Restarting... | Amber or neutral pulsing dot, same as Starting. No pulse when reduced motion is on. | No |
| Stopping | Stopping... | Stopping... | Amber or neutral pulsing dot, same as Starting. No pulse when reduced motion is on. | No |
| Error (start or restart failed) | Couldn't start | Couldn't start the tracker | Red dot | No |
| Token rejected | Token rejected | Website token rejected | Amber dot | Last game, if any, on the long form |

"Couldn't start" is also used when Restart fails (the tracker may now be down). The tooltip or menu can show the error string from `run_verb`.

Token rejected and the focus states are overlays on a running daemon. The menu still follows whether the daemon is up (Restart and Stop), not the warning. Focus unknown stays a special case: a click on the amber pill opens Settings at the focus row, as today (`Msg::ShowFocusSetting`). A chevron or a second small control on that pill opens the Start/Stop/Restart menu so the two actions do not collide. Under 1100 px the "?" button keeps opening Settings; the menu is available from a long press, or from the keyboard shortcut in 1.6.

Demo mode always shows Ready (current Waiting rule) and does not enable menu actions.

### 1.3 The menu

The pill always looks like a button: a 1 px `BORDER` resting outline in every state, including Starting, Restarting, Stopping and Error, and a `SURFACE` fill on hover. Open or focused, it also takes a 1 px `ACCENT` ring. A downward chevron appears at 1200 px and up, and on hover or focus at any width, except during Starting, Restarting or Stopping. At 1000 px the Error pill drops the chevron (and may shorten the label) so at least 16 px stays between the pill and the Settings tab.

The state words (Ready, Tracker ready, Watching Overwatch, In game, Stopped, Starting..., and the rest) are primary text: 12 px Urbanist SemiBold in `TEXT`. Only the time is muted (`TEXT_2`). Error uses the same primary text, not `TEXT_3`.

Open the menu on click, Space or Enter. Close it on Escape, click outside, or choosing an item. One menu, right-aligned under the pill, `SURFACE` background, `BORDER` 1 px, `RADIUS_INNER` (12 px), max width 280 px. Items are 14 px semibold, 8 by 12 px padding.

| Daemon state | Menu items |
| --- | --- |
| Stopped | Start tracker |
| Error | One line with the reason from `run_verb` plus a Troubleshooting link that opens Settings at Troubleshooting. Then Retry (runs Start or Restart, whichever failed; if the tracker is down, Retry is Start). |
| Ready, Live, Paused, Focus unknown, Token rejected | Restart tracker. Stop tracker. |
| Starting, Restarting, Stopping | No menu. Click does nothing. The pill is not disabled-grey; the words already say it is busy. |

If After restart changes are waiting (`changes_waiting` > 0), Restart tracker in the menu is the primary item (accent text or a tag "Applies N changes"). It still opens the confirm in 1.4 when a match is in progress, and it still follows the pinned-bar rules in 2.5 when unsaved drafts exist.

### 1.4 Confirm rules

Confirm only Restart tracker and Stop tracker while a match is in progress. Use `game_file_in_progress` (and/or a present `live_game_session`), not merely `game_running`. Overwatch open on a menu is Live on the pill, but it is not a confirm.

Copy, two dialogs, same body, no repeated title in the body:

- Stop: title "Stop the tracker?". Body "The current game won't be recorded." Buttons: Keep tracking (secondary, default, Escape or click outside) and Stop tracker (danger, filled).
- Restart: title "Restart the tracker?". Body "The current game won't be recorded." Buttons: Keep tracking (same as Stop) and Restart tracker (accent, filled).

No confirm when:

- Ready or otherwise idle (daemon up, no match in progress).
- Stopped, Starting, Restarting, Stopping, or Error.
- Start or Retry.
- Demo mode (actions are already disabled).

The update-to-new-binary path that waits for the game to save (`app.rs:1604`) is unchanged and is not this menu.

### 1.5 Unsaved or restart-needed settings

If the user picks Start, Retry, Restart or Stop while the pinned bar is showing unsaved drafts (section 2), do not silently drop them:

- Start, Retry or Restart: if drafts need a tracker restart once saved, or if After restart changes are already waiting, treat the item as "Save and restart" (save drafts, then `run_verb(Restart)` or Start if down). If drafts are app-only (shortcut, session window is already instant, update channel is already instant; the remaining drafts are timings and sync, which the tracker reads at start), saving them first is still required so the restarted tracker sees them.
- If the user has unsaved drafts and picks Stop: prompt "Save changes first?" with Save and stop, Discard and stop, and Cancel. Default is Cancel.
- If there are no drafts, only After restart changes already on disk: Restart proceeds (with the mid-game confirm when needed). Stop proceeds (with the mid-game confirm when needed) and leaves the saved file as it is. The waiting count stays until a later Start or Restart.

Details and button labels are in 2.5.

### 1.6 Keyboard, focus, labels

The app has no shortcut system for daemon verbs today, only the overlay bind (`Super+Shift+C`). Do not add a global Start/Stop key in this pass. If a later shortcut is added, it should go through the same confirm and draft rules.

Until then:

- The pill is in the tab order, after the nav tabs and before the "needs a look" chip.
- Space or Enter opens the menu (or, on the focus-unknown "?" button, opens Settings, as today).
- Arrow keys move through menu items. Enter chooses. Escape closes the menu or the confirm.
- iced 0.14 has no accessibility API (`nav.rs:423`). Put the full state in the visible words, and set the tooltip (and any widget label the toolkit allows) to the long words plus the time, plus "Menu: start or stop the tracker" when the menu can open. Colour is never the only cue.
- Confirm buttons: Keep tracking is the default focus. Stop tracker / Restart tracker is reached by Tab.

### 1.7 Starting, Restarting, Stopping, Error

`daemon_busy` is only a boolean today (`app.rs:1135`). Store the verb in flight so the pill can say Starting..., Restarting... or Stopping.... No backend change.

While busy:

- Ignore further Start, Stop and Restart clicks (`app.rs:1132` already does this).
- Use an amber or neutral pulsing dot, not Ready's green, for Starting, Restarting and Stopping. The pulse is a soft halo around the dot. If the user prefers reduced motion, show the amber or neutral dot with no pulse and no halo.
- The pill keeps its resting border so it still reads as a control, but the menu stays closed.
- Do not change nav, and do not block the rest of the UI.

On `DaemonDone`:

- Success: clear the error, refresh the daemon view, show Ready, Live, or Stopped as appropriate. A short toast may still say "Tracker started" / "Tracker stopped" / "Tracker restarted".
- Failure: enter Error, keep the error string for the tooltip and the menu (the reason line plus the Troubleshooting link), and keep today's toast. Retry runs the same verb that failed, or Start if the process is down.

### 1.8 What needs UI state, not a backend change

- The verb in flight (Starting / Restarting / Stopping).
- The last start/restart error, so Error can persist after the toast is dismissed.
- The confirm dialog.
- The pill menu.

`run_verb`, systemd, and pid identity checks stay as they are.

## 2. Pinned Settings save / restart bar

### 2.1 Purpose

The 0.6 page has no Save, and switches already apply at once. Do not bring back a Save for those controls.

The pinned bar appears only on the Settings tab, fixed to the bottom of the window, above any floating toast or GUI-restart prompt (`chrome.rs` `floating_prompts`). It appears only when something is pending, and it replaces both the always-on Save settings control in the stale shots and the Restart tracker / "N changes waiting" cluster on the Settings strip.

Two kinds of pending, counted separately:

1. Unsaved drafts: typed fields not yet committed (Check every, Wait after detection, Website URL, Account token, Shortcut). Session window and switches are not drafts; they are already written.
2. After restart: committed (or already in the file) keys the running tracker has not loaded (`changes_waiting`).

### 2.2 When it shows, and the count

Hide the bar when both counts are zero, and in demo mode (demo cannot write or restart).

Copy, left side:

- Only drafts, one: "1 unsaved change"
- Only drafts, several: "N unsaved changes"
- Only After restart, one: "1 change waiting"
- Only After restart, several: "N changes waiting"
- Both: "N unsaved changes, M waiting for restart"

The count is the existing `changes_waiting` plus the number of dirty draft groups (timings count as one if either timing field differs, sync as one, shortcut as one). Do not double-count a draft that has not been written.

Right side buttons, in this order: Discard (only when there are drafts), then the primary action.

### 2.3 Discard and Save

Discard (ghost button):

- Reverts timings, sync fields and the shortcut field (and the shortcut checkbox, if it was changed but not committed) to the last saved values.
- Does not revert already written switches, chips or the session window.
- Does not restart the tracker.
- If only After restart changes remain, the bar switches to the "N changes waiting" form. If nothing remains, the bar hides.

Save (accent button), used when there are drafts and no After restart work, and the drafts do not themselves need a tracker restart (today every timings and sync commit does need one if the tracker is running; the shortcut does not):

- Commits the open drafts with the same paths as Enter / Apply (`Commit::Timings`, `Commit::Sync`, `Commit::Shortcut`).
- Runs the same validation: unsafe sync URL blocks the save, classic OCR rules do not apply here, empty poll stays auto.
- On success, if nothing needs a restart, hide the bar. Stay quiet (the 0.6 page already stays quiet on a successful apply) or use a short toast only for sync env exceptions that today's save already toasts.
- The shortcut commit applies at once to this app, no tracker restart.

### 2.4 Save and restart

The primary button is labelled "Save and restart" when:

- There are drafts that the tracker reads at start (timings or sync), or
- There are drafts plus After restart changes, or
- There are no drafts, the tracker is running, and After restart changes are waiting. In this last case the button can say "Restart tracker" instead, because nothing is unsaved. Prefer "Restart tracker" so it does not sound like a second save.

If the tracker is stopped and drafts need to be written, the button is "Save and start". If it is stopped and there are no drafts, the pill (not this bar) is the way to start; the bar hides.

Save and restart / Save and start:

1. Validate and commit drafts. If validation fails, stay on the bar (section 2.6) and do not restart.
2. If a match is in progress, run the pill confirm from 1.4 before calling `run_verb`. Cancel leaves the drafts saved (they are already on disk after step 1) and the bar in the After restart form.
3. Call `run_verb(Restart)` or `run_verb(Start)`.
4. The pill shows Restarting… or Starting…. On success the waiting count clears (`restart_started` / `observe_tracker` already do this) and the bar hides.

Restart from the pill menu uses the same sequence.

### 2.5 Leaving with unsaved changes

If drafts are open and the user:

- Navigates to another tab,
- Opens a dialog that would cover Settings (Uninstall, setup guide) after leaving the fields,
- Hides the window, or
- Quits,

show a small confirm: "You have unsaved changes" with Discard and leave, Save (or Save and restart if 2.4 applies), and Cancel. Default is Cancel. Escape is Cancel.

Already written After restart changes do not block leaving. The next visit to Settings shows the waiting bar again, and the pill menu still offers Restart tracker.

Closing the Settings fold or scrolling does not count as leaving.

### 2.6 Error on save

If the file does not parse, changed on disk, or the URL is unsafe:

- Do not restart.
- Keep the bar open.
- Show the existing messages on the bar, in danger or warn colour: `SETTINGS_SAVE_REFUSED`, `PAGE_RELOADED_FROM_DISK`, or the sync URL problem line.
- For a reload from disk, the fields already update to the file; Discard would be a no-op and can hide if nothing remains dirty.
- Retry is the same Save or Save and restart button, not a new control.

Daemon errors after a successful save belong on the pill (Error) and a toast, not on this bar. The waiting count may still be accurate if the restart did not run.

### 2.7 Layout

Bar height about 56 px, full width of the window, `SURFACE` fill, a 1 px `BORDER` along the top, 24 px side padding (the page gutter at 1280 and 1000). Content maxes at the Settings page width (`PAGE_MAX` in two columns, `SINGLE_COLUMN_MAX` in one) and stays left-aligned with the page in two columns, centred in one column.

At 1000 px, if the copy plus buttons cannot fit on one line, wrap the buttons onto a second line, end-aligned. Total bar height may grow to two lines. Do not cover the last Settings row: the page gains bottom padding equal to the bar height while the bar is visible.

Z order: below any modal (confirm, uninstall, setup) and below the report dialog; above the page; the existing floating toast stays above the bar if both are visible, or the toast is reserved until the bar hides. Prefer not to show both: a save error lives on the bar.

### 2.8 Keyboard

- When the bar appears, do not steal focus from a field the user is typing in.
- Tab from the last Settings control moves to Discard, then to the primary button.
- Enter in a dirty timings or shortcut field keeps today's commit, then the bar updates (it may become "Restart tracker" or hide).
- Escape does not discard. Discard is an explicit click or Space on that button.

## 3. Out of scope

- Classic layout Save footer, unless it is later aligned in a follow-up.
- A global keyboard shortcut for Start or Stop.
- Changing how `run_verb` talks to systemd.
- Auto-restarting the tracker on every After restart switch (the bar and the pill remain the explicit action, as today).

## Switch control

Every on/off setting uses Design System's switch (`/workspace/scuffed/tracker-design/switch-spec.md`, state sheet `switch-spec.png`). No checkboxes and no radio-like dots for on/off. The Reader radio list and the Auto/Off segmented focus-gate control stay as they are.

- Track 40 by 24 px, corner radius 12 px. Thumb 20 by 20 px, 2 px inset, 16 px travel.
- Label (and any helper or tag) on the left, switch on the right, 16 px gap. No On/Off word beside the track. The whole setting row is the click and keyboard target.
- Off: `SURFACE` fill, 2 px `TEXT_3` outline, `TEXT` thumb.
- On: `ACCENT` fill, no outline, `TEXT` thumb.
- Focus: 2 px `ACCENT` ring, 2 px outside the track.
- Disabled: the live On or Off colours at 40% on the parent surface. Still shows the real state.

The Companion keyboard shortcut is this switch, not a checkbox.

## Settings page notes

- The Wayland companion failure is a notice inside the Companion card: an icon, a tinted `ACCENT_SOFT` fill, and no input-style border.
- Closed Advanced rows use the same chevron icon as the pill, centred vertically, not a ">" character.
- A closed "Capture and reading" summary is one line, for example "Automatic on first monitor, new reader with OCR backup".
- "After restart" tags appear only on rows or folds that actually have a restart-needed change. The pinned bar carries the waiting count, so the tags do not repeat it on every Advanced row.

