# Tracker issue report bundles

This document is the contract. The tracker app, the API, and the site all build against it. This change adds the document only. There is no app, API, or site behaviour in this change.

Status: HOLD-MERGE. Do not merge until the spec is accepted.

The desktop GUI is `scuffed-stat-tracker-ui` (the binary is still `stat-tracker-gui`). The daemon is `scuffed-stat-tracker`. Blanking, the zip, and the preview live on the member's PC, in that app, before anything is saved or uploaded.

## 1. Purpose

A member sends a report so we can fix a tracker problem. The report contains a scrubbed log and debug crops from one game.

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

`bundle_version` is the integer `1`. A reader rejects any other version.

`app_version` is the tracker version string of the build that made the zip, from 1 to 32 characters.

`recognizers` has two ids:

- `matcher` is the `RECOGNIZER_ID` constant compiled into that build. On the revision this spec was written against, that constant is `cv-v3`. When the constant changes, the field reports the new id. It is not a second copy that someone updates by hand.
- `ocr` is the string `ocr-v1`. That is the Tesseract read the tracker already stores.

`resolution` is the capture size in pixels, `width` and `height`, both positive integers. It is the full frame, not the 16:9 playfield inside it.

`ui_scale` is either `null` or a positive number up to 200. The unit is the game UI scale where 100 is the default. The tracker does not detect this today. If the member leaves it blank, the value is `null`. The app does not guess.

`reason` is the member's category plus free text:

- `category` is one of `wrong_stats`, `wrong_hero`, `wrong_map`, `wrong_mode`, `wrong_result`, `missed_game`, `other`.
- `text` is a string of 0 to 500 characters. It may be empty. Before it is written into the manifest, it is scrubbed with the same rules as the log. The free-text box tells the member not to type other players' names (section 7).

`session_id` is the local session id, 1 to 64 characters. If the game has none, the value is `none`.

`game` holds the match context:

- `map`, `mode`, and `result` are strings or `null`.
- `team_size` is `5`, `6`, or `null`. `null` means the app could not tell 5v5 from 6v6 and kept the frame by filling both name layouts (section 4). A frame with any other team size is dropped.
- `captured_at` is the frame time in UTC, RFC 3339, such as `2026-10-09T12:00:00Z`. The log window in section 6 is measured from the session around this time.

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

For `e`, `a`, `d`, `dmg`, `h`, and `mit`, the primary read is the matcher (`RECOGNIZER_ID`) when it ran. `ocr_v1` is the Tesseract value beside it. `suspect` is `true` when the matcher's own suspect rule flags the cell (for `cv-v3` that includes confidence below 0.35), when the matcher and `ocr-v1` disagree, or when the value is `null`. When the matcher did not run, `value` copies `ocr_v1`, `confidence` is `null`, and `suspect` follows the tracker's existing flag for that cell.

For `mode`, `result`, and `hero`, the matcher does not read the field. `value` and `ocr_v1` are the Tesseract read (the same string twice when it exists). `confidence` is `null`. `suspect` is `true` when the tracker would not trust that read, or when the value is `null`.

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
- `screen_class`: the screen class from section 4 for a `crop`, otherwise `null`.

There is exactly one `log` entry, and its path is `log.txt`. A `glyph` or `own_name` entry exists only when the matching consent flag is `true`. A glyph path is `crops/glyph-<id>.png`. `<id>` is 8 lowercase hex characters from the operating system's cryptographic random source, unique inside the zip. It is not a counter, not a place in a name, and not shared by glyphs that came from the same name. The `files` array lists glyph entries in the shuffled order from section 5, and it is not sorted again by id, hash, or name. The own-name paths are `crops/own-name.png` and, only when the HUD plate is also included, `crops/own-name-hud.png`. Every other image is role `crop` and lives under `crops/`.

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
  "bundle_version": 1,
  "app_version": "0.0.0",
  "recognizers": {
    "matcher": "cv-v3",
    "ocr": "ocr-v1"
  },
  "resolution": { "width": 2560, "height": 1440 },
  "ui_scale": null,
  "reason": { "category": "wrong_stats", "text": "elims looked high" },
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

