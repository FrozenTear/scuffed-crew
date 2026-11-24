# Companion pages

Short spec for the 0.7 multi-page companion. Overlay size follows the current code (`overlay.rs`): 360 px wide, 24 px screen margins, exclusive zone 0, click-through, keyboard none, height to content (clamped 200 to 900). Design only. No em or en dashes. Call the game Overwatch. Copy must never say "should ban" or "best ban".

This stays consistent with the 779 When banned view: all read bans count (own and enemy), "Your ban" is text plus tint, rates are historical hints, and the all-maps floor in the tracker stays 10 decided games. The companion uses a lower per-map floor, below.

## 1. Pages

Pages build on the swappable panels in the companion design notes, section 1. They do not replace that idea: today's tonight card is page 1, later pages are the other panels, and the "next panel" shortcut becomes next page. Automatic switching by game phase is the same framework.

| Page | When | What |
| --- | --- | --- |
| Tonight (stream page) | Default, and whenever the match is on | The current companion, unchanged in content except the tonight result strip: each square has a W or L letter inside it (or D for a draw), so the result is not colour alone. Season line, last game (map, hero, win or loss, stats), tonight's results, top heroes tonight. No picks, bans, hints or death rates. |
| Map vote | During the vote, if the three maps were read | The three candidate maps, each with the player's best heroes on that map, as coloured pills (section 6). If the vote read fails, stay on Tonight. |
| Picks and bans | After the map is locked, through pick and ban | The locked map and mode, best picks on that map, and the ban hint card (section 4). |
| Matchup | After the match is going, if the player has left Tonight | The locked map, win rate with the current (or most played) hero on that map, and the same ban hints as a compact strip. Against-enemy and teammate-pairing sections stay hidden until that data exists (issues 780 and 782, 0.8). Do not show placeholder or "ship in 0.8" roadmap copy on the card; that note belongs only in this spec. |
| Tip | Before a match or between games only | The pre-match map tip (section 5). Never on Tonight. Not one of the in-match dots. |

## 2. Stream capture

The notes treat Tonight as a page that can be left up so viewers see the history. The companion is a Wayland layer-shell overlay, not part of the Overwatch window, so a game-window capture does not include it, and a display or output capture does. Design as if Tonight can be on stream. Nothing hint-related may appear there, including by an automatic page change.

## 3. Navigation

Clicks do not work: the overlay is click-through and takes no keyboard focus, same as today. Page changes use global shortcuts, in the same family as the existing hide bind (Super+Shift+C).

Three options:

1. **Automatic at map vote and pick/ban, with a manual override.** The page follows the stage, and next/prev still work. Fast return to Tonight is only manual (a safe-page key or stepping back). Risk: if the display is captured, hint pages can stay on stream into the match.
2. **Manual only.** Next-page and safe-page keys, no stage switching. Safest for stream. Easy to miss the vote or ban hint in the moment.
3. **Automatic with auto-return (recommended).** Auto-advance to Map vote when the three maps are read, then to Picks and bans when the map locks. When the match starts (loading, or the first live Tab of the game), auto-return to Tonight. Next/prev still work. A single safe-page key jumps to Tonight and holds there, suppressing auto-advance, until the next next-page press or the hold is cleared. Hide (the current shortcut) still hides the whole overlay.

Recommended, and the 0.7 default: option 3. Auto-advance is on by default, because it puts the right hint up during vote and ban. Auto-return at match start is always on, so Tonight is forced once the match is on (the moment most likely to be on stream). Auto-advance can still be turned off in Settings ("Hold the stream page") for a session that must never leave Tonight.

The safe-page control is a new shortcut, Super+Shift+T, not a hold or double-tap of the hide bind. T stands for Tonight. It uses the same modifier family as hide (Super+Shift+C), does not clash with that bind, and is remappable in Settings in the same way. The key jumps to Tonight and holds there, suppressing auto-advance, until the next next-page press or the hold is cleared. Hide still hides the whole overlay. If the compositor already uses Super+Shift+T, pick another key in Settings; do not reuse C, and do not fold this into the hide bind.

How the active page is shown, on every in-match page: four dots plus the current page name in text ("Tonight", "Map vote", "Picks and bans", "Matchup"). Colour is never the only cue. The Tonight pip is labelled so it is obviously the stream page. When the safe-page hold is on, a "Held on the stream page" tag sits next to the name, and the footer also shows "Safe page with Super+Shift+T". The tip page shows the word "Tip" and a Tonight button, not the four in-match dots. The map mode (for example Escort) is plain muted text, not a pill, so Tonight is the only button.

## 4. Ban hint card (Picks and bans)

Shown once the map is locked. Width of the card is the overlay (360 px, 12 px inset). Three rows, maybe a fourth if height allows. Sort by win rate among heroes that meet the floor, then by more decided games, then name. No "best ban" label and no Best / Worst rail.

