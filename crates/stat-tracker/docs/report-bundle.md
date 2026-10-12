# Tracker issue report bundles

This document is the contract. The tracker app, the API, and the site all build against it.

Revision 2 (2026-10-11) adds in-app sending with the tracker's sync token (section 8), the `crash` and `performance` categories with log-only bundles (sections 2 and 7), an optional `system` object (section 2), and `bundle_version` 2. It also fixes four places where revision 1 and the validator in `crates/types/src/stat_report.rs` disagreed (`team_size: null`, extra crop paths, the reports-off status codes, and the matcher id example). The server endpoints (#189) and the My reports pages (#192) are on main. The app's Report button builds against this revision.

Revision 3 (2026-10-12) adds `crops/outcome.png` and the first scoreboard allowlist. Revision 4 (2026-10-12) blanks the scoreboard with geometry measured on each board instead of fixed columns, and reads chat from the full frame (section 4). Revision 5 (2026-10-12, blank table 10) copies a portrait only when it matches the same row on an earlier capture of the game, and keeps portraits only at the 1440p and 1080p scales (section 4).

The desktop GUI is `scuffed-stat-tracker-ui` (the binary is still `stat-tracker-gui`). The daemon is `scuffed-stat-tracker`. Blanking, the zip, and the preview live on the member's PC, in that app, before anything is saved or uploaded.

## 1. Purpose

A member sends a report so we can fix a tracker problem. A game report contains a scrubbed log and debug crops from one game. A crash or performance report contains a scrubbed log and the `system` object, and no crops (section 7).

Consent is recorded in the manifest, in the `consent` object. Sending the report records consent to use it for the fix. Training is a separate flag in that same object. The training box is off by default, so `training` is false unless the member ticks it. A report recorded with `training` false is deleted after 30 days. A report recorded with `training` true may be kept for training until the member deletes it or withdraws that consent.

The flags are independent. The training flag does not change which files are in the zip. Leaving it off does not block the fix. The member can send a report with training left off, and can still include or exclude their own name and wrong glyphs under section 5. Those boxes are also off by default, and their flags are recorded in the same `consent` object.

## 2. Bundle

One report is one zip file. The zip has this layout and nothing else:

```text
manifest.json
log.txt
crops/*.png
```

Paths inside the zip use forward slashes. There is no absolute path, no home directory, and no player name in any path. The zip has no extra directory entries, no hidden OS files, and no file whose name ends in `.jpg` or `.jpeg`.

Every zip entry uses the fixed date 1980-01-01 00:00:00. That is the earliest time a zip entry can store, so the member's clock is not in the file. Entries have no comment and no extra fields. There is no Unix owner field, no extra timestamp, and nothing else beside the path, that fixed date, and the file bytes.

`manifest.json` is UTF-8 JSON with no byte order mark. `log.txt` is UTF-8. Every image is a PNG crop that already went through blanking, except the optional own-name crop and the optional glyph crops, which are cut under section 5 before the frame is blanked.

The app builds the zip in a temp directory, then reads that same zip back for the preview. Save report copies that zip. It does not build a second one with different settings.

### Manifest fields

`bundle_version` is the integer `2`. A reader accepts `1` and `2` and rejects any other version. Version 1 is revision 1 of this document: it has no `system` object and no `crash` or `performance` category. A version 1 manifest that carries either is invalid. The app writes `2`.

`app_version` is the tracker version string of the build that made the zip, from 1 to 32 characters.

`recognizers` has two ids:

- `matcher` is the `RECOGNIZER_ID` constant compiled into that build (`crates/stat-tracker/src/shadow/digits.rs`). On 2026-10-11 that constant is `cv-v5`. When the constant changes, the field reports the new id. It is not a second copy that someone updates by hand, and the server does not require a particular id.
- `ocr` is the string `ocr-v1`. That is the Tesseract read the tracker already stores.

`resolution` is the capture size in pixels, `width` and `height`, both positive integers. It is the full frame, not the 16:9 playfield inside it.

`ui_scale` is either `null` or a positive number up to 200. The unit is the game UI scale where 100 is the default. The tracker does not detect this today. If the member leaves it blank, the value is `null`. The app does not guess.

`reason` is the member's category plus free text:

- `category` is one of `wrong_stats`, `wrong_hero`, `wrong_map`, `wrong_mode`, `wrong_result`, `missed_game`, `crash`, `performance`, `other`. `crash` and `performance` are not about one game. They need `bundle_version` 2.
- `text` is a string of 0 to 500 characters. It may be empty. Before it is written into the manifest, it is scrubbed with the same rules as the log. The free-text box tells the member not to type other players' names (section 7).

`session_id` is the local session id, 1 to 64 characters. If the game has none, the value is `none`. A `crash` or `performance` report uses `none`.

`game` holds the match context:

- `map`, `mode`, and `result` are strings or `null`.
- `team_size` is `5`, `6`, or `null`. `null` means the app could not tell 5v5 from 6v6; such a board writes no scoreboard (section 4), so `null` shows up on a `missed_game` report without a kit. A frame with any other team size is dropped. Revision 1's validator took only 5 or 6, so a `null` was a 400. The validator accepts `null` from revision 2 on, for both bundle versions.
- In a `crash` or `performance` report, `game` is `null`. An `other` report (bundle version 2) may also have `game: null`; it is then a log-only report and follows the same rules as `crash` and `performance` (this section and section 7). An `other` report with a game follows the game report rules. `reads` keeps its usual shape: all nine keys, each an object with `value`, `confidence`, and `ocr_v1` set to `null` and `suspect` set to `true` (a null value is always suspect, as below). A read is never JSON `null` as a whole.
- `captured_at` is the frame time in UTC, RFC 3339, such as `2026-10-09T12:00:00Z`. For a game report, the log window in section 6 is measured from the session around this time. Crash and performance reports have no `game` object; their window follows section 6.

`reads` is one object per field. The keys come from the `suspect_fields` names map. The map is fixed:

| Key | Meaning | Column label in the tracker today |
| --- | --- | --- |
| `mode` | Game mode | |
| `result` | Win, loss, or draw | |
| `hero` | Hero | |
| `e` | Eliminations | E |
| `a` | Assists | A |
| `d` | Deaths | D |
| `dmg` | Damage | DMG |
| `h` | Healing | H |
| `mit` | Mitigation | MIT |

Healing uses the key `h`. There is no `hlg` key. Every bundle has all nine keys. A missing read is still present, with null values, so the shape does not change between reports.

Each read has:

- `value`: the primary read, as a string, or `null`. Numbers are decimal strings such as `"1204"`, not JSON numbers, so hero names and stats share one type.
- `confidence`: a number from 0 to 1, or `null`.
- `suspect`: `true` or `false`.
- `ocr_v1`: the Tesseract value as a string, or `null` when Tesseract did not read that field.

