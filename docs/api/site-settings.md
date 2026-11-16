# Site settings: display font

`brand_font_display` is the display font an admin picks for the site. It is a
field of the one `site_settings` row, next to `brand_accent_dark` and
`brand_accent_light`, and travels in the same `SiteSettings` object
(`crates/types/src/org/settings.rs`).

## Values

| Value             | Meaning                        |
|-------------------|--------------------------------|
| `""`              | Default. The site uses Urbanist. |
| `"urbanist"`      | Urbanist, picked explicitly.   |
| `"ibm-plex-sans"` | IBM Plex Sans.                 |

The list is `BRAND_FONT_DISPLAY_ALLOWED` in `scuffed-types`, checked with
`is_allowed_brand_font_display`. The value is an id from that list. It is
never a font name, a file path or a URL.

## Reading

The key is always present, also when it is `""`:

- `GET /api/settings` (public, no session needed).
- The body of a successful `PUT /api/settings`.
- The `#sc-settings` JSON in the SPA shell, so the client has it on first
  paint. That block is the `GET /api/settings` object plus `public_base_url`.

A settings row written before the field existed reads as `""`.

## Writing

`PUT /api/settings` with `"brand_font_display"` in the JSON body. Admin only.

- Key absent or `null`: the stored value is kept.
- `""`: back to the default.
- The value is compared exactly. There is no trimming and no case folding, so
  `" urbanist"` and `"Urbanist"` are refused.
- A refused body changes nothing, including its other fields.

| Caller and body                         | Result                                                                                   |
|-----------------------------------------|------------------------------------------------------------------------------------------|
| Signed out                              | 401 `{"error":"Authentication required"}`                                                |
| Unknown or expired session              | 401 `{"error":"Session expired or invalid"}`                                             |
| Member or recruit                       | 403 `{"error":"Officer access required"}`                                                |
| Officer, any value for this key         | 403 `{"error":"Admin access required"}`                                                  |
| Admin, a listed value                   | 200 with the full settings object                                                        |
| Admin, any other string                 | 400 `{"error":"Display font must be urbanist or ibm-plex-sans (or empty for default)"}` |
| Admin, a value that is not a string     | 422, plain text from the JSON extractor                                                  |

## Upgrade

The field is defined at startup with the other `site_settings` fields
(`crates/db/src/migrations.rs`):

```
DEFINE FIELD OVERWRITE brand_font_display ON site_settings TYPE string DEFAULT ALWAYS '';
```

There is no separate migration step and no backfill statement. An existing
row gets `""` the first time anything writes it. The first such write is the
`officers_can_edit_teams` backfill, which runs at every start. `DEFAULT ALWAYS`
is what lets that partial update through: a plain `DEFAULT` is applied on
create only, and `TYPE string` is checked on every write.

## Rollback

Revert the change and start the older build. Nothing else is required. The
field definition stays in the database, and because it is `DEFAULT ALWAYS ''`
the older build's settings save (a full row without this field) is accepted.
Each such save sets the stored font back to `""`.

Optional cleanup, to drop the definition as well:

```surql
REMOVE FIELD IF EXISTS brand_font_display ON TABLE site_settings;
```

After it, the next settings save by the older build removes the stored value
from the row. Rolling forward again needs no step in either case: the field is
defined again at startup and the row reads as `""`.

Both paths are tested in `crates/db/src/queries/settings.rs`
(`rollback_old_build_saves_settings_with_the_field_definition_in_place` and
`rollback_documented_remove_field_query_works`, which runs the query above as
read from this file).
