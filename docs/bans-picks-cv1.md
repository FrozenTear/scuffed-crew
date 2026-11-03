# Bans and picks reader (cv-v1)

`crates/stat-tracker/src/shadow/bans_picks/` reads the ban results, hero
select and ban vote screens by icon template matching. It sits next to the
Tesseract reader `ocr-v1`, which still reads everything it read before. Nothing in the tracker
calls it yet: the capture wiring and the server field come in separate
changes.

## Switch

On by default. `STAT_TRACKER_BANS_PICKS_RECOGNIZER=off` (or `0`, `false`,
`no`) turns it off (`BansPicksSetting`). Anything else, or nothing, is on.
Without the templates below it reads nothing. There is no config.toml key yet.

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
| Ban results | `read_ban_results` | four bans with team side, the lobby ban when the lobby layout shows |
| Hero select | `read_hero_select` | six own-team picks and the own ring colour |
| Ban vote | `read_ban_vote` | six own-team picks |

Every value is `{value, confidence}`. A position that cannot be read is
`null`, never a guess. The "no preferred hero" icon reads `no_preference`
(only when `no-preference.png` is installed). The combined `read` returns
`bans`, `lobby_ban`, `picks` and `bans_picks_recognizer: "cv-v1"`.

## Rules for the caller

- Pass the LAST frame of the hero select and ban vote screens. The first
  frame is mid fade or under a wipe band, and a first frame read gave false
  accepts on rows nobody could read.
- Only 2560x1440 frames are read. Any other size returns `None`.
- 1080p, other UI scales, colourblind filters and non-English clients are not
  measured.

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