For `e`, `a`, `d`, `dmg`, `h`, and `mit`, the primary read is the matcher (`RECOGNIZER_ID`) when it ran. `ocr_v1` is the Tesseract value beside it. `suspect` is `true` when the matcher's own suspect rule flags the cell (for `cv-v5` that includes confidence below 0.35), when the matcher and `ocr-v1` disagree, or when the value is `null`. When the matcher did not run, `value` copies `ocr_v1`, `confidence` is `null`, and `suspect` follows the tracker's existing flag for that cell.

For `mode`, `result`, and `hero`, the matcher does not read the field. `value` and `ocr_v1` are the Tesseract read (the same string twice when it exists). `confidence` is `null`. `suspect` is `true` when the tracker would not trust that read, or when the value is `null`.

`system` is optional and only allowed in `bundle_version` 2. The app includes it on every report it writes. It describes the tracker setup, not the person. Every key is required when the object is present, and unknown keys are invalid:

| Key | Type | Example |
| --- | --- | --- |
| `session_type` | `x11`, `wayland`, or `unknown` | `x11` |
| `capture_backend` | `x11`, `wayshot`, `portal`, or `none` | `x11` |
| `cpu_threads` | positive integer, the machine's available parallelism | `16` |
| `ocr_threads` | positive integer, the OCR worker count in use | `4` |
| `reader` | `ocr-v1` or `new` | `new` |
| `shadow_recognizer` | boolean, the Extra number reader setting | `false` |
| `update_channel` | `stable` or `prerelease` | `prerelease` |
| `daemon_running` | boolean, whether the daemon answered when the report was built | `true` |
| `last_exit` | `clean`, `crashed`, or `unknown` | `crashed` |

`last_exit` comes from the crash marker in section 7. There is no hostname, user name, GPU serial, path, or display name in `system`, and the app does not add one.

`corrections` is the member's edits, if any. Keys are a subset of the nine names above. Values are strings of 1 to 64 characters. An empty object means the member changed nothing. A correction does not replace the original read. The original stays in `reads`.

`consent` has three booleans:

- `training`: off unless the member ticked the training box.
- `own_name_included`: off unless the member ticked "include my own name".
- `glyphs_included`: off unless the member ticked the wrong-glyph box.

`files` lists every file in the zip except `manifest.json`. Each entry has:

- `path`: a relative path with forward slashes, no `..`, and no leading slash.
- `sha256`: 64 lowercase hex characters, the SHA-256 of the uncompressed file bytes.
- `bytes`: the uncompressed size, a positive integer.
- `role`: `log`, `crop`, `own_name`, or `glyph`.
- `screen_class`: the screen class from section 4 for a `crop`, otherwise `null`. `crops/outcome.png` has class `outcome`, and no other path has class `outcome`.

There is exactly one `log` entry, and its path is `log.txt`. A log-only report (`crash`, `performance`, or `other` with `game: null`) has no `crop`, `own_name`, or `glyph` entry, and its consent `own_name_included` and `glyphs_included` are `false`. A `glyph` or `own_name` entry exists only when the matching consent flag is `true`. A glyph path is `crops/glyph-<id>.png`. `<id>` is 8 lowercase hex characters from the operating system's cryptographic random source, unique inside the zip. It is not a counter, not a place in a name, and not shared by glyphs that came from the same name. The `files` array lists glyph entries in the shuffled order from section 5, and it is not sorted again by id, hash, or name. The own-name paths are `crops/own-name.png` and, only when the HUD plate is also included, `crops/own-name-hud.png`. Every other image is role `crop` and lives under `crops/`.

`debug/mapcrops/` is local only. Those files are never listed in `files`, and the zip does not contain them. If a manifest lists a path under `debug/mapcrops/`, the server responds 400 with `file missing` and stores nothing.

A glyph entry has the file fields above, with `role` set to `glyph` and `screen_class` set to `null`. The only per-letter labels on that entry are:

- `id`: the same random id as in the filename.
- `reader_guess`: one character, what the reader produced for that glyph.
- `confidence`: a number from 0 to 1.
- `correct_char`: one character, present only when the member knows the right letter. Omit the key when they do not.

No other letter field is allowed. There is no name, no name group, no index, and no position.

```json
{
  "path": "crops/glyph-a1b2c3d4.png",
  "sha256": "2222222222222222222222222222222222222222222222222222222222222222",
  "bytes": 800,
  "role": "glyph",
  "screen_class": null,
  "id": "a1b2c3d4",
  "reader_guess": "O",
  "confidence": 0.31,
  "correct_char": "0"
}
```

Leave `correct_char` out of the object when the member does not know the letter. The hash above is only the right width.

The manifest does not list a hash of itself. Unknown keys are invalid. A repeated key in one object is invalid. The schema test rejects both. The API responds 400 (section 8). It does not keep the last copy of a repeated key.

### Example

Hashes below are the right width and are not digests of a real file. `app_version` `0.0.0` stands in for the running build.

```json
{
  "bundle_version": 2,
  "app_version": "0.0.0",
  "recognizers": {
    "matcher": "cv-v5",
    "ocr": "ocr-v1"
  },
  "resolution": { "width": 2560, "height": 1440 },
  "ui_scale": null,
  "reason": { "category": "wrong_stats", "text": "elims looked high" },
  "system": {
    "session_type": "x11",
    "capture_backend": "x11",
    "cpu_threads": 16,
    "ocr_threads": 4,
    "reader": "new",
    "shadow_recognizer": false,
    "update_channel": "prerelease",
    "daemon_running": true,
    "last_exit": "clean"
  },
  "session_id": "sess-example",
  "game": {
    "map": "Busan",
    "mode": "Control",
    "result": "Defeat",
    "team_size": 6,
    "captured_at": "2026-10-09T12:00:00Z"
  },
  "reads": {
    "mode": { "value": "Control", "confidence": null, "suspect": false, "ocr_v1": "Control" },
    "result": { "value": "Defeat", "confidence": null, "suspect": false, "ocr_v1": "Defeat" },
    "hero": { "value": "Ana", "confidence": null, "suspect": false, "ocr_v1": "Ana" },
    "e": { "value": "21", "confidence": 0.22, "suspect": true, "ocr_v1": "20" },
    "a": { "value": "8", "confidence": 0.9, "suspect": false, "ocr_v1": "8" },
    "d": { "value": "4", "confidence": 0.88, "suspect": false, "ocr_v1": "4" },
    "dmg": { "value": "8432", "confidence": 0.8, "suspect": false, "ocr_v1": "8432" },
    "h": { "value": "2100", "confidence": 0.77, "suspect": false, "ocr_v1": "2100" },
    "mit": { "value": "0", "confidence": 0.7, "suspect": false, "ocr_v1": "0" }
  },
  "corrections": { "e": "20" },
  "consent": {
    "training": false,
    "own_name_included": false,
    "glyphs_included": false
  },
  "files": [
    {
      "path": "log.txt",
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "bytes": 24,
      "role": "log",
      "screen_class": null
    },
    {
      "path": "crops/scoreboard.png",
      "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
      "bytes": 1000,
      "role": "crop",
      "screen_class": "scoreboard"
    }
  ]
}
```

