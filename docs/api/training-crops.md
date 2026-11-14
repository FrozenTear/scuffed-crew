# Training crops (opt-in, from the stat tracker)

Alpha.3 item 14. The stat tracker can send small PNG crops of what it read
(hero icons, stat digits, map name banners, rank screen regions) so recognizers can be trained on
real captures. Every kind is opt-in, off by default, and consent is only
given on the site (with a session). The tracker sends crops itself with its
sync token and shows the consent read-only, linking to the site. JSON types and error codes live in
`crates/types/src/api/training_crops.rs`; kinds and caps live in
`crates/types/src/training_crops.rs`.

This is the hero icon store generalized, not a second store:

- Every crop is a row of the `hero_icon` table with a `kind`. Hero icon
  contributions are the `hero_icon` kind.
- Files share the private `HERO_ICONS_DIR` (`<id>.png`, mode 0600), the
  startup orphan sweep, and the 503 `hero_icons_disabled` switch.
- `hero_icon` consent is the existing hero icon opt-in
  (`member_settings.hero_icon_opt_in`), so today's opt-ins carry over as
  `hero_icon` consent and nothing else.
- `/api/hero-icons` (see `docs/api/hero-icons.md`) keeps working exactly as
  before for Site. It sees only `hero_icon` rows: another kind's id answers
  404 `icon_not_found` there, and its delete-all removes hero icons only.

## Kinds

| Kind | Stored | Cap per member | Constant |
|------|--------|----------------|----------|
| `hero_icon` | yes | 500 | `HERO_ICON_MAX_PER_MEMBER` |
| `stat_digits` | yes | 1000 | `STAT_DIGITS_MAX_PER_MEMBER` |
| `map_name` | yes | 200 | `MAP_NAME_MAX_PER_MEMBER` |
| `rank` | yes | 1000 | `RANK_MAX_PER_MEMBER` |
| `full_screen` | **no**: refused with 422 `full_screen_not_accepted` | 0 | |

`full_screen` uploads are refused before the body is read and nothing is
written to disk or the database. Its consent cannot be stored either: the
consent shapes have no `full_screen` field, a `PUT` naming it is 400
`invalid_consent`, and `member_settings` has no column for it.

### Crop size per kind

| Kind | Edges (width x height) | Bytes per `crop` part | Constants |
|------|------------------------|-----------------------|-----------|
| `hero_icon`, `stat_digits` | 8 to 256 each | 64 KiB | `HERO_ICON_MAX_EDGE`, `HERO_ICON_MAX_BYTES` |
| `map_name` | 8 to 1536 wide, 8 to 256 tall | 256 KiB | `MAP_NAME_MAX_WIDTH`, `MAP_NAME_MAX_HEIGHT`, `MAP_NAME_MAX_BYTES` |
| `rank` | 8 to 512 wide, 8 to 256 tall | 128 KiB | `RANK_MAX_WIDTH`, `RANK_MAX_HEIGHT`, `RANK_MAX_BYTES` |

Map name banners are wide and the tracker sends them unscaled (a native 4K
banner is 1036 px wide), so `map_name` has the wider bound and a byte limit
to match. The 256 KiB cap leaves headroom for 4K banners (measured max
about 72 KB at 1440p, about 121 KB estimated at 4K). The whole request
stays capped at 2 MiB (`HERO_ICON_MAX_REQUEST_BYTES`) for every kind, so a
request of large banners holds fewer than 20 of them. The bounds live on
`CropKind::max_edges` and `CropKind::max_crop_bytes`.

`rank` crops are regions of the post-game rank screen: role, tier, SR and
the percentile line, never a region with player names. The tracker's rank
reader takes only 1080p and 1440p frames; at 1440p its widest region (the
mode banner) is 440 px and its tallest (the tier emblem) 200 px, so 512 by
256 leaves room above both. 128 KiB is twice the hero icon limit for twice
its widest edge, and half the `map_name` limit for a third of its pixels.
About four regions per competitive game make the 1000 cap about 250 games.

## Auth