The main scoreboard image is `crops/scoreboard.png`: the scoreboard rectangle cut from the blanked frame. The app may also include `crops/frame.png`, the full blanked frame, when the zip still fits the size cap in section 8. If the full frame would make the zip too big, the app leaves it out and says so in the preview. It does not shrink the frame to fit. On a scoreboard frame, `crops/frame.png` fills every pixel outside the scoreboard rectangle with the solid colour. That covers side panels and the part of the chat band (x 0 to 340, y 620 to 855 of the playfield) that falls outside the rectangle. Inside the rectangle, name cells are filled and hero portraits and digit columns stay as they are. Suspect digit cells may be included as further 1:1 crops taken from the blanked frame, and only from the digit columns, so they cannot contain a name.

Blanking runs on the full frame first. Crops are cut from that blanked frame. The only images cut from the unblanked frame are the own-name crop and the glyph crops, and only when their boxes are ticked.

Unblanked frames stay in memory while those crops are cut. If an unblanked frame has to be a file, that file is readable and writable by the owner only. The app deletes it when the zip is sealed, when the preview closes, and again at the next start if a crash left it behind. It is not written to the download directory or left in the data directory.

## 4. Blanking

Blanking runs on the member's PC, before the preview. The member never sees a preview of an unblanked enemy name, and that image is never written into the zip.

Name regions are filled with the solid colour RGB 32, 32, 32. The fill does not blur, pixelate, or sample nearby pixels. The same colour is used for every region.

Regions are fractions of `game_rect_16_9`, the centered 16:9 playfield the tracker already uses. The code keeps them in one table, in one module. The preview, the blanker, and the synthetic fixtures all read that table. Call sites do not repeat the fractions. If this document and the code table disagree, this document wins, and both are updated in the same change. A test checks that the code table matches the numbers here.

Pixel edges use the same truncation as the current scoreboard crop: scale by the fraction, then truncate toward zero. On an exact 16:9 frame, `game_rect_16_9` is the whole frame.

The bands outside the playfield (the letterbox or pillarbox) are filled with the same solid colour on every frame we keep. Those bands are the complement of the playfield, not a row in the table. Overlays and notifications sit there on ultrawide and 16:10 captures.

### Scoreboard rows

The scoreboard crop, as fractions of the playfield, matches the crop the tracker already uses:

| Edge | Fraction of the playfield |
| --- | --- |
| x | 175/1000 |
| y | 150/1000 |
| width | 650/1000 |
| height | 700/1000 |

Inside that crop, the row grid matches the player row crop. Header height is 25/1000 of the crop height. Team 2 starts at 565/1000 of the crop height. Row height in pixels is `(team2_start - header) / (team_size + 1)` using integer division. Team 1 row `i` starts at `header + i * row_height`. Team 2 row `i` starts at `team2_start + i * row_height`. There are `team_size` rows on each side: 10 rows in 5v5, 12 rows in 6v6. The blank covers the full row height, not the tighter window the OCR pads inward.

Name, portrait, and digit columns are fractions of the scoreboard crop width. Ranges are half-open: the start is included and the end is not. They share edges and do not overlap. Together, portrait, name, and digits cover the full crop width of each player row.

5v5:

| Column | Start | End |
| --- | --- | --- |
| Portrait (protected) | 0 | 200/1000 |
| Name (blank) | 200/1000 | 450/1000 |
| Digits (protected) | 450/1000 | right edge |

6v6:

| Column | Start | End |
| --- | --- | --- |
| Portrait (protected) | 0 | 140/1000 |
| Name (blank) | 140/1000 | 255/1000 |
| Digits (protected) | 255/1000 | right edge |