## 3. Crops

Every image in the zip is a lossless PNG at native resolution. Native means one capture pixel becomes one image pixel. A crop may be smaller than the frame because it is a rectangle cut from the frame. It is not scaled down, and it is not scaled up. The app never writes JPEG. The app never recompresses a crop as JPEG and renames it.

Colour type is 8-bit truecolour RGB, or 8-bit RGBA when a glyph crop needs a mask. Interlace is off. All metadata is stripped. The only chunks allowed are `IHDR`, `IDAT`, `IEND`, and `PLTE` when a palette image cannot be avoided. The app should write RGB or RGBA and not a palette. Text chunks, EXIF, time chunks, colour profiles, and physical-pixel chunks are removed. The PNG magic bytes are `89 50 4E 47 0D 0A 1A 0A`.

The main scoreboard image is `crops/scoreboard.png`: the scoreboard rectangle of the frame, blanked by the allowlist in section 4. The rectangle is cut in memory and blanked before anything is written. The app may also include `crops/frame.png`, the full blanked frame, when the zip still fits the size cap in section 8. If the full frame would make the zip too big, the app leaves it out and says so in the preview. It does not shrink the frame to fit. On a scoreboard frame, `crops/frame.png` fills every pixel outside the scoreboard rectangle with the solid colour. That covers side panels and the part of the chat band (x 0 to 340, y 620 to 855 of the playfield) that falls outside the rectangle. Inside the rectangle, `crops/frame.png` uses the same allowlist as `crops/scoreboard.png` (section 4): only the kept portrait and digit cells show, and everything else inside the rectangle is filled. The app in this revision does not write `crops/frame.png`; the report kit holds only `crops/scoreboard.png` and `crops/outcome.png`. Revision 1 also allowed extra 1:1 crops of suspect digit cells. The validator only accepts `crops/scoreboard.png` and `crops/frame.png` for role `crop`, so those extra crops were a 400. Revision 2 drops them. A later change adds `crops/outcome.png`, so a `crop` path is exactly one of those three. The outcome crop is the competitive rank screen's result word box, x 10/1000, y 145/1000, width 240/1000, height 80/1000 of the playfield (the box `read_rank_screen_result` reads). It holds only the result word (Victory, Defeat, or Draw). Like every crop, it is cut from the blanked frame. On a rank screen frame, the `outcome` class fills only `overlay_top`, `overlay_bottom`, and the bands outside the playfield, and none of them reaches this box, so blanking does not paint it. It is cut only from a frame on which the rank screen detector (`OutcomeSource::RankScreen`) fired, from those same pixels, after that box held still across ticks (the detector's stability check), and only when the scoreboard detector (the capture preflight) did not accept that same frame: the preflight answers "not a scoreboard" and the row scan does not look like a scoreboard. The rank word match is loose (a title such as QUICKDRAW contains DRAW), so a Tab-held scoreboard can fire the rank screen detector. It is never cut from a frame on which only the banner, the accolade result word, or the end title fired, and never from a different frame than the one that read the word. On other screens the same box is not name-free: on a scoreboard frame it holds the member's hero portrait, rank badge, and title, with their name a few pixels to the right. The accolade screen is excluded because the member's own name sits just below its word box. Banner, end title, and scoreboard-header outcomes write no outcome crop. A later revision can add a cell-crop path with its own validator change.

The scoreboard rectangle and the outcome box are blanked in memory before they are written, and the scoreboard blanker reads chat from the rest of the frame. The only images cut from the unblanked frame are the own-name crop and the glyph crops, and only when their boxes are ticked.

Unblanked frames stay in memory while those crops are cut. If an unblanked frame has to be a file, that file is readable and writable by the owner only. The app deletes it when the zip is sealed, when the preview closes, and again at the next start if a crash left it behind. It is not written to the download directory or left in the data directory.

## 4. Blanking

Blanking runs on the member's PC, before the preview. The member never sees a preview of an unblanked enemy name, and that image is never written into the zip.

Name regions are filled with the solid colour RGB 32, 32, 32. The fill does not blur, pixelate, or sample nearby pixels. The same colour is used for every region.

Regions are fractions of `game_rect_16_9`, the centered 16:9 playfield the tracker already uses. The code keeps them in one table, in one module. The preview, the blanker, and the synthetic fixtures all read that table. Call sites do not repeat the fractions. If this document and the code table disagree, this document wins, and both are updated in the same change. A test checks that the code table matches the numbers here.

Pixel edges use the same truncation as the current scoreboard crop: scale by the fraction, then truncate toward zero. On an exact 16:9 frame, `game_rect_16_9` is the whole frame.

The bands outside the playfield (the letterbox or pillarbox) are filled with the same solid colour on every frame we keep. Those bands are the complement of the playfield, not a row in the table. Overlays and notifications sit there on ultrawide and 16:10 captures.

### Scoreboard (revision 5, measured)

The scoreboard crop, as fractions of the playfield, matches the crop the tracker already uses:

| Edge | Fraction of the playfield |
| --- | --- |
| x | 175/1000 |
| y | 150/1000 |
| width | 650/1000 |
| height | 700/1000 |

`crops/scoreboard.png` is an allowlist. It starts as the solid colour, and only two kinds of pixel are copied from the frame: a row's portrait cell and a row's digit glyphs. Each comes from a structure measured on that board and checked by a second cue. Nothing is placed by a fixed fraction of the crop. The in-match table sits at four widths and offsets (perk icons move it by up to 57 px at 1440p) and the post-match table is centred, so revision 3's fixed columns cut through portraits and left names visible. Everything that is not copied stays filled: the header, the name column with its badge, rank icon, name, and title, the ult and perk icons, the voice icons, the gap between the teams, the hero card, and the game world behind and beside the table, where in-world nameplates show.

Geometry, measured per board. Lengths are in the 1664 px crop of a 2560x1440 frame, times `s` = crop width / 1664.

- Board: the crop aspect is within 1% of 1664:1007 and `s` is between 0.70 and 1.60. Only the 1440p and 1080p crops (`s` within 5% of 1 or of 0.75) keep anything: digits and portraits have no measured floors at other scales, so a board measured at any other scale keeps nothing. The light header strip gives the table edges `L` and `R` and the header bottom. Only the measured layouts pass: the in-match table centred at 522 `s`, or the post-match table centred in the crop, with `R - L` of 930, 978, 994, or 1044 `s`. The stat header labels sit at their offsets from `R` (5 of the 6 must be found; the one-letter H can drop out of the label scan), with no other label between E and MIT.
- Rows: each team's grid is fitted to the 2 px separators between rows: team 1 from the header bottom, team 2 from 556 to 576 `s`, pitch 74.5 to 75.75 `s`. A team's grid is trusted when 3 of its internal separators show, and a row is used only when the separators on both of its sides show.
- Only 6v6 has measured geometry. Any other team size, or an unknown one, keeps nothing. A board whose geometry fails keeps nothing at all.