**Minimum: 5 decided games on that map.** Reasoning: the per-map pool is thin (up to four bans per game, spread across many heroes and maps), so the tracker's 10 is too high here. 3 decided games can still swing 0% or 100% after one session and read as a strong hint. 5 is reachable on a map that comes up often, and is the lowest count that still feels like more than a single night. The 779 all-maps view keeps 10.

All read bans count, own and enemy, same as 779. Unread slots are ignored. `lobby_ban` counts when it is a known hero and not already in the four slots, with no team tag. Votes are not results.

Each row: hero, record, and a rate only from 5 decided games on this map. A "Your ban" tag (words plus accent tint) and the own-team count, never colour alone. Under 5: no percent, "Too few games on this map (n of 5)", and every under-min row also shows an all-maps line. If that hero has 10 decided games all-maps, show that rate as "All maps 57%". If all-maps is under 10 as well, say "All maps: too few games". Do not hide the row when this map has any read ban. The same "Too few games on this map (n of 5)" form is used on Map vote, including when a candidate map has no third hero at the floor.

Copy is a hint: "Past results when this hero was banned on Circuit Royale, not a suggestion of who to ban."

**Map context without clutter.** The locked map is the card title, so every row is already "banned on this map." Do not repeat the map on each row. Under the list, keep the one-line cue for the top row (or the first hero that has a clear other map): "Elsewhere: Kiriko is highest on Havana." A "More maps" disclosure under that opens a short list on this card in 0.7, not a fifth page. The list is for that hero: the top 3 to 5 maps, each with its rate or "Too few games on this map (n of 5)", and the locked map marked with a "This map" tag, never colour alone.

Clicks cannot open the list (the overlay is click-through). On Picks and bans, the next-page key opens the list first; a further next-page press goes on to Matchup. Prev closes the list if it is open. That avoids a third global bind. Hide and safe-page are unchanged.

Empty: "No ban reads on this map yet." If the reader is off, say so in that line.

## 5. Pre-match tip

A later, between-games page, never Tonight. It uses only the player's own history on the locked or last-voted map. The death-rate sentence waits on Blizzard's OK and is an optional slot: design and ship the ban and play lines so they stand without it. Draw both states.

Full example, with the slot: "Circuit Royale. On this map you have a high death rate. Be careful. You play best when you ban Kiriko, Ana and Widowmaker, and play Widowmaker, Ashe and Cassidy."

Without the slot: "Circuit Royale. You play best when you ban Kiriko, Ana and Widowmaker, and play Widowmaker, Ashe and Cassidy."

If there is too little data for a side, drop that clause rather than fill it. Ban and play each need 5 decided games on this map (the same floor as the card). Both missing: "Circuit Royale. Not enough games on this map yet for a ban or play hint." Only play: "Circuit Royale. You play best when you play Widowmaker, Ashe and Cassidy. Not enough games on this map yet for a ban hint." And the other way around. Never invent heroes, and never say "should ban." The footer has a Tonight button; the map mode is plain muted text, not a pill.


## 6. Map vote

The map vote page is pills plus colour. Each hero sits in a compact chip that wraps onto a second row. A short-name map is used first (Widowmaker to Widow, Reinhardt to Rein, Junker Queen to Queen, Soldier: 76 to Soldier, Wrecking Ball to Ball). Every other official name is used in full. The short-name map plus wrapping is enough that no name truncates at 360 px, so there is no cut-off fallback. The overlay is click-through, so there is no hover. The full name goes in the accessible name only.

Under-min pills stay muted and uncoloured, with no percent: "Sojourn  3 of 5", and the full "Too few games on this map (n of 5)" line in the accessible name. "Your ban" stays a text tag on the ban card, not a colour-only cue. Hint wording is unchanged: past results, not a suggestion.

Colour marks a high or low rate on top of the percent, which always stays visible, plus an up or down mark, so meaning never relies on colour alone. Thresholds: 55% or more is high (green), 45% or less is low (red), and 46% to 54% is neutral, with no tint and no mark. The band sits five points off 50%, so a coin-flip rate (and this player's all-time 47% in the frames) stays neutral, and 55% is a clear edge rather than a single extra win. Under-min rows never take colour.

Tints are a 16% mix of `OK` or `DANGER` on `SURFACE`. `TEXT` on those fills is above 4.5:1 (about 10.8:1 on the green tint and 12.2:1 on the red). The percent and mark use `TEXT` on a tinted pill, not green or pink on the tint. The sample set includes a low example (Ana 42%, down mark, red tint) so both ends show. A live card can still look very green when most rates are high; that is acceptable if neutral and under-min stay visually quiet.

**Considered.** Plain text, pills without colour, and colour on plain text were compared on sheet 25d. Plain text keeps density but runs long names together. Pills alone fix the names but are less glanceable. Colour on plain text shows high and low, with the percent and an up or down mark, but long names stay hard to read.