The tracker's sync token is the same daemon token stats sync uses
(`Authorization: Bearer <token>`, hashed and matched against active unexpired
`daemon_token` rows; the owner must be an active member who is not suspended
or banned). It is accepted, besides a session, on exactly these routes:

| Route | Session | Sync token |
|-------|---------|------------|
| `POST /api/training-crops?kind=` (upload) | yes | yes |
| `GET /api/training-crops/consent` (own consent) | yes | yes |
| `PUT /api/training-crops/consent` (set own consent) | yes | no (401) |
| `DELETE /api/training-crops/mine` (own delete-all) | yes | yes |
| `DELETE /api/training-crops/{id}` (own single delete) | yes | yes, own crops only |
| `GET /api/training-crops/mine` (own list) | yes | no (401) |
| `GET /api/training-crops/{id}/image` | yes | no (401) |
| `GET /api/training-crops` (admin list) | admin | no (401) |
| `GET /api/training-crops/members` (counts) | officer or admin | no (401) |
| `GET /api/training-crops/resolutions` (totals) | admin | no (401) |
| every `/api/hero-icons` route | as before | no (401) |

Setting consent is session only, so a leaked or misused sync token can never
turn a kind on. The tracker shows the consent read-only and links to the site
to change it.

The four routes that take the sync token share one per-IP limit: burst 40
requests, then one every 2 s (`STATS_UPLOAD_IP_BURST`,
`STATS_UPLOAD_IP_REPLENISH_SECS`, the same numbers as the stats sync IP
limit), in their own bucket. It runs before the credential is checked, so
logged-in requests to those four routes share the per-IP limit too, admin
deletes included; a refused request is the shared 429 `rate_limited`. The
upload also keeps its per-member, per-kind limiter (below).

There is no admin route to delete all of one member's `stat_digits` or
`map_name` crops: an admin removes those one id at a time, inside that
limit. For `hero_icon` there is one, `DELETE /api/hero-icons/members/{member_id}`,
outside this per-IP limit. An admin delete-all of every training kind is a
Site/API follow-up.

A sync token never acts as staff. On `DELETE /api/training-crops/{id}` an
admin's or officer's sync token reaches only that member's own crops; any
other id is 404 `crop_not_found`.

Credential order is the same as `POST /api/stat-reports`: the session is tried
first (cookie, or a session token as Bearer). When that is a 401 and a Bearer
is present, the Bearer is tried as a sync token.

| Status | `error` | When |
|--------|---------|------|
| 401 | `Authentication required` | No cookie and no Bearer |
| 401 | `Unauthorized` | Bearer that is neither a session nor a usable sync token (unknown, revoked or expired; one body for all) |
| 403 | `Not an org member`, `Membership inactive`, `Account suspended` | Session checks, as for every member route |
| 403 | `Member not found`, `Membership inactive`, `Member is suspended or banned` | Sync token checks, as for stats sync |

## Consent

`GET /api/training-crops/consent`:

```json
{ "hero_icon": false, "stat_digits": false, "map_name": false, "rank": false }
```

`PUT /api/training-crops/consent` (session only; a sync token is 401) with
any subset of the four kinds. Kinds left out keep their value. The answer is
all four as stored.

```json
{ "stat_digits": true, "map_name": false }
```

A kind sent as `null` (`{"map_name": null}`) counts as left out and keeps
its value. Unknown kinds (`full_screen` included) or other non-bool values
(`"yes"`, `1`) are 400 `invalid_consent` and change nothing. Both routes work while the store is
disabled.

Turning a kind off works like the hero icon opt-out: stored crops of that kind
are kept and hidden. New uploads of the kind are refused (403
`crop_consent_required`). Admins can no longer read them or see their
metadata, and they leave officer counts. The owner can still list and delete them. An admin
still finds their ids in the admin list (marked `hidden`, without metadata)
and can delete them while the owner is active. Turning the kind on again
makes them visible again.

## Upload

`POST /api/training-crops?kind=<kind>`, `multipart/form-data`.