The name blank is wider than the OCR name window on purpose. The OCR window stays where it is today (5v5 from 260/1000 with width 120/1000, 6v6 from 150/1000 with width 100/1000). Blanking does not use that tighter window. A name that spills past it would otherwise survive. The 6v6 digit edge sits just to the right of the measured 6v6 name window. At 260/1000 the old name window already landed on digits, so digits are protected from 255/1000.

Hero portrait pixels and digit-column pixels are not modified. After blanking they are byte-identical to the same rectangles before blanking, compared as raw pixels, not as a re-encoded PNG. The header row and the gap between the teams are not name rows. They are left unchanged unless another region in the active class covers them.

### Other name regions

These rows are also in the same table. Fractions are of the playfield (`x`, `y`, `width`, `height`), in thousandths.

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

A kept frame is blanked with the rows for its class, plus `overlay_top` and `overlay_bottom`. A test checks that, at 1920x1080 and 2560x1440, for both team sizes, no scoreboard name rectangle shares a pixel with a portrait rectangle or a digit rectangle.

### What is dropped

The app classifies the frame with the detectors it already trusts (scoreboard, POTG nameplate, and the other screen classes above). If no detector accepts the frame, the frame is dropped. If a system notification could cover the game, or the app cannot see that every name on that screen falls inside a blank region for that class, the frame is dropped. If the app is not sure whether the board is 5v5 or 6v6, it drops the frame. It may keep that frame only by filling the name cells of both layouts, so a name in either band is covered. A frame kept that way does not claim that portraits and digits stayed byte-identical where the two name bands overlap them, and `team_size` is `null`. A dropped frame is not blanked "as best we can" and then sent. It is absent from the zip and from the preview. If every frame is dropped, there is nothing to send, and Save report stays disabled.

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

The window is the whole session: from 120 seconds before the first stored capture to 120 seconds after the last stored capture. A line with no timestamp is left out. A line outside the window is left out. Every name the reader produced inside the window is in scope, not only names from the single frame in the zip.

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

The GUI has a Send report action. It opens a preview built by reading the zip that would be saved.

The preview lists every file: path, size in bytes, and sha256. It shows every image in that zip, including an own-name crop or a glyph crop when those boxes are on. The preview states each image's true pixel size. The on-screen fit may scale the display. A 1:1 view is available so the member can see the pixels that will be sent. The preview does not show a frame that was dropped, and it does not show an unblanked copy.

The preview has the consent boxes:

- Training, off by default. The label says the report is deleted after 30 days unless this is ticked, and that ticking it lets us keep the report for training. The choice is stored in `consent.training`.
- Include my own name, off by default.
- Include wrong glyphs, off by default.

The free-text box has a hint beside it: do not type other players' names. Scrubbing still runs on that text.

Changing a box rebuilds the zip and reloads the preview from the new zip before the member can save. The member can read the scrubbed log text in the preview.

Until the API endpoint exists, the preview does not upload. Save report writes that same zip to `XDG_DOWNLOAD_DIR`. The app reads that key from `$XDG_CONFIG_HOME/user-dirs.dirs`, or from `$HOME/.config/user-dirs.dirs` when `XDG_CONFIG_HOME` is unset. If the key is missing or the directory cannot be created, the app uses `$HOME/Downloads`. The file name is `scuffed-report-<UTC timestamp>.zip`, with the timestamp as `YYYYMMDDTHHMMSSZ`. If that name is already there, the app appends `-2`, `-3`, and so on. It does not overwrite an older report in silence. The home path is not stored inside the zip.

A saved zip may be handed to an officer before upload exists. The file is the preview zip, already blanked and scrubbed. Before the save, the app says that this is a private report, that the member should give it only to an officer, and that they should not post it in a public channel. Once the file has left the PC, the app cannot delete the copy they gave away. The same warning says that.

When the endpoint exists, the same preview gains an upload action that POSTs that same zip. Save report can remain as a local copy. Upload does not ship until the site has a My reports page with a Delete button and a Withdraw button for the member's own reports (section 8). The app does not send the zip anywhere else in the meantime.

