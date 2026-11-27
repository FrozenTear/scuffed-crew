# Settings switch

One switch for every on/off setting in the Scuffed Crew stat tracker. No checkboxes. No radio-like round dots. FrozenTear decided this in the Design room on 19 November 2026 (Europe/Oslo).

The widget is dark only. Colours come only from `crates/stat-tracker-ui/src/theme.rs` on `wip/0.6` at `9ec2bb662cbd3894d4e399006d3958351474e820`. UX Spec draws frames from this spec. Scuffed Tracker builds it. This note does not change the repo.

The shared control already exists: `settings_rows.rs` `switch` (line 187), styled by `switch_style` (line 210) and `theme::companion_toggle` (theme.rs line 372). It is an iced 0.14 `toggler` at size 20 (a 40 by 20 px track) with an "On" or "Off" word on the right of the knob. This spec replaces that look, drops the On/Off word, and requires keyboard use that the built-in toggler cannot do.

Redesign shots in `crates/stat-tracker-ui/docs/redesign-shots/` (Settings in 04-07, 09, 11 and 12) still show the older controls: a switch labelled "Companion showing", a checkbox under Companion, and round dots under Website account and Updates. On current `wip/0.6` those dots are already `switch` / `edit_switch` calls, except the Companion shortcut, which is still a checkbox. The shots are stale. Build against the file:line list below, not the shots.

## Places to replace

Paths are under `crates/stat-tracker-ui/src/`.

### Wrong control: change to this switch

0.6 Settings page:

- `settings_cards.rs:96` Keyboard shortcut to hide or show the overlay (checkbox)

Classic layout (`settings.rs`, still shipped):

- `settings.rs:1354` Keyboard shortcut to hide or show the overlay
- `settings.rs:1430` Watch for match start and end
- `settings.rs:1512` Use classic OCR (Tesseract)
- `settings.rs:1589` Get alpha updates
- `settings.rs:1604` New reader
- `settings.rs:1618` Extra number reader (test)
- `settings.rs:1654` Save debug images from each capture

`form_widgets.rs:89` `checkbox_style` is the look those checkboxes use. It can stay for Uninstall and Report a problem, which are confirmations, not settings.

### Already a switch: restyle through the shared widget

Update `settings_rows.rs:187` `switch`, `settings_rows.rs:210` `switch_style`, and `theme.rs:372` `companion_toggle` (or replace `companion_toggle` with the style in this spec). Drop the On/Off word beside the track. Every call site below then picks up the new look.

- `settings_cards.rs:88` Show Companion during games (shots still say "Companion showing")
- `settings_cards.rs:284` Get alpha updates
- `settings_cards.rs:332` Start on login
- `settings_cards.rs:342` Watch for match start and end
- `settings_cards.rs:482` Read bans and picks (`edit_switch`)
- `settings_cards.rs:540` Save debug images from each capture
- `settings_cards.rs:551` Save screens around each game (`edit_switch`)
- `settings_cards.rs:599` Extra number reader (test)
- `settings_keys.rs:195` and `:197` Upload games after each match (`edit_switch`, or a locked `switch` when the row cannot be changed)
- `settings_keys.rs:250` Read my rank after Comp games (`edit_switch`)
- `settings_keys.rs:271` Share stat pictures (`edit_switch`)
- `settings_keys.rs:288` Report pictures (`edit_switch`)
- `capture_reading.rs:550` Name reader

`settings_keys.rs:51` `edit_switch` is only a wrapper around `switch`. Change it only if the dirty-and-save behaviour needs to go through this helper.

### Leave these as they are

These are not on/off settings, so they do not become switches:

- Reader (`capture_reading.rs:534` `radio_list`): three choices (New reader only, New reader with OCR backup, Classic OCR only).
- Only capture while Overwatch is in front (`settings_keys.rs:324` Auto / Off segmented). Shot 07 still shows this as a round dot. Current code is a two-option mode control, and it should stay that way.
- Uninstall "delete data" and the Report a problem checkboxes.

## Anatomy and size

- Track: 40 px wide, 24 px tall, corner radius 12 px (a pill).
- Thumb: 20 by 20 px circle, 2 px inset from the track on every side.
- Thumb travel: 16 px (from x = 2 to x = 18).
- Hit target: at least 24 px tall. The track is already 24 px tall. The whole setting row is the click and keyboard target, so the usable area is taller than the track (current row pad is 12 px above and below, `ROW_PAD_Y` in `settings_rows.rs`).
- Gap between the label block and the switch: 16 px (the current `CONTROL_GAP`).
- Label on the left, switch right-aligned in the row. Title is 14 px Urbanist SemiBold in TEXT. Optional one-line helper under the title, 13 px Urbanist Medium in TEXT_2. Do not put an "On" or "Off" word next to the track. The thumb position and the track colour carry the state.
- Tags such as "After restart" or "New in 0.6" stay on the left with the title, as they do today.

## Colours

Contrasts are WCAG 2 relative luminance ratios, computed by script, not guessed. Non-text UI (thumb against track, track or outline against the surface, focus ring against the surface) must meet 3:1 or better. No new token is needed.

Parent surfaces are SURFACE (`#1C1C25`) for settings cards and SURFACE_2 (`#23232E`) for inner tiles.

