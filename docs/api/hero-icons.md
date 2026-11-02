# Hero icon contributions

Members can contribute PNG hero icons so the stat tracker can build
icon-matching templates for hero recognition. Contribution is opt-in, off by
default, and every route needs a signed-in session. JSON types live in
`crates/types/src/api/hero_icons.rs`; the numeric caps live in
`crates/types/src/hero_icons.rs`.

## Privacy rule

- Icons are stored only on the server volume `HERO_ICONS_DIR`. They are never
  written to the upload tree, never served as static files, never committed
  and never public.
- Image bytes are served only by `GET /api/hero-icons/{id}/image`, with a
  session, to the owner and to officers. An officer can read a member's icons
  only while that member is opted in.
- Turning opt-in off does not delete anything. It stops new uploads at once,
  and officers can no longer list or read that member's icons. The member can
  still list, read and delete their own icons, and can delete all of them with
  one call.
- Deleting an icon removes both the database row and the file.
- The `hero-icons-data` volume is part of the server backup snapshot
  (`scripts/backup.sh`), like `reports-data`. A deleted icon can stay in an
  older snapshot until restic prunes it.

## Caps

| Constant | Value | Meaning |
|----------|-------|---------|
| `HERO_ICON_MAX_BYTES` | 65536 (64 KiB) | Largest PNG in one `icon` part |
| `HERO_ICON_MAX_FILES_PER_UPLOAD` | 20 | Most `icon` parts in one request |
| `HERO_ICON_MAX_PER_MEMBER` | 500 | Most icons one member may have stored |
| `HERO_ICON_MIN_EDGE` | 8 | Smallest width or height, in pixels |
| `HERO_ICON_MAX_EDGE` | 256 | Largest width or height, in pixels |
| `HERO_ICON_RATE_BURST` | 10 | Upload requests per member before the limiter waits |
| `HERO_ICON_RATE_REFILL_SECS` | 60 | Seconds until the limiter gives back one upload request |
| `HERO_ICON_MAX_REQUEST_BYTES` | 2097152 (2 MiB) | Body cap of one upload request |

The rate limit is a per-member token bucket kept in server memory, so a restart
resets it. It counts every upload request that reaches the handler, including
refused ones.

## Validation

Each `icon` part is checked in this order. Nothing is stored unless every part
in the request passes.

1. The part starts with the 8-byte PNG signature. Otherwise 415 `not_png`.
2. The part is at most `HERO_ICON_MAX_BYTES`. Otherwise 413 `icon_too_large`.
3. The PNG header gives a width and height in `8..=256`. Otherwise 400
   `icon_bad_dimensions`.
4. The image decodes. Otherwise 400 `invalid_png`.

The server then re-encodes the decoded pixels as a new PNG and stores that, so
text chunks and other metadata from the upload are dropped. `size_bytes` is the
size of the stored file. File names on disk are `<id>.png`, where `id` is 32
lowercase hex characters made by the server. Nothing in a path comes from the
request.

The hero comes from the `hero` query parameter and must be a known hero from
`scuffed_types::HERO_NAMES`. Keys and display names are both accepted, accents
and punctuation are ignored (`ana`, `lucio`, `Lúcio`, `soldier-76`). The stored
and returned value is the display name, for example `"Lúcio"`.

## Errors

Every error body is `{"error": "<code>"}`. The codes are constants in
`crates/types/src/api/hero_icons.rs` (`HERO_ICON_ERR_*`).

| Status | `error` | When |
|--------|---------|------|
| 401 | `Authentication required` | No session |
| 403 | `Officer access required` | Officer route, caller is not an officer |
| 403 | `icon_opt_in_required` | Upload while the caller is not opted in |
| 403 | `icon_opt_in_off` | Officer asks for the icons of a member who is not opted in |
| 404 | `icon_not_found` | Unknown id, malformed id, or an icon the caller may not see |
| 400 | `unknown_hero` | `hero` missing or not a known hero |
| 400 | `no_icons` | Upload with no `icon` part |
| 400 | `unexpected_field` | A multipart part not named `icon` |
| 400 | `invalid_multipart` | Body is not readable multipart |
| 400 | `icon_bad_dimensions` | Width or height outside 8 to 256 |
| 400 | `invalid_png` | PNG signature present but the image does not decode |
| 409 | `icon_limit_reached` | The upload would go over `HERO_ICON_MAX_PER_MEMBER` |
| 413 | `icon_too_large` | A part over 64 KiB, or the body over 2 MiB |
| 413 | `too_many_icons_in_request` | More than 20 `icon` parts |
| 415 | `not_png` | A part without the PNG signature |
| 429 | `rate_limited` | Per-member upload limiter, see below |
| 503 | `hero_icons_disabled` | `HERO_ICONS_DIR` could not be used at startup |

The 429 is the shared rate-limit response: body
`{"code":"rate_limited","error":"rate_limited","retry_after":N}`, plus the
headers `Retry-After: N`, `x-ratelimit-after: N` and `Cache-Control: no-store`.

A sign-in check (401, 403 `Membership inactive`, 403 `Officer access required`)
comes before the 503.

## Endpoints

Times are RFC 3339 UTC, for example `"2026-11-02T18:04:05.123456Z"`, the same
format as other API timestamps.

### Upload

`POST /api/hero-icons?hero=<key>` (any active member, session)

