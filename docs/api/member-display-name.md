# Member display names

Display names are unique. Two members cannot share a name once case, spacing,
invisible characters, and a small set of Latin look-alikes are ignored. The
stored `display_name` is what the site shows. Comparison uses `display_name_key`.

## Comparison key

`display_name_key` is computed in this order:

1. Reject the name if it contains a Unicode control character (general category
   Cc) or a line/paragraph separator (U+2028, U+2029). Controls are not removed.
2. Unicode NFKC.
3. Remove zero-width and other invisible characters: soft hyphen (U+00AD),
   ZWSP (U+200B), ZWNJ (U+200C), ZWJ (U+200D), word joiner (U+2060), BOM
   (U+FEFF), bidi marks and overrides, variation selectors, and the other
   format characters listed in `crates/db/src/display_name.rs`. Invisible
   characters are removed before whitespace is collapsed, so they cannot keep
   two spaces apart or split a name.
4. Trim and collapse the remaining Unicode whitespace to a single ASCII space.
5. Unicode full casefold (locale-independent). This is casefold, so `ß` and
   `ss` match, and final sigma matches sigma.
6. Map the homoglyph table below onto Latin letters.

If nothing is left, the name is invalid. The stored display name is not
rewritten. A member may rename to their own name with different casing,
spacing, or invisible characters, because that is the same key.

The key is not returned by the API.

### Homoglyph table

Applied after casefold, so only lowercase code points are listed. Uppercase
Cyrillic and Greek fold to these first. The table is small on purpose.

| Code point | Name          | Maps to |
|------------|---------------|---------|
| U+0430     | Cyrillic a    | a       |
| U+0435     | Cyrillic ie   | e       |
| U+043E     | Cyrillic o    | o       |
| U+0440     | Cyrillic er   | p       |
| U+0441     | Cyrillic es   | c       |
| U+0456     | Cyrillic i    | i       |
| U+03BF     | Greek omicron | o       |
| U+03B9     | Greek iota    | i       |
| U+03C1     | Greek rho     | p       |

## Uniqueness

`member.display_name_key` has a UNIQUE index (`member_display_name_key_idx`).
Two concurrent creates of the same key cannot both succeed: the loser hits the
index. Missing keys are allowed (the column is optional) so a row can exist
before backfill. Startup backfill fills every existing row and then creates
the index. If two stored names share a key, or a stored name cannot be keyed,
startup fails and does not rename anyone.

Deactivated members keep their key. The name stays taken.

## Error codes

Both bodies are JSON. The frontend should match on `error`.

| Status | Body | When | What to show |
|--------|------|------|----------------|
| 409 | `{"error":"display_name_taken"}` | The key is already used by another member. | That name is taken |
| 400 | `{"error":"display_name_invalid"}` | The name is empty after normalization, or it contains a control character. | The name is not allowed |

### Who may edit

`PUT /api/members/{id}` sets the display name and the other profile fields.

- A member may edit their own profile.
- An admin may edit any other member. Last-admin guards still apply when the
  body changes `is_active` or when an admin changes a role.
- An officer may edit a member or a recruit. Those are the ranks
  `officer_may_moderate` allows. An officer may not edit an admin or another
  officer.
- A member or recruit who edits someone else gets 403
  `{"error":"Can only edit your own profile"}`.
- An officer who edits an admin or another officer gets 403
  `{"error":"Officers cannot edit the profile of admins or other officers"}`.

`POST /api/upload/avatar` with `member_id` uses this same rule.

`PUT /api/members/{id}/game-accounts` and
`DELETE /api/members/{id}/game-accounts/{account_id}` use this same rank rule
when the caller changes someone else's account. A member or recruit who edits
someone else's game account gets 403
`{"error":"Can only edit your own game accounts"}`. Deleting someone else's
account gets 403 `{"error":"Can only delete your own game accounts"}`. An
officer who edits or deletes an admin or another officer's account gets the
same officer 403 as a profile edit. A refused delete removes nothing. Editing
or deleting your own game account is allowed. An admin may edit or delete any
other member's game account.

### Routes

- `PUT /api/members/{id}` sets or changes a display name. Who may call it is
  described above.
- `POST /api/auth/setup` creates the first admin. The display name is the
  username, and the key is stored on that member row.
- Application trial and accept (`PATCH /api/applications/{id}`) create the
  member row for a new account. The display name is the account username.
  Local registration (`POST /api/auth/local/register`) and the OAuth callback
  create the user row only. They do not write `member.display_name`. The 409
  or 400 is returned when that username is copied onto the new member.

Dev seed and the in-memory bootstrap member (`DevAdmin`) set `display_name_key`
to the key of that name.
