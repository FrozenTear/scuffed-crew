# Personal stats

The `/api/stats/me` routes return the signed-in member's own tracker stats:
totals, games, and per-hero, per-role and per-map aggregates. Handlers are in
`crates/site-server/src/routes/stats.rs`, queries in
`crates/db/src/queries/personal_stats.rs`, and the response types in
`crates/db/src/types.rs` and `crates/types/src/api/stats.rs`.

All six routes are `GET` and read only. Games get there through
`POST /api/stats/upload` from the stat tracker (daemon token, not covered
here).

## Endpoints

| Route | Query | Answers |
|-------|-------|---------|
| `GET /api/stats/me` | `season` | `PersonalStats` |
| `GET /api/stats/me/matches` | `season`, `cursor`, `limit` | `{"data": [PersonalMatch, ...], "next_cursor": "..."}` |
| `GET /api/stats/me/matches/{id}` | none | `PersonalMatch` |
| `GET /api/stats/me/heroes` | `season` | `[HeroStats, ...]` |
| `GET /api/stats/me/roles` | `season` | `[RoleStats, ...]` |
| `GET /api/stats/me/maps` | `season` | `[MapStats, ...]` |

## Who can call them

Any active signed-in member (`OrgMember`). A session is read from an
`Authorization: Bearer <session token>` header first, else from the session
cookie. A tracker's daemon token is not a session and is refused.

The member always sees their own stats. `stats_visibility` is not checked on
these routes (see [Stats privacy](stats-privacy.md)).

## Season filter

`season` is a season id. Omitted or blank (after trimming) means all time.
A season matches games with `starts_at <= played_at < ends_at`. An id that
does not exist is 404 `{"error":"Season not found"}`. The same rule is used
by the leaderboards (`resolve_season_window`).

## Paging

`/api/stats/me/matches` is the only paged route. `limit` is 1 to 100,
default 25; a value out of range is clamped. `cursor` is the `next_cursor`
of the previous page. A cursor that does not decode starts from the first
page. `next_cursor` is left out on the last page.

## Errors

Every error body is `{"error":"..."}`.

| Status | `error` | When |
|--------|---------|------|
| 401 | `Authentication required` | No session header or cookie, or a header without `Bearer ` |
| 401 | `Session expired or invalid` | The session is unknown or expired |
| 403 | `Not an org member` | The account has no member record |
| 403 | `Membership inactive` | The member is not active |
| 403 | `Account suspended` | The member is suspended |
| 404 | `Season not found` | `season` names no season |
| 404 | `Match not found` | `/me/matches/{id}` only, see below |
| 500 | `Internal error` | A database or lookup failure |

## Outcomes

The upload stores only games whose `outcome` is `victory`, `defeat` or
`draw`, and skips the rest. `wins`, `losses` and
`draws` count those values. `total_matches` and `matches` count every stored
game in the window.

## GET /api/stats/me

Totals for the window.

```json
{
  "member_id": "abc123",
  "total_matches": 42,
  "wins": 22,
  "losses": 18,
  "draws": 2
}
```

## GET /api/stats/me/matches

The member's games, newest first (`played_at` descending).

`role` is filled when the list is read: a known stored role is kept, an empty
or unknown one takes the hero's role, and a hero that is not on the hero list
keeps `""`. The stored game is not changed.

`PersonalMatch` fields. Every field is always in the JSON.

| Field | Type | Meaning |
|-------|------|---------|
| `id` | string | The game's id, used by `/me/matches/{id}` |
| `member_id` | string | The owner |
| `session_id` | string | The tracker's game session id. `""` on old rows |
| `hero`, `map_name`, `game_mode`, `role`, `outcome` | string | As uploaded, with `role` filled as above |
| `elims`, `deaths`, `assists`, `damage`, `healing`, `mitigation` | number | The six stats |
| `played_at`, `uploaded_at` | string | RFC 3339 UTC |
| `edited` | bool | True when the member corrected a value in the tracker |
| `recognizer` | string | The reader that produced the stats. `ocr-v1` on rows from before the field |
| `name_recognizer` | string or null | The reader that found the member's own row. Null when not known |
| `suspect_fields` | array of strings | Stats the tracker marked as unsure. `[]` when none |
| `bans_picks` | object or null | Bans and picks read for the game (`BansPicks` in `crates/types/src/api/stats.rs`) |
| `rank` | object or null | The rank screen read for the game (`MatchRank` in `crates/types/src/api/stats.rs`) |

## GET /api/stats/me/matches/{id}

One of the member's own games, with the same fields and the same `role` fill
as a list row. An id that does not exist, an id that is not valid, and another
member's game all answer the same 404 `{"error":"Match not found"}`, so the
route does not tell whether someone else's game exists.

## GET /api/stats/me/heroes

One row per hero, ordered by `matches` descending, then `hero` ascending.
Heroes are grouped without regard to case, so `sombra` and `Sombra` are one
row.

```json
[
  {
    "hero": "Kiriko",
    "matches": 12,
    "wins": 7,
    "losses": 5,
    "draws": 0,
    "avg_elims": 11.5,
    "avg_deaths": 4.25,
    "avg_damage": 3200.0,
    "avg_healing": 9100.0
  }
]
```

The averages leave out elims above 99 (`PLAUSIBLE_ELIMS_MAX`), deaths above
50 (`PLAUSIBLE_DEATHS_MAX`), and any stat named in the game's
`suspect_fields`. The counts still include those games. A row with no game
left for a stat averages 0 for it.

## GET /api/stats/me/roles

The same fields as `/me/heroes` with `role` in place of `hero`. Games are
grouped by role, filled the same way as on `/me/matches`, so only games whose
hero is not on the hero list stay in the `""` row. Ordered by `matches`
descending, then `role` ascending. The averages follow the same rules as
`/me/heroes`.

## GET /api/stats/me/maps

One row per map: `map_name`, `matches`, `wins`, `losses`, `draws`. Ordered
by `matches` descending. Maps have no averages, and every game counts,
including one with an unsure stat.

```json
[
  {
    "map_name": "King's Row",
    "matches": 6,
    "wins": 4,
    "losses": 2,
    "draws": 0
  }
]
```

## Other members

To read another member's stats, use `GET /api/stats/member/{id}` and its
`/heroes`, `/roles` and `/maps` routes (any active member, subject to that
member's `stats_visibility`), or the officer match routes. Both are described
in [Stats privacy](stats-privacy.md).