The body is up to 20 (`HERO_ICON_MAX_FILES_PER_UPLOAD`) pairs of parts, in
order: a part named `meta` (JSON, at most 4 KiB, `TRAINING_CROP_META_MAX_BYTES`)
then a part named `crop` (PNG). Each `meta` belongs to the `crop` right after
it. A `crop` without a `meta` before it, two `meta` parts in a row, a `meta`
at the end, or any other part name is 400 `unexpected_field`. The whole body
is capped at 2 MiB (`HERO_ICON_MAX_REQUEST_BYTES`).

One request carries one kind. The kind is a query field so that the limiter,
consent, the `full_screen` refusal and the cap are all decided before the
body is read.

Check order:

0. 429 `rate_limited` from the per-IP limit of the sync token routes (see
   Auth), before the credential is checked.
1. 401 `Authentication required` when there is no cookie and no Bearer, or
   401 `Unauthorized` when the Bearer is neither a session nor a usable
   sync token (see Auth). 403 membership failures also come from this
   check. The `CropUploader` extractor runs this after the per-IP limit
   and before the handler body.
2. 503 `hero_icons_disabled` when the store is off.
3. 400 `unknown_crop_kind` for a missing or unknown `kind`.
4. 422 `full_screen_not_accepted` for `full_screen`. This wins over the 403
   below: a member without consent who sends `full_screen` gets 422, not
   403 `crop_consent_required`. On purpose: `full_screen` is refused for
   everyone whatever the consent (there is no `full_screen` consent to
   have), so 422 is the true answer, and it comes before the per-member
   limiter, so nothing is read and no per-member rate limit token is
   spent. The per-IP limiter at step 0 has already spent a token.
5. 429 `rate_limited`: per member and per kind, burst 10 requests then one
   every 60 s (`HERO_ICON_RATE_BURST`, `HERO_ICON_RATE_REFILL_SECS`). Each
   kind has its own bucket; `hero_icon` shares the bucket of
   `POST /api/hero-icons`. Refused requests still spend a token.
6. 403 `crop_consent_required` when the caller does not consent to the kind.
7. 409 `crop_limit_reached` when the caller is already at the kind's cap.
8. 400 `invalid_multipart` when the body is not readable multipart.
9. The parts, in order. Each `meta`: 400 `invalid_crop_meta` or
   `unknown_hero`. Each `crop`: 415 `not_png`, 413 `icon_too_large` (over
   the kind's byte limit), 400 `icon_bad_dimensions` (outside the kind's
   edges, see Crop size per kind), 400 `invalid_png`. A 21st pair is 413
   `too_many_crops_in_request`. A body over 2 MiB is 413 `icon_too_large`.
10. 400 `no_crops` when there was no pair.
11. 409 `crop_limit_reached` when this request would pass the cap.

Nothing is stored unless every part passes. After storing, the cap and the
consent are checked again: if another upload pushed the caller over the cap
(409) or consent was withdrawn while the body was read (403), this request's
rows and files are removed again.

The PNG is decoded and re-encoded before it is stored, as for hero icons, so
text chunks and other metadata in the file are dropped.

The tracker version is taken from a `User-Agent` of the form
`scuffed-stat-tracker/<semver>` (same parsing as the stats sync legacy-id
gate of PR 649) and stored as `tracker_version`. A missing, foreign or
unparseable User-Agent stores no version; so does a version longer than 64
characters or one holding a control character. None of these is an error.

### `meta` fields

Every field is optional unless noted. Unknown fields are refused (400
`invalid_crop_meta`), so a typo never silently drops data.