A row's portrait cell runs from `L` to the measured portrait edge (`L + 104 s`), 1 px inside the row. It is copied whole only when all of these hold:

- the cell matches the same row's cell on an earlier capture of the same game, taken at least a minute earlier and at most 30 minutes earlier: of the difference between the two cells, minus its 15 `s` box mean (a team colour or world shift moves the whole bar or background and cancels), no channel may be off by more than 20 at more than 20 px (x `s` squared). Text, boxes and fades drawn on one capture and not the other differ at hundreds of px. The daemon keeps the portrait cells of a game's last 8 captures in memory for this (never on disk); the first capture of a game, a hero swap, a death and a respawn keep no portrait until a matching earlier capture exists;
- the role bar is one opaque colour: 55% of its non-icon pixels sit within 40 of their median;
- the bar shows at most 2 sharp-stroke pixels (a channel more than 30 from its 5x5 median) outside the role icon, a role icon of at least 120 white px (x `s` squared), and white only inside the icon zone, with the icon centred and mirror-symmetric;
- the 6 px strip right of the portrait holds at most 2 white or grey or bright bar-coloured pixels and at most 30 sharp-stroke pixels of any colour, and the 8 px strip left of the bar at most 2 white or grey pixels;
- the portrait's right edge shows on at least 40% of the row's pixel rows (a menu box laid across it hides it);
- the row separators above and below the portrait square show: on at most 35% of its columns may a separator sit within 12 of the pixels 3 `s` above and below it on every channel (real rows: at most 26%);
- no chat can be over the row (below).