If the server has reports switched off, upload gets 503 with `{"error":"reports_disabled"}` (section 8). The app shows "Reports are off right now". It keeps the saved bundle. It does not retry on its own. The member can try the upload again by hand later.

## 8. API contract

This section is the contract the API in #189 builds against. The privacy rules and the status codes below are fixed. How the desktop app obtains a site session is still open.

`POST` a zip with signed-in member auth. That is the site session for an org member (`OrgMember`), the same kind of session the site already uses. It is not the daemon sync token. The sync token stays out of the bundle. Until the app can hold that session, it only saves the zip locally, as in section 7.

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

`PATCH` does the same thing as `POST .../withdraw`. The My reports page is part of this contract. It has a Delete button and a Withdraw button for the member's own reports, wired to the delete route and to either withdraw route. Those buttons exist before the app's upload action ships. A member can still save a zip locally before the page exists.

Officer means `OfficerUser` (officer or admin). A member who is not an officer gets 403 on the zip bytes and on the manifest route. Ids are unguessable. A create response returns the id, the training flag, and `expires_at`. It does not echo the zip. The manifest route returns the stored manifest, after the hash recompute below.

Limits: the request body is at most 10 MB (`10 * 1024 * 1024` bytes). The uncompressed files together are also at most 10 MB. Either one over that size is 413. At most 30 files besides the manifest. No image edge is longer than 7680 pixels. A member may store 5 successful reports per UTC day. A rejected upload does not count. Over the daily cap, respond 429.

The server checks the zip again. It does not trust the client. It responds 400 for a bad manifest, a path that escapes the zip, a repeated JSON key, JPEG magic (`FF D8 FF`), a file named as JPEG, or a PNG that is not a real 8-bit image (bad magic, bad CRC, interlace, or a critical chunk other than `IHDR`, `IDAT`, `IEND`, or `PLTE`). On that failure it stores nothing. It does not keep the last value of a repeated key.

Text, EXIF, time, colour profile, and physical-pixel chunks are stripped. The server rebuilds each PNG, rebuilds the zip, and recomputes each file `sha256`, the byte sizes, and the zip `sha256`. The stored manifest uses those recomputed hashes. A missing or bad session is 401. A signed-in user who is not a member is 403.

When reports are switched off, every report route returns 503. The body is exactly `{"error":"reports_disabled"}`. That covers create, list, download, the manifest read, delete, and both withdraw routes. The server stores nothing for that response. It does not answer 401, 404, or 200 instead.

Storage is private. The zip bytes sit outside the database and outside the git repo. They are not placed in the public upload directory and they are not served as static files. The database may store metadata only: id, member id, received time, training flag, expiry, byte size, zip sha256, reason category, app version, and recognizer ids. Free text is stored with that metadata for officers, not in a public response. Image bytes and log bytes are not columns in the database.

Retention: a report with training left off is deleted 30 times 24 hours after it was received, in UTC. A sweeper may run hourly. It deletes expired files, files whose metadata is gone, and metadata whose file is gone. Delete removes the file and the metadata now. Withdraw clears training. Expiry is then the original received time plus 30 days. If that time has already passed, withdraw deletes the report now. Withdraw does not extend the life of a report.

Reading the zip is officer-only. A member can list their own report ids, times, and consent flags, and can delete or withdraw. They cannot download the stored zip. An officer download, a create, a delete, and a withdraw each write an audit log entry. Audit is fire-and-forget: log an error, and do not fail the request because the audit write failed.

Contabo has no backups. A report stored on that disk is for testing this flow only. A lost disk loses the reports. The site does not tell a member that a training report is archived, and nobody treats that disk as the only copy of training data.

## 9. Tests required