| Field | Type | Rule |
|-------|------|------|
| `hero` | string | Hero key or display name (`ana`, `lucio`, `soldier-76`, `Soldier: 76`), checked against the known heroes (`HERO_NAMES`). **Required for `hero_icon`.** Optional for other kinds. Unknown hero: 400 `unknown_hero`. Answered as the hero key, the pack file stem (`soldier-76`, `scuffed_types::hero_key`). `hero_icon` rows store the display name, as before; other kinds store the key. `/api/hero-icons` still answers with the display name |
| `screen` | string | 1 to 32 chars of `A-Z a-z 0-9 _ - .` |
| `slot_type` | string | Same as `screen` |
| `slot_index` | int | 0 to 63 |
| `game_token` | string | Exactly 32 lowercase hex characters (`TRAINING_CROP_GAME_TOKEN_LEN`). Must be random (for example UUID v4); the server checks only the shape. A dashless UUIDv7 also matches that shape and would carry a time, so the admin list groups visible crops by a SHA-256 of the token, not the raw value. The server never links it to a match and never checks it against stored games |
| `score` | float | Finite, absolute value at most 1000 |
| `runner_up` | float | Same as `score` |
| `margin` | float | Same as `score` |
| `icon_set_version` | string | 1 to 64 chars of `A-Z a-z 0-9 _ - . +` |
| `spot_version` | string | Same as `icon_set_version` |
| `vote_card_filled` | bool | |
| `locked_in` | bool | |
| `screen_width`, `screen_height` | int | Game resolution. Both or neither, each 320 to 16384 |
| `ui_scale` | float | Above 0, at most 200 (same range as the report bundle) |
| `found_spot` | object `{x, y, w, h}` | Unsigned ints, `w` and `h` at least 1, the box inside the resolution when one is given (inside 16384 otherwise). Unknown keys refused. Sending it only for unknown screen sizes is the tracker's rule: the server accepts it at any resolution, since it cannot know the tracker's layouts |
| `found_confidence` | float | 0 to 1. `found_spot` and `found_confidence` come together or not at all |

Any bound failure is the one stable code 400 `invalid_crop_meta`; the server
logs the field name at debug level.

The server takes any `slot_type` of that shape. The tracker sends these,
all with `screen` `tab`:

| Kind | `slot_type` | What the crop is |
|------|-------------|------------------|
| `hero_icon` | `career_panel` | The career panel's hero portrait, with `hero`, `score`, `runner_up`, `margin` and the hero pack name as `icon_set_version` |
| `stat_digits` | `elims`, `assists`, `deaths`, `damage`, `healing`, `mitigation` | One stat cell of the member's own row, named by its column left to right, with `hero` only when that Tab's own read named it |
| `map_name` | none | The map name banner |
| `rank` | tracker's choice | One rank screen region (role, tier, SR, percentile). No player names |

There is no colourblind filter field. It hints at a member's colour vision
and is not in the agreed field list or the consent text, so it is not
accepted (a `meta` naming `colorblind_filter` is 400 `invalid_crop_meta`,
like any unknown field) and there is no column for it.

Example request (curl):

```sh
curl -X POST "$SITE/api/training-crops?kind=stat_digits" \
  -H "Authorization: Bearer $SYNC_TOKEN" \
  -H "User-Agent: scuffed-stat-tracker/0.6.0-alpha.3" \
  -F 'meta={"screen":"scoreboard","slot_type":"ally","slot_index":2,"game_token":"4f2a9c1e7b0d3e5f6a7b8c9d0e1f2a3b","score":0.93,"runner_up":0.41,"margin":0.52,"screen_width":1920,"screen_height":1080};type=application/json' \
  -F 'crop=@digits.png;type=image/png'
```

`201 Created`, crops in the order sent:

```json
{
  "kind": "stat_digits",
  "crops": [
    {
      "id": "3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c",
      "kind": "stat_digits",
      "size_bytes": 412,
      "width": 40,
      "height": 16,
      "uploaded_at": "2026-11-10T18:04:05.123456Z",
      "tracker_version": "0.6.0-alpha.3",
      "meta": {
        "screen": "scoreboard", "slot_type": "ally", "slot_index": 2,
        "game_token": "4f2a9c1e7b0d3e5f6a7b8c9d0e1f2a3b", "score": 0.93, "runner_up": 0.41,
        "margin": 0.52, "screen_width": 1920, "screen_height": 1080
      },
      "thumbnail_url": "/api/training-crops/3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c/image"
    }
  ]
}
```

