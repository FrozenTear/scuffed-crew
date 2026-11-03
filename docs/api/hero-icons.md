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
  session, to the owner and to officers. An officer can see a member's icons
  only while that member is active and opted in.
- Deleting an icon removes both the database row and the file.
- The `hero-icons-data` volume is deliberately not in the backup snapshot:
  icons never leave the server volume. Lost icons can be contributed again.

## Opt-out

Decision of 2026-11-03: "when a member turns the opt-in off, the icons
already stored are KEPT and hidden (not served to officers or anyone else).
They are deleted only when the member explicitly deletes them (single delete
or delete-all)."

So while a member is opted out:

- New uploads are refused (403 `icon_opt_in_required`).
- Officers cannot list, count, read or delete that member's icons. The
  member is left out of `GET /api/hero-icons` and
  `GET /api/hero-icons/members`. `GET` and `DELETE` on
  `/api/hero-icons/members/{member_id}` answer 403 `icon_opt_in_off` and
  delete nothing. `GET /api/hero-icons/{id}/image` and
  `DELETE /api/hero-icons/{id}` answer 404 `icon_not_found`, the same as for
  an id that does not exist.
- The member can still list, read and delete their own icons, one at a time
  with `DELETE /api/hero-icons/{id}` or all at once with
  `DELETE /api/hero-icons/mine`. Nothing else deletes them.
- Opting in again makes the kept icons visible to officers again.

Opting in and out (`GET` and `PUT /api/hero-icons/opt-in`) works even when the
icon store is disabled, since it only reads or flips the flag.

Inactive members are treated the same way for officers: their icons are kept
and hidden, and the per-member routes answer 404 `member_not_found`.

An officer delete that races with an opt-out: the officer delete-all checks the
member again before each icon and stops at the next icon once the member has
opted out or gone inactive. It answers 403 `icon_opt_in_off` when nothing was
deleted yet, otherwise the count deleted so far, and the rest stay stored. A
single officer delete checks once, so an opt-out landing in the few
milliseconds between that check and the file delete does not stop it. The
icon was visible to officers when the delete was accepted.

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
refused ones. Once the limiter holds more than 1024 buckets, buckets that have
refilled completely are dropped on the next upload. A full bucket acts like a
new one, so this changes no answer. Keys are member ids, so what remains is
bounded by the members who uploaded within the last ten minutes.

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
| 401 | `Authentication required` | No session cookie or bearer token |
| 401 | `Session expired or invalid` | Expired or unknown session |
| 403 | `Not an org member` | Signed in, but the user has no member record |
| 403 | `Membership inactive` | The member record is not active |
| 403 | `Account suspended` | The member is suspended |
| 403 | `Officer access required` | Officer route, caller is not an officer |
| 403 | `icon_opt_in_required` | Upload while the caller is not opted in |
| 403 | `icon_opt_in_off` | Officer per-member route for a member who is not opted in |
| 404 | `icon_not_found` | Unknown id, malformed id, or an icon the caller may not see |
| 404 | `member_not_found` | Officer per-member route for an unknown or inactive member |
| 400 | `unknown_hero` | `hero` missing or not a known hero |
| 400 | `no_icons` | Upload with no `icon` part |
| 400 | `unexpected_field` | A multipart part not named `icon` |
| 400 | `invalid_multipart` | Body is not `multipart/form-data` or is not readable multipart |
| 400 | `icon_bad_dimensions` | Width or height outside 8 to 256 |
| 400 | `invalid_png` | PNG signature present but the image does not decode |
| 409 | `icon_limit_reached` | The upload would go over `HERO_ICON_MAX_PER_MEMBER` |
| 413 | `icon_too_large` | A part over 64 KiB, or the body over 2 MiB |
| 413 | `too_many_icons_in_request` | More than 20 `icon` parts |
| 415 | `not_png` | A part without the PNG signature |
| 429 | `rate_limited` | Per-member upload limiter, see below |
| 503 | `hero_icons_disabled` | `HERO_ICONS_DIR` could not be used at startup (every route except opt-in) |

The 429 is the shared rate-limit response: body
`{"code":"rate_limited","error":"rate_limited","retry_after":N}`, plus the
headers `Retry-After: N`, `x-ratelimit-after: N` and `Cache-Control: no-store`.

The sign-in checks (the 401 and the extractor 403 rows above) come before the
503. One body is not JSON from this API: a PUT to `/api/hero-icons/opt-in`
whose body is not the JSON object below gets axum's plain-text 400, 415 or 422.

## Endpoints

Times are RFC 3339 UTC, for example `"2026-11-02T18:04:05.123456Z"`, the same
format as other API timestamps.

Every icon list (mine, officer all icons, officer one member) is newest
first: by `uploaded_at` descending, then by id descending. Within one upload
request each part gets an `uploaded_at` one microsecond after the part before
it, so within one request the last part comes first. The upload answer itself
lists the icons in part order.

### Upload

`POST /api/hero-icons?hero=<key>` (any active member, session)

