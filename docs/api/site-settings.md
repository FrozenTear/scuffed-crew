# Site settings: display font and brand accents

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

The brand settings page (`/admin/brand`) sends `brand_font_display` only
when the font differs from the stored one, compared as fonts (`""` and
`"urbanist"` are both Urbanist). When it does send Urbanist, by a pick or by
Reset to default, it sends `""`, the same as the accent's empty default. So
over a stored `"urbanist"`, picking Urbanist sends nothing and the stored
value stays. `"urbanist"` stays a valid value for API callers and reads the
same as `""`.

## Reading

The key is always present, also when it is `""`:

- `GET /api/settings` (public, no session needed).
- The body of a successful `PUT /api/settings`.
- The `#sc-settings` JSON in the SPA shell, so the client has it on first
  paint. That block holds the same JSON value as `GET /api/settings`. It
  also carries `public_base_url`, but only when `REDIRECT_BASE_URL` is a
  public http(s) origin. The bytes can differ: the shell writes `<`, `>`,
  `&`, U+2028 and U+2029 inside strings as `\u` escapes so the block cannot
  close its `<script>` element, and parsing gives back the same value.

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
| Signed in, no member row                | 403 `{"error":"Not an org member"}`                                                      |
| Member or recruit                       | 403 `{"error":"Officer access required"}`                                                |
| Officer, any string for this key        | 403 `{"error":"Admin access required"}`                                                  |
| Officer or admin, `null` for this key   | 200, stored font kept (`null` means keep, like an absent key)                            |
| Admin, a listed value                   | 200 with the full settings object                                                        |
| Admin, any other string                 | 400 `{"error":"Display font must be urbanist or ibm-plex-sans (or empty for default)"}` |
| Officer or admin, not a string or null  | 422, plain text from the JSON extractor                                                  |

## Brand accents

`brand_accent_dark` and `brand_accent_light` are the accent hexes for the
dark and light themes. `PUT /api/settings` takes them as `""` (default) or
`#rgb` / `#rrggbb`, stored lowercase as `#rrggbb`. Admin only, like
`brand_font_display`.

The defaults (`BrandConfig::product_default` in `scuffed-types`):

- `#8f73ff` is the dark default. Used when `brand_accent_dark` is `""`.
- `#6d4aff` is the light default. Used when both accents are `""`. When only
  `brand_accent_light` is `""`, the light theme follows the dark accent.

The two defaults are not interchangeable. `#6d4aff` fails as a dark accent:
its fill is 2.83:1 on the dark card `#282831`, under the 3:1 floor, so
`{"brand_accent_dark":"#6d4aff"}` is refused with the dark fill reason below.

### Accent contrast lock

`PUT /api/settings` applies the same contrast lock as the brand settings
page (`/admin/brand`). A refused body changes nothing, including its other
fields. The order of checks is: role (403), accent hex format (400
`{"error":"Brand accent must be a hex color like #8f73ff (or empty for default)"}`),
`brand_font_display` (400), then the contrast lock.

Refusal: 400 with `{"error": <reason>}`. The reason is the first failing
pair, checked in this order, dark theme first:

| Theme | Pair                                   | Reason                                                                                                  |
|-------|----------------------------------------|---------------------------------------------------------------------------------------------------------|
| dark  | accent text on the page, under 4.5:1   | `This accent does not reach 4.5:1 on the dark page background.`                                         |
| dark  | ink on the accent fill, under 4.5:1    | `Ink on this accent does not reach 4.5:1 in the dark theme.`                                            |
| dark  | fill on page and cards, under 3:1      | `Buttons in this accent do not reach 3:1 against the dark page and cards, so they lose their edge.`     |
| light | accent text on the page, under 4.5:1   | `This accent does not reach 4.5:1 on the light page background.`                                        |
| light | ink on the accent fill, under 4.5:1    | `Ink on this accent does not reach 4.5:1 in the light theme.`                                           |
| light | fill on page and cards, under 3:1      | `Buttons in this accent do not reach 3:1 against the light page and cards, so they lose their edge.`    |

The light reasons cannot be returned today. The light theme's derived
colours are darkened until every light pair passes, so every refusal names
the dark theme. The light check is kept as a guard in case that derivation
changes.

Which themes are checked (`changed_accent_themes` in
`crates/site-server/src/routes/settings.rs`):

| Accent keys in the body                     | Checked                                                   |
|---------------------------------------------|-----------------------------------------------------------|
| Neither (absent or `null`)                  | Nothing. The lock does not run.                           |
| `brand_accent_dark` only                    | Dark. Light too when the stored light accent is `""`, since light then follows dark. |
| `brand_accent_light` only                   | Light only.                                               |
| Both                                        | Dark and light. Stored accents are not read.              |

A theme is checked on the accents the save would leave stored: the request
value for a key that is sent, the stored value for one that is not.

Stored accents that fail the lock (saved before the lock existed):

- They do not block a save that leaves that theme's key out. A stored
  failing dark accent does not stop a body with only `brand_accent_light`,
  or with no accent keys at all.
- Sending the same failing value again is refused like any other value.
  There is no exemption for the stored value.
- `GET /api/settings`, the body of a successful `PUT`, and the
  `#sc-settings` block in the SPA shell still serve them as stored. The lock
  runs on writes only.

## Upgrade

`brand_font_display` is defined at startup with the other `site_settings` fields
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
the older build's settings save (a full row without this field) is accepted,
and so is its boot backfill. Each such save sets the stored font back to `""`.
Until the older build saves settings, the font stored before the rollback
stays on the row; the older build ignores it.

Rolling forward again needs no step: the field is defined again at startup.
The row then reads `""` if the older build saved settings in between, and
otherwise the font stored before the rollback.

Optional cleanup, to drop the definition and the value. Run it only once the
older build is up, never while this build still serves (this build's next
settings save would be refused, a 500, until it restarts). Both statements, in
one query, in this order:

```surql
REMOVE FIELD IF EXISTS brand_font_display ON TABLE site_settings; UPDATE site_settings UNSET brand_font_display;
```

The `UNSET` is what matters. `REMOVE FIELD` deletes the definition only, and
the row keeps its value (after one boot of this build every row has at least
`""`). `site_settings` is `SCHEMAFULL`, so a row with a field that has no
definition refuses every partial update ("Found field 'brand_font_display',
but no such field exists"), including the older build's boot backfill, and a
`surreal export` taken then does not restore the settings row on import. An
`UNSET` before the `REMOVE` does nothing: `DEFAULT ALWAYS` fills the value
straight back. The query can be run again safely. After it, rolling forward
reads `""`.

Both paths are tested in `crates/db/src/queries/settings.rs`
(`rollback_old_build_saves_settings_with_the_field_definition_in_place` and
`rollback_documented_remove_field_query_works`, which runs the query above as
read from this file and then the older build's boot backfill with no settings
save in between).