`hero` and `tracker_version` are left out when not set; `meta` leaves out
every field that was not sent.

## Own crops

`GET /api/training-crops/mine?kind=&cursor=&limit=` (session only). Own crops
of every kind, consented or not, newest first, with the usual cursor
(default 25, max 100):

```json
{
  "consent": { "hero_icon": true, "stat_digits": true, "map_name": false, "rank": false },
  "kinds": [
    { "kind": "hero_icon", "count": 12, "consented": true, "max": 500 },
    { "kind": "stat_digits", "count": 40, "consented": true, "max": 1000 },
    { "kind": "map_name", "count": 0, "consented": false, "max": 200 },
    { "kind": "rank", "count": 0, "consented": false, "max": 1000 }
  ],
  "crops": [ /* TrainingCropEntry, as in the upload answer */ ],
  "next_cursor": "00000019"
}
```

`DELETE /api/training-crops/mine?kind=` (session or sync token). Removes every
own crop, or every own crop of one kind, rows and files. Consent is left as
it is. `{"deleted": n}`. Each file goes before its row; a file that cannot be
removed keeps its row and the answer is a 500 after the others went, so the
call can be retried. The delete reads ids only, never whole rows, so a row
that would not parse (and so is missing from the lists) is deleted and
counted too. The same holds for the single delete below and for
`DELETE /api/hero-icons/mine`.

`DELETE /api/training-crops/{id}` (session or sync token). `{"deleted": 1}`.
The file goes first. With a session, an admin may also delete another active
member's crop of any kind, consent or not, using an id from the admin list
(audited: `deleted_hero_icon` for a hero icon, `deleted_training_crop` for
any other kind, details `"crop <id> kind <kind>"`); an officer gets 403
`Admin access required`. Everyone else, and every sync token, gets 404
`crop_not_found` for an id that is not their own. A row whose stored kind
is unknown is audited as `deleted_training_crop` with kind `unknown`.