Multipart body, one or more parts named `icon`, each a PNG file. Check order:
503, 429, 403 `icon_opt_in_required`, 400 `unknown_hero`, 409 when the member is
already at the cap, then the parts.

`201 Created`:

```json
{
  "icons": [
    {
      "id": "3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c",
      "hero": "Ana",
      "size_bytes": 1834,
      "width": 64,
      "height": 64,
      "uploaded_at": "2026-11-02T18:04:05.123456Z",
      "thumbnail_url": "/api/hero-icons/3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c/image"
    }
  ]
}
```

### My icons

`GET /api/hero-icons/mine` (any active member)

Lists the caller's icons, newest first, opted in or not.

```json
{
  "icon_contribution_opt_in": true,
  "count": 1,
  "max_icons": 500,
  "icons": [
    {
      "id": "3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c",
      "hero": "Ana",
      "size_bytes": 1834,
      "width": 64,
      "height": 64,
      "uploaded_at": "2026-11-02T18:04:05.123456Z",
      "thumbnail_url": "/api/hero-icons/3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c/image"
    }
  ]
}
```

`DELETE /api/hero-icons/mine` (any active member)

Deletes every icon of the caller, rows and files. Opt-in is left as it is.

```json
{ "deleted": 3 }
```

### Opt-in

`GET /api/hero-icons/opt-in` and `PUT /api/hero-icons/opt-in` (any active member)

The PUT body and both responses:

```json
{ "icon_contribution_opt_in": true }
```

Default `false`. The PUT changes only this flag; the tracker player name and
stats visibility in the same settings row are kept. Turning it off keeps the
icons.

### Thumbnail

`GET /api/hero-icons/{id}/image` (owner, or an officer while the owner is opted in)

`200` with the PNG bytes and these headers:

```
Content-Type: image/png
Content-Disposition: inline
X-Content-Type-Options: nosniff
Cache-Control: private, no-store
```

Anyone else, an unknown id and a malformed id (anything that is not 32
lowercase hex characters, such as `..%2Fetc%2Fpasswd`) all get 404
`icon_not_found`, so the route does not reveal whether an id exists.

### Delete one

`DELETE /api/hero-icons/{id}` (owner, or any officer)

Officers can delete any icon, also while its owner is opted out. Anyone else
gets 404 `icon_not_found`. Officer deletes of another member's icon are written
to the audit log as `deleted_hero_icon`.

```json
{ "deleted": 1 }
```

### Officer: all icons

`GET /api/hero-icons?limit=25&cursor=<next_cursor>` (officer)

Icons of members who are opted in right now, newest first, in the usual cursor
page (`limit` 1 to 100, default 25). `next_cursor` is left out on the last page.

```json
{
  "data": [
    {
      "id": "3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c",
      "member_id": "k2l9x0q7w1",
      "display_name": "FrozenTear",
      "hero": "Ana",
      "size_bytes": 1834,
      "width": 64,
      "height": 64,
      "uploaded_at": "2026-11-02T18:04:05.123456Z",
      "thumbnail_url": "/api/hero-icons/3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c/image"
    }
  ],
  "next_cursor": "opaque-cursor"
}
```

### Officer: per-member counts

`GET /api/hero-icons/members` (officer)

Every member with stored icons or an opt-in, sorted by display name. Members
who opted out are counted, but their icons cannot be listed or read.

```json
{
  "members": [
    {
      "member_id": "k2l9x0q7w1",
      "display_name": "FrozenTear",
      "icon_count": 12,
      "icon_contribution_opt_in": true
    }
  ]
}
```

`display_name` is an empty string when the member record has no name.

### Officer: one member's icons

`GET /api/hero-icons/members/{member_id}` (officer)

All icons of one member, newest first, not paged (at most 500). 403
`icon_opt_in_off` while that member is not opted in.

```json
{
  "icons": [
    {
      "id": "3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c",
      "member_id": "k2l9x0q7w1",
      "display_name": "FrozenTear",
      "hero": "Ana",
      "size_bytes": 1834,
      "width": 64,
      "height": 64,
      "uploaded_at": "2026-11-02T18:04:05.123456Z",
      "thumbnail_url": "/api/hero-icons/3f2a9c0e6b1d4e8f9a7c5b3d1e0f2a4c/image"
    }
  ]
}
```

## Server configuration

| Variable | Default | Compose value |
|----------|---------|---------------|
| `HERO_ICONS_DIR` | `data/hero-icons` | `/app/data/hero-icons` on the named volume `hero-icons-data` |

The server creates the directory with mode 0700 and writes each file with mode
0600. The image entrypoint gives the directory to the `scuffed` user first, as
it does for `REPORTS_DIR`. If the directory is missing, not writable, or
overlaps `UPLOAD_DIR`, `REPORTS_DIR` or `dist`, the server logs
`hero icons disabled: ...`, keeps serving, and every hero icon route answers
503 `hero_icons_disabled` until the directory is fixed and the server restarts.
See `docs/deploy.md`.

## Storage

- `member_settings.hero_icon_opt_in` (bool, default false). Rows written before
  the field existed read as false.
- Table `hero_icon`: `member_id`, `hero`, `file_name`, `size_bytes`, `width`,
  `height`, `created_at`, indexed on `member_id`. The record id is the icon id.