| State | Track fill | Track outline | Thumb | Ring |
| --- | --- | --- | --- | --- |
| Off | SURFACE `#1C1C25` | 2 px TEXT_3 `#8D8DA0` | TEXT `#F2F2F7` | none |
| On | ACCENT `#8F73FF` | none | TEXT `#F2F2F7` | none |
| Hover, off | SURFACE `#1C1C25` | 2 px TEXT_2 `#C9C9D6` | TEXT `#F2F2F7` | none |
| Hover, on | ACCENT `#8F73FF` | 2 px TEXT `#F2F2F7` | TEXT `#F2F2F7` | none |
| Focus, off or on | same as Off or On | same as Off or On | TEXT `#F2F2F7` | 2 px ACCENT `#8F73FF`, 2 px outside the track |
| Disabled | the live Off or On colours, composited at 40% onto the parent surface | same treatment | same treatment | none |

Disabled is only for a control that cannot be changed (demo mode, an env lock, no login service for Start on login, or a busy daemon). It still shows the real on or off state. Never use the disabled look to mean Off.

### Contrast ratios

| Pair | Ratio | Role |
| --- | --- | --- |
| TEXT `#F2F2F7` on ACCENT `#8F73FF` | 3.10:1 | On thumb against on track |
| TEXT `#F2F2F7` on SURFACE `#1C1C25` | 15.15:1 | Off thumb against off track (and against the card) |
| TEXT `#F2F2F7` on SURFACE_2 `#23232E` | 13.93:1 | Off thumb if the row sits on an inner tile |
| ACCENT `#8F73FF` on SURFACE `#1C1C25` | 4.88:1 | On track, and the focus ring, against the card |
| ACCENT `#8F73FF` on SURFACE_2 `#23232E` | 4.49:1 | On track, and the focus ring, against an inner tile |
| TEXT_3 `#8D8DA0` on SURFACE `#1C1C25` | 5.19:1 | Off outline against the card |
| TEXT_3 `#8D8DA0` on SURFACE_2 `#23232E` | 4.77:1 | Off outline against an inner tile |
| TEXT_2 `#C9C9D6` on SURFACE `#1C1C25` | 10.31:1 | Off hover outline against the card |
| TEXT_2 `#C9C9D6` on SURFACE_2 `#23232E` | 9.48:1 | Off hover outline against an inner tile |

Today's off track uses a SURFACE fill and a 1 px BORDER (`#2A2A36`) outline. BORDER against SURFACE is 1.19:1, which fails 3:1, so that outline is not enough. A filled off track in BG, SURFACE_2 or BORDER also fails against SURFACE (about 1.1:1). A filled off track in TEXT_3 would pass against SURFACE (5.19:1) but the TEXT thumb against TEXT_3 is 2.92:1, just under 3:1. That is why Off is a SURFACE fill with a 2 px TEXT_3 outline, not a grey fill.

The 2 px TEXT outline on Hover-on sits on ACCENT, which is the same 3.10:1 pair as the thumb. Tight, but it passes. The focus ring is ACCENT sitting on SURFACE or SURFACE_2, not on the ACCENT track: keep a 2 px gap so the ring stays visible when the switch is on.

Disabled colours are exempt from the 3:1 non-text rule. Composite the live Off or On drawing at 40% alpha onto the parent surface, and use TEXT_3 for the label and helper while it is locked.

## Behaviour

- The whole setting row is clickable, not only the 40 by 24 px track.
- The row (or the switch) can take keyboard focus. Space toggles it.
- Flipping a switch marks the form dirty and waits for the pinned save bar (UX Spec's rule). Nothing applies at the moment you flip it, including rows that today write through `edit_switch` or apply at once (Start on login is one of those).
- Discard on the save bar puts the switch back to the saved value.
- The only motion is a 120 ms ease-out slide of the thumb. Track and outline colours may change over that same 120 ms. No bounce, no extra fade, no scale.

## Iced 0.14 note

The built-in `toggler` can take the colours and a 24 px height, but it cannot meet this spec on its own. Use a small custom widget.

Limits of `iced_widget` 0.14.2 `toggler`:

- Pointer and touch only. No focus and no Space. The 0.6 page already notes this at `settings_cards.rs:496`.
- No focus ring in `toggler::Style`.
- The thumb jumps. There is no 120 ms slide.
- `size(n)` always lays out a track of `2n` by `n`. `size(24)` would be 48 by 24, not 40 by 24.
- If a label is set, it is drawn to the right of the track. Leave the toggler unlabelled and keep the label on the left of the row.

A style closure can still describe the colours, even on a custom widget. Outline only, not the widget itself:

```rust
fn switch_style(status: SwitchStatus) -> SwitchStyle {
    // SwitchStatus: { on, hovered, focused, disabled }
    // Off: SURFACE fill, 2px TEXT_3 outline, TEXT thumb.
    // On: ACCENT fill, no outline, TEXT thumb.
    // Hover off: 2px TEXT_2 outline. Hover on: 2px TEXT outline.
    // Focus: 2px ACCENT ring, 2px outside the track, on SURFACE and SURFACE_2.
    // Disabled: composite the live colours at 40% onto the parent surface.
}
```

Wire that style into the custom widget, and use the widget from `settings_rows::switch` so every call site stays one line. Classic layout rows should use the same widget, with the label on the left and the switch on the right, so the two Settings pages feel the same.