`GET /api/training-crops/{id}/image` (session only). The PNG, with
`Content-Type: image/png`, `X-Content-Type-Options: nosniff` and
`Cache-Control: private, no-store`. The owner, and an admin while the owner is
active and consents to that crop's kind. Officer: 403 `Admin access
required`. Everyone else: 404 `crop_not_found`.

## Staff

`GET /api/training-crops?kind=&cursor=&limit=` (admin). Every stored crop of
active members, of every kind or of one kind with `?kind=`, so an admin can
find the id of any crop and delete it. Sorted by the upload hour (newest
first), then visible crops before hidden ones, then a SHA-256 of
`game_token` for visible crops that have one (rows with no token first
in the hour), then id: stable, visible crops of one game stay together,
hidden crops are not sorted into that group, and nothing shows the order
within an hour. Each item is a `TrainingCropEntry` plus `member_id`,
`display_name` and `hidden`:

- `hidden: false`: the owner consents to this kind right now. Every field is
  sent and the image can be read.
- `hidden: true`: consent for this kind is off. Only `id`, `kind`,
  `uploaded_at` and `thumbnail_url` carry values. `size_bytes`, `width` and
  `height` are 0, since a crop's size can give away the game resolution
  (map banners are sent unscaled). `meta` is `{}`, so `game_token` is
  omitted too, there is no `hero` or `tracker_version`, and the image
  answers 404. Hidden crops of the same hour sort after the visible ones,
  by id only, so a shared `game_token` cannot group them with visible
  crops of that game. The id is there so the crop can be deleted.

`uploaded_at` in this list is cut down to the start of its UTC hour, so the
times are rough. That does not make a crop unlinkable to a game: admins
still see exact upload times of hero icons on `/api/hero-icons`, and stat
digit values may match a game. The owner's own list keeps the exact time.

One audit entry per request, target `all`: `viewed_hero_icons` when the list
is filtered to `hero_icon`, `viewed_training_crops` otherwise, details
`"<n> training crops, <kind or all kinds>"`.

`GET /api/training-crops/members?cursor=&limit=` (officer or admin). Counts
by kind only, no ids, images or resolutions. Active members sorted by display
name, then member id:

```json
{
  "data": [
    {
      "member_id": "m1",
      "display_name": "Ana Main",
      "total": 3,
      "kinds": [
        { "kind": "stat_digits", "count": 3, "consented": true, "max": 1000 }
      ]
    }
  ]
}
```

- Officers get only the kinds a member consents to (with a zero count when
  nothing is stored yet), and only members who consent to at least one kind.
- Admins also get kinds with consent off that still have stored crops, with
  `consented` false. Their ids are in the admin list (`hidden: true`), so
  they can be deleted.
- There is no per-member breakdown by game resolution, for officers or
  admins: it would show each member's setup.

`GET /api/training-crops/resolutions` (admin). Stored crops by kind and game
resolution, summed across active members, counting only the kinds each
member consents to. Largest count first; crops sent without a resolution
are one row with no `screen_width`/`screen_height`:

```json
{
  "resolutions": [
    { "kind": "stat_digits", "screen_width": 1920, "screen_height": 1080, "count": 2 },
    { "kind": "stat_digits", "screen_width": 2560, "screen_height": 1440, "count": 1 },
    { "kind": "map_name", "count": 1 }
  ]
}
```

Officers get 403 `Admin access required`. Matched against the exact
per-member counts of `/members`, these totals would give each member's
resolution (two members, 40 and 7 crops, and totals of 40 at 1080p and 7 at
1440p). Admins lose nothing by this: they already see each consented crop's
game resolution in the admin list.

## Release order

1. The API (this contract) ships first.
2. Then Site's consent page, so members can turn kinds on.
3. The first tracker build that sends `stat_digits` or `map_name` also ships
   a delete button that calls `DELETE /api/training-crops/mine`, in the same
   release. Site's "delete my contributions" calls `DELETE /api/hero-icons/mine`,
   which removes hero icons only, so without that button a member could not
   remove what the build sends.

## Errors

Every error body is `{"error": "<code>"}`, except the shared 429 body
`{"code":"rate_limited","error":"rate_limited","retry_after":N}` with
`Retry-After`. New codes are `TRAINING_CROP_ERR_*` in
`crates/types/src/api/training_crops.rs`; the PNG checks reuse
`HERO_ICON_ERR_*`.

| Status | `error` | When |
|--------|---------|------|
| 400 | `unknown_crop_kind` | `kind` missing or unknown (upload), or unknown (`?kind=` filters) |
| 422 | `full_screen_not_accepted` | Upload with `kind=full_screen`, with or without consent (wins over 403, see Check order). Nothing read or stored |
| 403 | `crop_consent_required` | Upload of a kind the caller does not consent to. Nothing stored |
| 400 | `invalid_crop_meta` | A `meta` part that is not JSON, over 4 KiB, has an unknown field (`colorblind_filter` included), or a value out of bounds |
| 400 | `unknown_hero` | `hero` not a known hero, or missing on a `hero_icon` crop |
| 400 | `unexpected_field` | Part named other than `meta`/`crop`, or pairs out of order |
| 400 | `no_crops` | No `meta`/`crop` pair |
| 400 | `invalid_multipart` | Body is not readable multipart |
| 400 | `icon_bad_dimensions` | PNG width or height outside the kind's bound (8 to 256; `map_name` 8 to 1536 wide, 8 to 256 tall; `rank` 8 to 512 wide, 8 to 256 tall) |
| 400 | `invalid_png` | PNG signature present but the image does not decode |
| 400 | `invalid_consent` | Consent body not JSON, unknown kind (`full_screen` included), or not a bool |
| 401 | session error (as for every member route) | `PUT /consent` without a session; a sync token is not a session there |
| 403 | `Admin access required` | Officer session on another member's image or delete; non-admin on the admin list or `/resolutions` |
| 403 | `Officer access required` | Member session on `/members` |
| 404 | `crop_not_found` | Unknown id, malformed id, or a crop the caller may not see |
| 409 | `crop_limit_reached` | The upload would pass the kind's cap |
| 413 | `icon_too_large` | A `crop` part over the kind's limit (64 KiB; `map_name` 256 KiB; `rank` 128 KiB), or the body over 2 MiB |
| 413 | `too_many_crops_in_request` | More than 20 pairs |
| 415 | `not_png` | A `crop` part without the PNG signature |
| 429 | `rate_limited` | Per-member, per-kind upload limiter, or the per-IP limit of the sync token routes |
| 503 | `hero_icons_disabled` | Store off (every route except consent) |

## Storage, migration and rollback

Additive only (`crates/db/src/migrations.rs`):

- `hero_icon.kind`: `string`, `DEFAULT 'hero_icon'`, asserted to
  `hero_icon`, `stat_digits`, `map_name` or `rank`. A fatal startup step
  (`hero_icon kind backfill`) writes `kind = 'hero_icon'` on rows that have
  none, so existing rows become hero icons. A no-op after the first boot.
- `hero` keeps its type. `hero_icon` rows keep holding the display name
  (`Soldier: 76`), as before this change; rows of the other kinds hold the
  hero key (`soldier-76`). Reads map both spellings to the key, and the hero
  icon routes answer with the display name either way.
- New optional `hero_icon` columns: `screen`, `slot_type`, `slot_index`,
  `game_token`, `score`, `runner_up`, `margin`, `icon_set_version`,
  `spot_version`, `vote_card_filled`, `locked_in`, `screen_width`,
  `screen_height`, `ui_scale`, `tracker_version`, `found_x`, `found_y`,
  `found_w`, `found_h`, `found_confidence`. Existing rows leave them NONE.
  There is no `colorblind_filter` column. `hero` stays a plain string; crops of other kinds
  store it empty when no hero was sent.
- Index `hero_icon_member_kind_idx` on `(member_id, kind)`.
- `member_settings.stat_digits_opt_in`, `map_name_opt_in`: `bool DEFAULT
  false`. Older rows read false. `hero_icon_opt_in` is untouched and is the
  `hero_icon` consent. There is no `full_screen_opt_in`: that consent is
  never stored.
- `member_settings.rank_opt_in`: `bool DEFAULT false`, added with the
  `rank` kind. Additive: no row is rewritten, rows from before it read
  false. A build without the `rank` kind ignores the field and skips
  `rank` rows on read (an unknown stored kind is skipped); to remove them
  on rollback, use step 1 below, which covers every kind but `hero_icon`.

Take a database snapshot before deploying this change. Restoring it is only
for a rollback right after the deploy: it rolls back every table, so once
members have used the site it loses their new data (members, applications,
stats). Later, roll back with the steps below instead.

Rollback to the build before this change:

1. Before deploying the old build, remove crops of the new kinds, files
   first. This deletes members' contributed `stat_digits` and `map_name`
   crops for good, so it needs FrozenTear's yes first: list `SELECT id, file_name FROM hero_icon WHERE kind != 'hero_icon'`,
   delete those files from `HERO_ICONS_DIR`, then
   `DELETE hero_icon WHERE kind != 'hero_icon'`. The old build has no kind
   filter and would otherwise show them as hero icons with an empty hero.
   (Files left behind are also removed by the old build's orphan sweep after
   one hour once their rows are gone.)
2. Deploy the old build. Its migrations redefine only the old fields; the new
   columns, the index and the consent fields stay defined and unused, and
   hero icon rows keep working: the old build reads rows with its own row
   struct, which picks its fields by name and ignores the others, so a row
   carrying `kind` and the new columns reads as before (checked by the db
   test `old_build_row_shape_reads_rows_with_kind_and_metadata`, which reads
   such a row with a copy of the old struct). Hero icons the old build
   writes get `kind = 'hero_icon'` from the field default.
3. Optional cleanup:
   `REMOVE INDEX hero_icon_member_kind_idx ON hero_icon;` and
   `REMOVE FIELD <name> ON hero_icon;` / `ON member_settings` for each new
   field. Not needed for the old build to run.

Hero icon opt-ins are never rewritten, so a rollback keeps every member's
hero icon choice.