Multipart body, one or more parts named `icon`, each a PNG file. Check order:
503, 429, 403 `icon_opt_in_required`, 400 `unknown_hero`, 409 when the member is
already at the cap, 400 `invalid_multipart` when the body is not multipart, then
the parts. After storing, the server checks the cap and the opt-in once more:
if another upload pushed the member over the cap (409) or the member opted out
while the body was read (403 `icon_opt_in_required`), this request's icons are
removed again. Icons stored by earlier requests are kept, as for any opt-out.

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
Each file goes before its row. If a file cannot be removed, its row is kept
(the icon stays listed), the other icons are still removed, and the answer is
500 `Internal error`, so the call can simply be repeated.

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
stats visibility in the same settings row are kept. It is one atomic upsert of
the settings row. Turning it off keeps the icons and hides them, see
[Opt-out](#opt-out). Both calls work while the store is disabled.

### Thumbnail

`GET /api/hero-icons/{id}/image` (owner, or an officer while the owner is active and opted in)

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

Thumbnail fetches are not written to the audit log. One page of an officer
list loads up to 100 thumbnails, and the list read itself is audited.

### Delete one

`DELETE /api/hero-icons/{id}` (owner, or an officer while the owner is active and opted in)

The owner can always delete their own icon. An officer can delete another
member's icon only while that member is active and opted in. Otherwise, and
for anyone else, the answer is 404 `icon_not_found` and the icon stays
stored. Officer deletes of another member's icon are written to the audit log
as `deleted_hero_icon` (see [Audit log](#audit-log)).

```json
{ "deleted": 1 }
```

### Officer: all icons

`GET /api/hero-icons?limit=25&cursor=<next_cursor>` (officer)

Icons of members who are active and opted in right now, newest first, in the
usual cursor page (`limit` 1 to 100, default 25). `next_cursor` is left out on
the last page. Each request writes one audit entry `viewed_hero_icons` with
target member `all`.

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

`GET /api/hero-icons/members?limit=25&cursor=<next_cursor>` (officer)

Active members who are opted in, with how many icons each has stored (0 is
possible), sorted by display name (case folded), then member id. The usual
cursor page (`limit` 1 to 100, default 25). Opted-out and inactive members are
not listed, so `icon_contribution_opt_in` is always `true`. Counts are not
audited: they carry no icon.

```json
{
  "data": [
    {
      "member_id": "k2l9x0q7w1",
      "display_name": "FrozenTear",
      "icon_count": 12,
      "icon_contribution_opt_in": true
    }
  ],
  "next_cursor": "opaque-cursor"
}
```

`display_name` is an empty string when the member record has no name.

### Officer: one member's icons

`GET /api/hero-icons/members/{member_id}` (officer)

All icons of one member, newest first, not paged (at most 500). 404
`member_not_found` for an unknown or inactive member, then 403
`icon_opt_in_off` while that member is not opted in. An officer asking for
their own id gets their icons opted in or not. A read of another member's
list writes one audit entry `viewed_hero_icons` with that member as target.

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

### Officer: delete all of one member's icons

`DELETE /api/hero-icons/members/{member_id}` (officer)

Removes every icon of that member, rows and files, while that member is active
and opted in. 404 `member_not_found` for an unknown or inactive member and 403
`icon_opt_in_off` for an opted-out member, and nothing is deleted then. An
officer may clear their own icons here opted in or not. Opt-in is left as it
is. Visibility is checked again before each icon, so a member who opts out
mid-call keeps the icons not yet removed (see [Opt-out](#opt-out)). File
failures work as in `DELETE /api/hero-icons/mine`. When at least one
icon went and the member is not the caller, one audit entry
`deleted_all_hero_icons` is written. A member with no icons gives
`{ "deleted": 0 }`.

```json
{ "deleted": 12 }
```

## Audit log

Officer actions on another member's icons. Every entry has target type
`member`. The owner's own actions are not audited.

| Action | Target id | Details | When |
|--------|-----------|---------|------|
| `viewed_hero_icons` | `all` | `"<n> icons"` on the page | `GET /api/hero-icons` |
| `viewed_hero_icons` | member id | `"<n> icons"` | `GET /api/hero-icons/members/{member_id}` |
| `deleted_hero_icon` | owner member id | `"icon <id>"` | `DELETE /api/hero-icons/{id}` |
| `deleted_all_hero_icons` | member id | `"<n> icons"` | `DELETE /api/hero-icons/members/{member_id}`, when n > 0 |

Thumbnail fetches and the per-member counts are not audited.

## Server configuration

| Variable | Default | Compose value |
|----------|---------|---------------|
| `HERO_ICONS_DIR` | With `PRODUCTION` set: `data/hero-icons` relative to the working directory. Otherwise `scuffed-hero-icons` under the system temp dir, so a dev server never writes icons into the repo tree | `/app/data/hero-icons` on the named volume `hero-icons-data` |

The server creates the directory with mode 0700 and writes each file with mode
0600. The image entrypoint gives the directory to the `scuffed` user first, as
it does for `REPORTS_DIR`. If the directory is missing, not writable, or
overlaps `UPLOAD_DIR`, `REPORTS_DIR`, `PACKS_DIR` or `dist`, the server logs
`hero icons disabled: ...`, keeps serving, and every hero icon route answers
503 `hero_icons_disabled` until the directory is fixed and the server restarts.
The opt-in routes keep working. See `docs/deploy.md`.

## Orphan sweep

Once at startup, when the store is enabled, the server removes icon files that
have no `hero_icon` row and are older than one hour (a crash between the file
write and the row insert can leave one). Only plain files directly in
`HERO_ICONS_DIR` named `<32 lowercase hex>.png` are looked at. Symlinks,
directories and other names are left alone and never followed. Uploads write
straight to `<id>.png` with no temp file, so there are no other names to
clean. The count is logged at info level (`hero icon orphan sweep done`) and a
failure at warn level. A row whose file is missing is not removed by the
sweep; its thumbnail answers 404 and the owner or an officer can delete it.

## Storage

- `member_settings.hero_icon_opt_in` (bool, default false). Rows written before
  the field existed read as false.
- Table `hero_icon`: `member_id`, `hero`, `file_name`, `size_bytes`, `width`,
  `height`, `created_at`, indexed on `member_id`. The record id is the icon id.
  `created_at` is `uploaded_at` in the API.