On the member's 34 boards of 2026-10-11 and 10-12, blanked in time order as one member's captures (the daemon's case), the edge gates drop 39 of 408 portraits (the rows with no hero picked, which show no role icon) and the content check drops 263 more (207 differ from every earlier capture at least a minute older, 56 have none): 106 of 408 portraits are kept at 1664 px and 80 of 408 at 1248 px. Those captures are a few minutes apart, with deaths and hero swaps between them; the daemon sees every accepted Tab, so it finds a matching earlier capture more often.

Chat is read on the full frame. The chat box stacks lines upward from a bottom line below the crop, and each line starts left of the crop. When the strip at x 60 to 448, y 1250 to 1340 of the 2560x1440 playfield holds chat text (more than 12 sharp-stroke pixels; the member's chat-free Tab frames hold 0 to 1, frames with chat 282 to 2,071), the stack is followed upward line by line. Every row it can reach keeps no portrait, and at least the rows reaching below 85% of the crop height. Blanking a crop with no frame drops those bottom portraits too. Chat drawn in a role bar's own colour cannot be seen on the bar, so this is the gate that stops it.

A row keeps its numbers only when all six of its cells pass every gate; otherwise all six cells of that row stay filled. The cells are read by a copy of the tracker's digit reader (cv-v5, `shadow/digits.rs` at a9c01c0, in `src/report/blank/reader.rs`): segmentation, row text band, template beam, and field grammar. The gates:

- Board: the board's digit height is 11 to 15.5 `s`, its white level at least 150, each column's numbers centred within 3 `s` of `R` minus the column offset, the scale within 5% of the 1440p or 1080p crop, and the bundled letter templates loaded.
- Row: the team tint has a saturation of at least 0.30, from its own role bar or the team's other bars (a grey tint lets text behind the row look like white digits); the row has a text band.
- Cell: one ink group; a cell colour saturation of at least 0.30; no second ink layer; a grammatical reading with none of the reader's sanity flags; at most 2 digits in E, A, and D; no slanted cut; centred within 2.5 `s` + 0.5 px of the column's board consensus; the board's digit height on the row's baseline; all ink on the text line.
- Glyph: per-class floors for the template score, the calibrated margin, and the ink mass, set just under the member's real digits; not more letter-like than digit-like against 1,512 letter templates drawn from 18 openly licensed fonts (`assets/blank-letters`, built by `scripts/gen_blank_letters.py`; a missing set keeps no glyph); the digit height; commas below the line; consecutive glyphs 9 to 12.5 `s` apart, 6 to 9 `s` around a comma; touching glyphs cut apart only at a thin joint, at most 0.45 of the text height in ink; a lone 0 dim (0.55 to 1.05 of the white level), anything else at least 0.80 of it.

Copied from a kept cell are its glyphs' binarised pixels, plus the rim pixels around them that lie at least 12% of the way from their local background to the glyph colour and within 40 of the blend ray from the cell's smooth background. The cell is painted one flat colour first: the colour nearest its median that sits more than 2 levels from every source pixel of the cell, so no painted pixel equals a source pixel. Every kept row is read back from the output, and a row that does not read the same keeps nothing. Kept portrait and glyph pixels are byte-identical to the frame's pixels.

Measured on 2026-10-12 with the Rust blanker:

- The member's 34 real boards at 1664 px: 0 secret pixels kept, 97.5% of digit pixels, 407 of 408 rows of numbers (one row needed a slanted cut), and (blank table 10, in time order) 106 of 408 portraits.
- Both post-match fixtures keep all 12 rows of numbers.
- 118 forged attacks: 3 leak, 278 px in all, none readable.
- 219 further forgeries (menus, notifications, killcam banners, coloured text beside digits, chat in every channel colour): all closed, except a box drawn wholly inside one portrait.
- Attack round 2, re-run on blank table 10 (582 forgeries with the clean board as the earlier capture): every portrait and overlay forgery copies 0 drawn px (context menus opened inside a portrait, menus mid fade-in, coloured or faded text across the portrait edge, translucent boxes, tooltips over the portrait column, panels over the role bar, text in the icon zone, chat in the bar colour, a translucent board over in-world nameplates). What is left is the digit rim residue below (53 forgeries, 4,959 px) and one 2 px sample.
- The same 711 forgeries with the overlay also on the earlier capture (it stayed pixel-identical for a minute or more): the edge gates alone leave 178 leaking, 102,353 px (portraits and digit rims together).

Known limits:

- An overlay that stays pixel-identical over a portrait for a minute or more (a pinned stream overlay, a menu held open through two captures a minute apart) passes the content check, and only the edge gates judge it. A box or text drawn wholly inside one portrait, or one whose edges miss every gate, is then copied with the portrait. Fades and translucent boxes over the moving world cannot stay identical for a minute; opaque ones can.
- Pale text behind a translucent row can leave rim samples on digit rims: chains of up to about 60 px per component, unreadable.
- Text drawn exactly like the game's digits (same font, size, baseline, and spacing) is read as a number.
- 5v5, Stadium, FFA, custom and spectator boards, non-English thousands separators, other HUD scales, and resolutions other than 1440p and 1080p keep nothing until a real board is measured.

A kit stores the blank table version it was blanked with (`BLANK_TABLE_VERSION`, 10 for this revision), and a kit from another version is never reported.

### Other name regions

Fractions are of the playfield (`x`, `y`, `width`, `height`), in thousandths. The app uses only `overlay_top` and `overlay_bottom` today, to blank the rank screen before the outcome box is cut. The other rows are the plan for screen classes it does not send yet; the scoreboard crop needs none of them, because its allowlist keeps nothing outside the measured cells.

| Id | Class | x | y | w | h | Confirmed |
| --- | --- | --- | --- | --- | --- | --- |
| `overlay_top` | every kept frame | 0 | 0 | 1000 | 40 | no |
| `overlay_bottom` | every kept frame | 0 | 960 | 1000 | 40 | no |
| `kill_feed` | scoreboard | 700 | 40 | 300 | 105 | no |
| `chat_below` | scoreboard | 0 | 855 | 1000 | 145 | no |
| `own_name_hud` | gameplay | 360 | 860 | 280 | 90 | no |
| `chat` | gameplay, killcam | 0 | 620 | 340 | 300 | no |
| `kill_feed` | gameplay, killcam | 680 | 40 | 320 | 220 | no |
| `death_recap` | killcam | 180 | 160 | 640 | 560 | no |
| `potg_name` | potg | 20 | 550 | 500 | 300 | no |
| `accolade_names` | accolade | 150 | 180 | 700 | 620 | no |
| `party_list` | party | 0 | 100 | 240 | 800 | no |
| `lobby_left` | lobby | 40 | 140 | 300 | 720 | no |
| `lobby_right` | lobby | 660 | 140 | 300 | 720 | no |

What each region is:

- Scoreboard names: all 10 or 12 player rows, from the tables above.
- `own_name_hud`: the plate that draws the member's own name on the HUD.
- `chat` and `chat_below`: the chat box. On a scoreboard frame the strip below the board is used, so the fill does not cover portraits or digits. On gameplay and killcam the larger box is used.
- `party_list`: the party or group list.
- `kill_feed`: the kill feed.
- `death_recap`: names on the death recap and killcam.
- `potg_name`: the POTG name card. These fractions are the tracker's existing nameplate crop (20, 550, 500, 300 in thousandths of the playfield).
- `accolade_names`: name cards on the accolade screen.
- `lobby_left` and `lobby_right`: name slots in the custom game lobby.
- `overlay_top` and `overlay_bottom`: top and bottom overlay bars, including MangoHud and tools like it.

A row is confirmed only when this document records that the rectangle covered the name on members' local crops. That check includes other UI scales, 16:10, ultrawide, and colourblind filters. Synthetic fixtures that share the blanker's area table only prove that the fill paints those rectangles. They do not prove a real name sits inside them. No row is confirmed yet. `potg_name` matches a crop the tracker already measures, and that is not enough to mark it confirmed.

Until a row is confirmed, the app does not send that screen class, with two first-cut exceptions. Scoreboard frames may be sent: their name rows follow the crop the tracker already measures, and `kill_feed`, `chat_below`, and the overlay bars are filled as well. On those frames `crops/frame.png` also fills everything outside the scoreboard rectangle (section 3). POTG frames may be sent from the existing nameplate crop. Gameplay, killcam, accolade, party, and lobby frames are dropped until their rows are confirmed.

A kept frame of another class is blanked with the rows for its class, plus `overlay_top` and `overlay_bottom`.

### What is dropped

The app classifies the frame with the detectors it already trusts (scoreboard, POTG nameplate, and the other screen classes above). The `outcome` class is used only for `crops/outcome.png` and has no name region. If no detector accepts the frame, the frame is dropped. If a system notification could cover the game, or the app cannot see that every name on that screen falls inside a blank region for that class, the frame is dropped. If the app is not sure whether the board is 5v5 or 6v6, the blanker keeps nothing of it, so it writes no scoreboard. A dropped frame is not blanked "as best we can" and then sent. It is absent from the zip and from the preview. If every frame is dropped, there is nothing to send, and Save report stays disabled.

## 5. Name read failures

Other players' whole names never leave the PC. They are not in the log, the manifest, the free text, a file name, or a crop.

The member may tick "include my own name". That box is off by default. When it is on, the zip may contain only that member's own name, as a crop of the name rectangle and nothing around it. On a scoreboard frame this is the name rectangle of the row the tracker already treats as the member, cut before the frame is blanked, saved as `crops/own-name.png`. If the HUD plate region is on a frame we are allowed to send, that plate is `crops/own-name-hud.png`. The crop is allowed only when the name read on that row, or on that HUD plate, matches the configured player name. If the tracker does not know which row is the member, or the read on the chosen row does not match the configured name, the box cannot add a crop. The app says why. It does not guess a row, and it does not send the crop from a row that was identified wrongly. The BattleTag discriminator (the `#` and the digits) is part of the member's own name and is included only in that crop, only when the box is on and the read matches.

When the name reader gets a glyph wrong, the bundle may include that glyph by itself. This is a second box, also off by default (`glyphs_included`). A glyph image is masked pixel by pixel. A pixel stays only when it belongs to that one letter. Every other pixel, including a two pixel pad around the letter, is the solid blank colour. If the reader cannot separate the glyph from its neighbours, that glyph is left out. The image, the file name, and the manifest carry no board position, no row, no column, and no neighbouring character. Glyphs are cut from other players' names. The member's own name uses the own-name box, not the glyph box.

A report sends at most 2 glyphs from any one name. The cap is for the whole zip, so the same name on two frames still contributes at most 2 glyphs. If that name has more than two wrong glyphs, the app picks the same two letters every time for that game and that name. The pick is a stable rule of the session id and the name. A later report of the same game does not choose a fresh pair, so the reports cannot be laid side by side to collect more than two letters from one name. The rule is not written into the zip.

Those chosen glyphs, from every name, go into one pool. The app shuffles that pool across the whole bundle. The shuffle and the random ids both come from the operating system's cryptographic random source, not a seeded game shuffle. Zip entry order and the manifest `files` order follow the shuffle. They are not sorted by name, by character position, or by a counter. Two glyphs from the same name are not grouped, and nothing in the manifest or the filenames says they belong together. Each file uses its own random id, as in section 2. The bundle keeps no ordering, no index, and no name grouping for glyphs.

Each glyph entry in the manifest carries only these per-letter labels: the random `id`, `reader_guess`, `confidence`, and `correct_char`. `correct_char` is optional and is filled only when the member knows that letter. The entry does not say which name the letter came from, or where it sat in that name.

Both boxes can be off. That is the default. The zip then has blanked crops and the scrubbed log only.

## 6. Log scrub

The log in the zip is `log.txt`. It is not the raw `daemon.log`. The app reads `{data_dir}/daemon.log` and `{data_dir}/daemon.log.1`, keeps the lines inside this game's time window, scrubs them, and writes those lines only.

The window is the whole session: from 120 seconds before the first stored capture to 120 seconds after the last stored capture. For a `performance` report, or a `crash` report with no crash marker, there is no session, so the window is the 30 minutes before the report is built. A `crash` report with a crash marker uses the 30 minutes before the marker's `last_seen` through 2 minutes after it, so the lines just before the stop are always included even when the report is built a day later. It also keeps the 30 minutes before the report is built when that window does not overlap. The marker's start time is not a window edge. A line with no timestamp is left out. A line outside the window is left out. The `capture timing` lines the daemon writes once a minute are kept like any other line. They carry no names. Every name the reader produced inside the window is in scope, not only names from the single frame in the zip.

The app drops name fields from the reader's log lines. A field that holds a player name is not copied into `log.txt`. It also scrubs name text, because a name can sit in the message rather than in a field. The scrub list is every name read in the window: other players, the configured player, misreads, and short names of one or two characters.

These values are replaced with placeholders, longer matches first:

| What | Placeholder |
| --- | --- |
| The configured sync token, exact match | `[sync-token]` |
| A Bearer credential: the word Bearer in any case, then whitespace, then the credential | `[sync-token]` |
| Any other token-shaped run in the log or in `reason.text`: 24 or more characters from letters, digits, underscore, hyphen, or dot, or a base64 run of that length (letters, digits, plus, slash, and trailing equals signs) | `[sync-token]` |
| The configured server URL, and any URL on that host | `[server-url]` |
| The home directory prefix, wherever it lives, including paths under `/home/`, `/Users/`, and `/var/home/` | `[home]` |
| The account name, as a whole token or a path segment, including outside `/home` | `[user]` |
| The hostname, as a whole token | `[host]` |
| A BattleTag: 2 to 12 letters (any language, not only A to Z), digits, or underscores, then `#`, then 4 to 8 digits | `[battletag]` |
| Every name on the scrub list above, including misreads and short names | `[player]` |

Token-shaped scrub does not apply to `sha256` fields in the manifest. The configured player name is scrubbed even when it has no `#` discriminator. Short names are scrubbed in the log as well as blanked in the image.

The config file is not attached. The sync token file is not attached. The shadow digit log is not attached. After scrubbing, the raw token, any Bearer credential, any token-shaped run, the raw URL, the raw home path, the account name, the hostname, the raw BattleTag, and the raw player names do not appear in the zip, including in `reason.text` and in file names.

If the window has no lines, `log.txt` still exists and contains the single line `[no log lines in window]`.

## 7. Flow

The GUI has two ways in:

- **Report** on a game card (in the Games tab and the game detail view). It builds a game report for that session. The category list starts at `wrong_stats` and leaves out `crash` and `performance`.
- **Report a problem** in the app menu and in Settings. It builds a `crash` or `performance` report (the member picks one), or `other`. It has no game and no crops.

Crash marker: when the daemon starts, it writes `{data_dir}/run.marker` with its pid, its start time, and `last_seen`, the last time it was known to be alive. While running it rewrites `last_seen` once a minute (an atomic replace of the file, so a crash mid-write leaves the previous copy). A clean shutdown (SIGTERM, a normal exit, or an update restart) removes it. When the daemon starts and finds an old marker whose pid is not running, or is running but is not a `scuffed-stat-tracker` process (pids are reused), the last run did not exit cleanly. It moves the marker to `{data_dir}/last_crash.marker` (one file, overwritten) and logs one WARN line. The next time the app opens, if `last_crash.marker` exists and is newer than the last report or dismissal, the app asks once: "The tracker stopped unexpectedly. Send a report?" Yes opens Report a problem with `crash` picked. No or Send removes the marker. The marker holds only the pid, the start time, and `last_seen`. It is never uploaded as a file. `last_seen` sets the crash log window (section 6). The app shows the stop as "around `last_seen`", which is at most a minute before the real stop.

Report kit: while a game is captured, the daemon keeps what a game report needs in `{data_dir}/report-kit/<session_id>/` (directory 0700, files 0600, each replaced atomically, the newest 20 sessions kept). `scoreboard.png` is the newest accepted scoreboard of the session, blanked at capture time under section 4, so no unblanked frame is ever written for a report. A board the blanker keeps nothing of writes no kit. `outcome.png` is the rank screen word box under section 3. `reads.json` holds the reads of the capture `scoreboard.png` came from, in the manifest read shape. Those reads are the original reads; a later edit by the member does not change them, and the edit goes into `corrections`. The tracker does not keep a per-field confidence yet, so `confidence` is `null`. A session id that is not 1 to 64 ASCII letters, digits, `-`, or `_` gets no kit. A game with no kit cannot be reported, except as `missed_game`, which then has a `game` object with null fields, the build time as `captured_at`, empty reads, and session `none`.

Both ways open a preview built by reading the zip that would be sent.

The preview lists every file: path, size in bytes, and sha256. It shows every image in that zip, including an own-name crop or a glyph crop when those boxes are on. The preview states each image's true pixel size. The on-screen fit may scale the display. A 1:1 view is available so the member can see the pixels that will be sent. The preview does not show a frame that was dropped, and it does not show an unblanked copy.

The preview has the consent boxes:

- Training, off by default. The label says the report is deleted after 30 days unless this is ticked, and that ticking it lets us keep the report for training. The choice is stored in `consent.training`.
- Include my own name, off by default.
- Include wrong glyphs, off by default.

The free-text box has a hint beside it: do not type other players' names. Scrubbing still runs on that text.

Changing a box rebuilds the zip and reloads the preview from the new zip before the member can save. The member can read the scrubbed log text in the preview.

The preview has **Send** and **Save report**. Send uploads that same zip (section 8). Save report writes that same zip to `XDG_DOWNLOAD_DIR`. The app reads that key from `$XDG_CONFIG_HOME/user-dirs.dirs`, or from `$HOME/.config/user-dirs.dirs` when `XDG_CONFIG_HOME` is unset. If the key is missing or the directory cannot be created, the app uses `$HOME/Downloads`. The file name is `scuffed-report-<UTC timestamp>.zip`, with the timestamp as `YYYYMMDDTHHMMSSZ`. If that name is already there, the app appends `-2`, `-3`, and so on. It does not overwrite an older report in silence. The home path is not stored inside the zip.

A saved zip may be handed to an officer, for example when Send fails or the tracker is not linked. The file is the preview zip, already blanked and scrubbed. Before the save, the app says that this is a private report, that the member should give it only to an officer, and that they should not post it in a public channel. Once the file has left the PC, the app cannot delete the copy they gave away. The same warning says that.

Send POSTs the zip to the server the tracker is linked to, and nowhere else. On success the preview shows the report id, its expiry, and a link to the site's My reports page (`/reports`), where the member can delete it or withdraw training consent. The app keeps no copy after a successful send, unless the member also pressed Save report. If the tracker is not linked (no sync token), Send is disabled and the preview says to link the tracker in Settings or to use Save report. A 401 says the link is no longer valid and to link again. A 429 shows the daily cap message with the `Retry-After` wait when there is one. Any other failure keeps the preview open and says the report was not sent. The app does not retry on its own.

If the server has reports switched off, upload gets 503 with `{"error":"reports_disabled"}` (section 8). The app shows "Reports are off right now". It keeps the saved bundle. It does not retry on its own. The member can try the upload again by hand later.

## 8. API contract

This section is the contract the API in #189 builds against. The privacy rules and the status codes below are fixed. The desktop app does not hold a site session. It sends with the tracker's sync token (create only, below), and Send is disabled when the tracker is not linked (section 7).

Create accepts either of two credentials. Every other report route accepts only the site session.

- The site session for an org member (`OrgMember`), the same session the site already uses.
- The tracker's sync token, sent as `Authorization: Bearer <token>` (the same header the tracker uses for stats sync). The server resolves it to the member that owns it and applies the same membership checks as the session: an inactive, suspended, or banned member gets 403. The sync token authorizes `POST /api/stat-reports` and nothing else on this API. List, download, the manifest read, delete, and withdraw answer 401 to a sync token. The token goes only in that header. It is never in the zip, the manifest, the free text, or a file name, and the log scrub (section 6) still replaces it.

A report sent with the sync token is stored exactly like one sent with the session: the member id is the token's owner, and the daily cap counts both together. Officers see the member id, as for any report. Hiding it from officers is a later option, not part of this revision.

Routes:

| Action | Route | Who |
| --- | --- | --- |
| Create | `POST /api/stat-reports` | Signed-in member |
| List metadata | `GET /api/stat-reports` | Officer, or the member seeing only their own ids |
| Download the zip | `GET /api/stat-reports/{id}` | Officer only |
| Read the stored manifest | `GET /api/stat-reports/{id}/manifest` | Officer only |
| Delete | `DELETE /api/stat-reports/{id}` | Owning member, or an officer |
| Withdraw training | `POST /api/stat-reports/{id}/withdraw` | Owning member |
| Withdraw training | `PATCH /api/stat-reports/{id}` | Owning member |

`PATCH` does the same thing as `POST .../withdraw`. The My reports page is part of this contract. It has a Delete button and a Withdraw button for the member's own reports, wired to the delete route and to either withdraw route. That page is on main (#192).

Officer means `OfficerUser` (officer or admin). A member who is not an officer gets 403 on the zip bytes and on the manifest route. Ids are unguessable. A create response returns the id, the training flag, and `expires_at`. It does not echo the zip. The manifest route returns the stored manifest, after the hash recompute below.

Limits: the request body is at most 10 MB (`10 * 1024 * 1024` bytes). The uncompressed files together are also at most 10 MB. Either one over that size is 413. At most 30 files besides the manifest. No image edge is longer than 7680 pixels. A member may store 5 successful reports per UTC day. A rejected upload does not count. Over the daily cap, respond 429.

The server checks the zip again. It does not trust the client. It responds 400 for a bad manifest, a path that escapes the zip, a repeated JSON key, JPEG magic (`FF D8 FF`), a file named as JPEG, or a PNG that is not a real 8-bit image (bad magic, bad CRC, interlace, or a critical chunk other than `IHDR`, `IDAT`, `IEND`, or `PLTE`). On that failure it stores nothing. It does not keep the last value of a repeated key. If a manifest lists a path under `debug/mapcrops/`, the server responds 400 with `file missing` and stores nothing.

Text, EXIF, time, colour profile, and physical-pixel chunks are stripped. The server rebuilds each PNG, rebuilds the zip, and recomputes each file `sha256`, the byte sizes, and the zip `sha256`. The stored manifest uses those recomputed hashes. A missing or bad credential is 401. A signed-in user who is not a member is 403.

When reports are switched off, every report route returns 503 to an authenticated caller. The body is exactly `{"error":"reports_disabled"}`. That covers create, list, download, the manifest read, delete, and both withdraw routes. The server stores nothing for that response. It does not answer 404 or 200 instead. Authentication runs first, so a request with no valid credential is still 401 while reports are off. Revision 1 said reports-off never answers 401, which contradicted the 401 rule above.

Storage is private. The zip bytes sit outside the database and outside the git repo. They are not placed in the public upload directory and they are not served as static files. The database may store metadata only: id, member id, received time, training flag, expiry, byte size, zip sha256, reason category, app version, and recognizer ids. Free text is stored with that metadata for officers, not in a public response. Image bytes and log bytes are not columns in the database.

Retention: a report with training left off is deleted 30 times 24 hours after it was received, in UTC. A sweeper may run hourly. It deletes expired files, files whose metadata is gone, and metadata whose file is gone. Delete removes the file and the metadata now. Withdraw clears training. Expiry is then the original received time plus 30 days. If that time has already passed, withdraw deletes the report now. Withdraw does not extend the life of a report.

Reading the zip is officer-only. A member can list their own report ids, times, and consent flags, and can delete or withdraw. They cannot download the stored zip. An officer download, a create, a delete, and a withdraw each write an audit log entry. Audit is fire-and-forget: log an error, and do not fail the request because the audit write failed.

Contabo has no backups. A report stored on that disk is for testing this flow only. A lost disk loses the reports. The site does not tell a member that a training report is archived, and nobody treats that disk as the only copy of training data.

## 9. Tests required

Tests use synthetic images drawn for the test, and the two post-match fixtures in `test-data/vision-baseline/`, which are real boards whose names were filled before they were committed. They do not read or commit member frames.

Scoreboard fixtures: synthetic 6v6 boards in each measured layout (the four in-match widths and the centred post-match table), with the measured geometry, white strokes in every name cell, and digits drawn from the reader's own templates; plus the two post-match fixtures. After blanking:

- Every name stroke is the solid colour, and the header and the world are filled.
- Every portrait passes on a clean synthetic board blanked after an identical earlier capture, and none passes on the first capture; on the post-match fixtures all 12 rows keep their numbers, read back the same.
- Each gate has a test that drops what it guards and copies none of the drawn pixels: a name in a stat cell, a notice across the columns, a grey row tint, a stroke over the role bar, a menu box laid across a portrait, touching glyphs joined by a thick stroke, chat on the frame (one line, and a stack), a crop with no frame, a board that is not 6v6, and a board whose header is gone.
- The portrait content check, each against a clean earlier capture: a dark menu opened inside a portrait, a menu caught mid fade-in, and orange, grey and blue text across the portrait edge copy nothing of the overlay; a changed portrait and an earlier capture under a minute old keep no portrait. With the overlay on both captures: a box over the portrait column hides the separators, and a panel in the bar colour hides the role icon. A board at another scale keeps no portrait.
- An ignored harness runs the blanker on a directory of member frames for retention, leaks, and read-back on the member's machine (`report::blank::port_check`).

One synthetic frame per other screen class, at both resolutions, paints the fake name into every name region of that class and checks that blanking removes it.

Classifier: a frame that no detector accepts produces no image in the zip. Gameplay, killcam, accolade, party, and lobby frames produce no image until this document marks their regions confirmed. Scoreboard and POTG frames may be included, and on a scoreboard frame the kill feed strip, the chat strip below the board, and both overlay bars are filled even though those rows are still a first cut.

Full-frame fixture: on a 1920x1080 scoreboard and a 2560x1440 scoreboard, paint a fake name outside the scoreboard rectangle, including in the band from x 0 to 340 and y 620 to 855 of the playfield and on a side panel. Build `crops/frame.png`. That name is gone. Portraits and digit columns inside the rectangle stay byte-identical.

Ambiguous team size: a board that is not clearly 6v6 keeps nothing and writes no kit.

Wrong-row fixture: the configured name is one fake name. The tracker is pointed at a row whose name read is a different fake name. With the own-name box on, the bundle has no `crops/own-name.png` and no `crops/own-name-hud.png`.

Log fixture: seed a log inside the session window with a fake sync token, a Bearer line that carries a different secret, a token-shaped run that is neither of those, a fake server URL, a home path under `/home`, a home path under `/var/home`, the account name and the hostname outside a `/home` path, a BattleTag that uses a non-ASCII letter, other players' names, a misread of a name, and a short name of one or two characters. Add one line before the window and one line after it. Also seed a reader log line that has a name field. Build a bundle. The zip does not contain the token, the Bearer secret, the token-shaped run, the URL, either home path, the account name, the hostname, the BattleTag, the other players' names, the misread, or the short name. The name field is not copied into `log.txt`. The out-of-window lines are absent. The in-window line is present with the placeholders from section 6.

Manifest: the bundle's `manifest.json` validates against section 2. A manifest with a repeated key is rejected. Every listed hash matches the file bytes. The zip contains those files and `manifest.json` only. Every zip entry is dated 1980-01-01 00:00:00, with no comment and no extra fields. Every PNG meets section 3. Training is false unless the test ticked it. With both name boxes off, the fake name does not appear as text anywhere in the zip, and there is no `own_name` or `glyph` file.

Glyph fixture: with the glyph box on, each glyph image holds one character and no neighbour, and neither the path nor the manifest records a row or a board position. Filenames use random ids, not a counter. With the box off, those files are absent. With the own-name box off, `crops/own-name.png` is absent.

Name rebuild fixture, required: seed at least three fake names. Each name is at least four characters, the names are distinct, and none is a prefix of another. Mark every character as a wrong glyph. Build one bundle with the glyph box on. From the zip alone, none of the seeded names can be rebuilt. In particular:

- Each seeded name contributes at most 2 glyph images.
- Each glyph entry has only the file fields plus `id`, `reader_guess`, `confidence`, and `correct_char` when the test filled it. It has no name, index, or position.
- No filename, manifest field, zip order, or `files` order groups those glyphs by name or by their place in the name.
- Reading `reader_guess` or `correct_char` in filename order, manifest order, or zip order does not spell a seeded name.
- No pair of glyphs that the bundle ties together spells a seeded name, because the bundle does not tie them together.

Repeat-report fixture: use the same session and the same seeded names, each with more than two wrong letters. Build two bundles with the glyph box on. Each name contributes the same two letters in both zips. The two zips together do not contain a third letter from that name. Ids and order may differ.

Revision 2 fixtures, required:

- Version: a `bundle_version` 1 manifest without `system` passes. A `bundle_version` 1 manifest with `system`, or with category `crash` or `performance`, is a 400. Version 3 is a 400.
- `system`: every key present passes. A missing key, an unknown key, `cpu_threads` 0, or a `session_type` outside the list is a 400.
- Log-only `other`: a version 2 `other` report with `game: null` follows the log-only rules below. A version 1 `other` report still needs a game.
- Log-only: a `crash` report with `game: null`, nine read objects whose `value`, `confidence`, and `ocr_v1` are null and `suspect` is true, `session_id: "none"`, only `log.txt`, and both name consents false passes. The same report with a `crops/scoreboard.png` file, or with `own_name_included: true`, is a 400.
- `team_size: null` passes on a game report.
- A `crop` path other than `crops/scoreboard.png`, `crops/frame.png`, or `crops/outcome.png` is a 400. `crops/outcome.png` with a class other than `outcome`, or `crops/scoreboard.png` or `crops/frame.png` with class `outcome`, is a 400.
- Sync token: a create with a valid tracker sync token stores the report under the token's member and counts toward that member's daily cap together with session uploads. The same token on list, download, manifest, delete, and withdraw is 401. A token whose member is suspended or inactive is 403. An unknown token is 401, also while reports are off.
- Crash marker: the daemon removes `run.marker` on SIGTERM. While running it rewrites `last_seen` at least once a minute. Starting with a stale `run.marker` (dead pid, or a live pid that is not the tracker) produces `last_crash.marker` with the old `last_seen` and one WARN line. A live tracker pid in `run.marker` does not.
- Crash window: with a `last_crash.marker` whose `last_seen` is 26 hours before the report, `log.txt` holds lines from 30 minutes before that `last_seen` and from the 30 minutes before the build, and no lines from between those windows.

Reports-off fixture, required: with reports switched off, each report route returns 503 to a caller with a valid credential (a site session, or the sync token on create) and the body `{"error":"reports_disabled"}`. Nothing is stored. On that response the app shows "Reports are off right now", the saved zip is still on disk, and no automatic retry is queued. A later upload happens only when the member tries again by hand.

## 10. Out of scope and open questions

Out of scope for this change, and out of scope for the first build unless a later spec says otherwise:

- Uploading anything the member did not see in the preview.
- Training a model, building a dataset, or a labelling tool.
- Blur, pixelation, JPEG, or downscaling as a substitute for the solid fill.
- Sending a frame the classifier is not sure about.
- Putting another player's whole name in the bundle.
- Using the sync token for any report route other than create.
- Keeping reports on Contabo as a long-term archive.
- Windows or macOS capture.
- Committing real game captures as fixtures.

Open questions:

- Confirmation is the local-crop check in section 4 (other UI scales, 16:10, ultrawide, colourblind filters). Scoreboard name rows and `potg_name` may be sent as a first cut before that check. The synthetic fixtures do not confirm them.
- Overlay thickness is a first cut (40/1000 of the playfield). If a real bar is taller than that strip, the measurement pass has to widen it or the frame has to be dropped.
- The 6v6 portrait width and digit start need the same synthetic check: the portrait rectangle must cover the portrait, and the digit rectangle must cover every digit, with the name fill in between and no shared pixel.
- UI scale is not read from the game. Until it is, the field stays whatever the member types, or `null`.
- The private directory path is chosen by the API. The routes, the 10 MB cap, 413, and 400 for a repeated JSON key are fixed in section 8.
- Hiding the member id from officers. USER chose to keep it for now (2026-10-11). It can become an option later.
- A single glyph cut from a rare name can still be recognisable. The box stays off by default, and a report keeps at most 2 glyphs from any one name, shuffled, with random ids. Dropping glyphs entirely would not change the rest of this contract.
- Training copies need a disk that has backups before anyone relies on them. Contabo is not that disk.
