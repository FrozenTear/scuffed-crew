# Bans and picks reader (cv-v1)

`crates/stat-tracker/src/shadow/bans_picks/` reads the ban results, hero
select and ban vote screens by icon template matching. It sits next to the
Tesseract reader `ocr-v1`, which still reads everything it read before. The
tracker feeds it the last frame of each game's ban results, hero select and
ban vote screens (see [Capture wiring](#capture-wiring)). The server field is a
separate change.

## Switch

On by default. Settings > Troubleshooting has **Read bans and picks**
(`bans_picks_reader` in `config.toml`). `STAT_TRACKER_BANS_PICKS_RECOGNIZER=off`
(or `0`, `false`, `no`) turns it off whatever the setting says
(`BansPicksSetting`). Settings then shows that row as Off whatever the
setting says, with the switch locked and the line "Set outside the app:
STAT_TRACKER_BANS_PICKS_RECOGNIZER" under it. Anything else, or nothing,
leaves the setting in charge, and the row shows the setting and stays
unlocked. Off means the reader
is never loaded or called and no warning shows. Without the templates below
it reads nothing.

## In the upload

Each uploaded match gets an optional `bans_picks` object, plain values only:

- `bans`: up to 4 of `{hero, team}`, `team` is `own`, `enemy` or null (the side
  that made the ban)
- `lobby_ban`: hero key or null
- `picks`: up to 6, own team, the local player first
- `ban_vote`: up to 6, same values as `picks`
- `recognizer`: `cv-v1`

A hero is a pack key or null. `picks` and `ban_vote` may also hold
`no_preference` (the reader's `no-preference` template name is sent with the
underscore). Confidence, score, margin and layout are not sent. A game with no
value read omits the key, so an older server, which ignores fields it does not
know, takes the upload as before. A server needs the site change that accepts
`bans_picks` to store it.

## Capture wiring

While a game is being captured the daemon keeps the last ban-results frame, the
last hero-select frame and the last ban-vote frame it saw. When the game gets
its first record, it reads those frames once and stores the result as one JSON
string on the game record (`bans_picks`, with `bans_picks_recognizer: "cv-v1"`).
A game that has no such screens, or whose screens are older than the game by
more than the 30 minute binding window, gets nothing. The frames are held in
memory only (about 3 to 4 full frames), and the reader runs even when
`debug_save_pregame_screens` is off. The upload carries the values of it as an
optional `bans_picks` object on the match (see the next section). The stored
JSON, with its confidences, stays on this computer.

The game is always recorded. When the reader cannot run, `bans_picks` stays
empty and the app status shows a warning:

| Case | Warning says |
|---|---|
| `bans-picks-v1.tar` not installed | the pack is not installed yet, the tracker fetches it in the background |
| not installed, install not linked to a site account | the same, and to link a site account in Settings, since the pack routes need the sync token |
| `Reader::load` fails | the pack did not load, with the reason, and to reinstall the pack or turn the setting off |
| a read fails | the reader failed on this game's screens |
| frame is not 2560x1440 | the game ran at that size and the reader needs 2560x1440 |

A good read clears the warning.

## What it needs

The hero art is the game's, so it is never committed. The reader loads
`<data_dir>/templates/bans-picks-v1/<key>.png` for all 54 hero keys (the stems
of `scuffed_types::HERO_NAMES`, for example `wrecking-ball.png`), each 100 wide
by 96 high, cut from the hero tile at 2560x1440. An optional
`no-preference.png` of the same size is the icon shown for a player with no
preferred hero. Without all 54 hero files there is no reader.

## What it reads

| Screen | Call | Output |
|---|---|---|
| Ban results | `read_ban_results_rings` | four bans with team side, the lobby ban when the lobby layout shows |
| Hero select | `read_hero_select` | six own-team picks and the own ring colour |
| Ban vote | `read_ban_vote` | the local player's three vote slots (hero, skipped, empty) and the TEAM BAN SCORE strip (three heroes with vote counts) |

Every value is `{value, confidence}`. A position that cannot be read is
`null`, never a guess. The "no preferred hero" icon reads `no_preference`
(only when `no-preference.png` is installed). The combined `read` returns
`bans`, `lobby_ban`, `picks` and `bans_picks_recognizer: "cv-v1"`.

## Ban vote screen

The left roster column of the ban vote screen shows each player's
preferred heroes, not their votes, so it is never read on this screen
(alpha.2.1 and older did, and stored those portraits as votes). The reader
reads two parts instead, each `None` when the frame does not look like it:

- The local player's three vote slots, the small boxes under their name
  (outer borders x 193..243, 255..304 and 316..366, y 456..506). A slot is
  `hero` (red border, hero art, matched at 0.465 of the template size on
  the full-size gradient plane), `skipped` (a circle with a slash), `empty`
  (a flat inside) or unread. An `empty` slot means nothing was in it on
  that frame, which can be a vote cast after the last frame, so the app
  shows it as "Not seen" and never as a missing vote. The layout check
  needs a red or grey border at each box and nothing border-like 6 pixels
  outside it.
- The TEAM BAN SCORE strip: the red title, three hero boxes (x 1154..1229,
  1242..1317 and 1330..1405, y 97..171, matched at 0.70) and the vote
  count under each, read with ten digit grids kept in code (no OCR). An
  empty box has a grey dash and reads as `empty`.

Teammates' vote slots sit at the same x in their rows and are not read
yet; the stored shape has a `voter` index so they can be added later.

The game record keeps `ban_vote` as the local player's three votes in slot
order (a hero, or null for skipped, empty or unread), plus `ban_votes`
(every slot with its state) and `ban_score` (the strip). The upload sends
`ban_vote` as before and `ban_votes` and `ban_score` as optional plain
values, which a server that does not know them ignores.

The tracker always writes `ban_votes` (an array, or an explicit null when
the vote screen was not read) whenever the bans and picks reader stores a
result, and `ban_score` the same way. The app reads our votes only from
`ban_votes`. A game with no `ban_votes` key was stored by alpha.2.1 or
older, where `ban_vote` holds the roster preferred heroes and not votes, so
the app shows its votes as not read and never falls back to that list.
`no_preference` in `ban_vote` is legacy data only.

## Team side

The game lets a player pick the team colours (blue and red, yellow and
purple, and others), so the reader does not assume any colour. The four ban
bars always form two groups, the outer pair and the middle pair, one group per
team. The reader sorts the bars into two groups by how far apart their hues
are (the OpenCV hue scale, 0 to 180). Then the own-team ring colour decides
which group is own: the group whose hue the ring is clearly nearer to.

The ring comes from the hero select frame (last frame, else first) and from
the ban vote frame, which shows the same ring at the same place. The daemon
gives both, in that order. A ring is skipped when it is grey or dark (a fade
frame has a meaningless hue), when it is about as far from both groups, or
when it is far from both. Usable rings must agree, or every team is `null`.
Every team is also `null` when the bars are one colour, two colours that are
too near, or three colours, and one bar is `null` when it is grey. If both
bars of one team are grey (that team banned nothing), there is only one
colour group, so every team is `null`, even for the coloured pair. The
previous rule still marked that pair from the ring. Three bars of one colour
against one is `null` too, since a team has two bars.

The ban vote frame's ring is always given to the reader, even on a vote frame
that shows no ring (the scan then returns whatever pixel it finds). A usable
select ring and a junk vote ring that points the other way disagree, and
every team is `null`. That is kept on purpose: a doubtful read gives `null`,
not a side.

## Rules for the caller

- Pass the LAST frame of the hero select and ban vote screens. The first
  frame is mid fade or under a wipe band, and a first frame read gave false
  accepts on rows nobody could read.
- Only 2560x1440 frames are read. Any other size returns `None`.
- 1080p, other UI scales and non-English clients are not measured. Team
  side was measured on the default colours and on yellow and purple only; a
  colourblind filter that gives other colour pairs reads by the same rule, but
  no frame of one was measured.

## What was measured

See the pull request that added it. The labels were made by Vision from the
saved frames of one player (FrozenTear) on one day range, so they are not
blind. 25 of the 54 heroes were never banned in that data.

## Risks

`TemplateSet::load` needs a template for every hero in `HERO_NAMES`. When a new
hero is added to that list, the load fails and cv-v1 reads nothing for everyone
until the bans-picks pack is republished with that hero. Tracker shows the load
error as a visible warning and keeps recording. Loading only the heroes that are
present is left for a later PR, because it changes how the pack is checked.