Tests use synthetic images drawn for the test. They do not read or commit anything under `crates/stat-tracker/test-data/`. The fake name is a fixed string that is not a real player name, painted in a colour that is not the blank colour and not the portrait or digit paint. This spec uses the letters `QXNAME` plus the region id, in pure red RGB 255, 0, 0.

Scoreboard fixtures: 1920x1080 and 2560x1440, each in 5v5 and in 6v6. That is four boards. Each board draws a hero portrait filling every portrait rectangle, digits filling every digit column, and the fake name filling every name rectangle for the scoreboard class, including the overlay bars, the scoreboard kill feed, and the below-board chat strip. All 10 names are painted in 5v5, and all 12 in 6v6.

After blanking:

- No red pixel remains in any name rectangle, and OCR of the image does not contain `QXNAME`. The test fails if any of that text survives.
- The raw pixels of every hero portrait rectangle and every digit column are byte-identical to the snapshot taken before blanking.
- No name rectangle shares a pixel with a portrait or a digit column.

One synthetic frame per other screen class, at both resolutions, paints the fake name into every name region of that class and checks that blanking removes it.

Classifier: a frame that no detector accepts produces no image in the zip. Gameplay, killcam, accolade, party, and lobby frames produce no image until this document marks their regions confirmed. Scoreboard and POTG frames may be included, and on a scoreboard frame the kill feed strip, the chat strip below the board, and both overlay bars are filled even though those rows are still a first cut.

Full-frame fixture: on a 1920x1080 scoreboard and a 2560x1440 scoreboard, paint a fake name outside the scoreboard rectangle, including in the band from x 0 to 340 and y 620 to 855 of the playfield and on a side panel. Build `crops/frame.png`. That name is gone. Portraits and digit columns inside the rectangle stay byte-identical.

Ambiguous team size: a board that is not clearly 5v5 or 6v6 is either absent from the zip, or both the 5v5 name band and the 6v6 name band are filled. A fake name painted in each band does not survive on a kept frame.

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

Reports-off fixture, required: with reports switched off, each report route returns 503 and the body `{"error":"reports_disabled"}`. Nothing is stored. On that response the app shows "Reports are off right now", the saved zip is still on disk, and no automatic retry is queued. A later upload happens only when the member tries again by hand.

## 10. Out of scope and open questions

Out of scope for this change, and out of scope for the first build unless a later spec says otherwise:

- Implementing the button, the blanker, the endpoint, or the My reports page. This document is the contract they must follow. Upload still waits on that page (section 7).
- Training a model, building a dataset, or a labelling tool.
- Blur, pixelation, JPEG, or downscaling as a substitute for the solid fill.
- Sending a frame the classifier is not sure about.
- Putting another player's whole name in the bundle.
- Using the daemon sync token as report auth.
- Keeping reports on Contabo as a long-term archive.
- Windows or macOS capture.
- Committing real game captures as fixtures.

Open questions:

- Confirmation is the local-crop check in section 4 (other UI scales, 16:10, ultrawide, colourblind filters). Scoreboard name rows and `potg_name` may be sent as a first cut before that check. The synthetic fixtures do not confirm them.
- Overlay thickness is a first cut (40/1000 of the playfield). If a real bar is taller than that strip, the measurement pass has to widen it or the frame has to be dropped.
- The 6v6 portrait width and digit start need the same synthetic check: the portrait rectangle must cover the portrait, and the digit rectangle must cover every digit, with the name fill in between and no shared pixel.
- UI scale is not read from the game. Until it is, the field stays whatever the member types, or `null`.
- The private directory path is chosen by the API. The routes, the 10 MB cap, 413, and 400 for a repeated JSON key are fixed in section 8.
- The desktop app does not have a site login today. Report upload waits on that. Local Save report does not.
- A single glyph cut from a rare name can still be recognisable. The box stays off by default, and a report keeps at most 2 glyphs from any one name, shuffled, with random ids. Dropping glyphs entirely would not change the rest of this contract.
- Training copies need a disk that has backups before anyone relies on them. Contabo is not that disk.
