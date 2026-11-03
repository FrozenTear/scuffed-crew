# Stats privacy

Each member picks who can see their tracker stats. The setting is
`member_settings.stats_visibility` (`StatsVisibility` in
`crates/types/src/api/stats.rs`). It is read and written on
`GET` and `PUT /api/stats/settings`. A member with no settings row reads as
`public`.

| Value     | Meaning                                                              |
|-----------|----------------------------------------------------------------------|
| `public`  | Signed-out visitors and every crew member.                           |
| `members` | Active signed-in crew members. Off public (signed-out) leaderboards. |
| `private` | The member, and officers on the aggregate routes. Off every board.   |

A member always reads their own stats and games on the `/api/stats/me` routes
(`/api/stats/me`, `/me/matches`, `/me/matches/{id}`, `/me/heroes`, `/me/roles`,
`/me/maps`), whatever the setting.

## Aggregate member routes (unchanged)

`GET /api/stats/member/{id}`, `/heroes`, `/roles` and `/maps` need an active
signed-in member. They use `StatsVisibility::denial`:

- `public` and `members`: any active member.
- `private`: the member and officers. Anyone else gets 403
  `{"error":"stats_private"}`.
- 401 `Authentication required` when signed out, 403 `Membership inactive`
  for inactive callers.

These routes return totals and per-hero, per-role and per-map aggregates only.
They never return individual games.

## Officer match routes

Officers can read a member's individual games:

- `GET /api/stats/member/{id}/matches`: list. Takes `cursor`, `limit`
  (default 25, 1 to 100) and `season` like `/api/stats/me/matches`. Answers
  `{"data": [PersonalMatch, ...], "next_cursor": "..."}`. `next_cursor` is
  absent on the last page.
- `GET /api/stats/member/{id}/matches/{match_id}`: one game. Answers a single
  `PersonalMatch`. `match_id` is the `id` of a list row.

The rows are the same `PersonalMatch` the member sees on their own routes,
with the same read-time role fill. `suspect_fields` is returned on officer
reads exactly as on `/api/stats/me/matches`, so the site can show the
"might be misread" marker.

### Who may call them

| Caller                          | Result                                    |
|---------------------------------|-------------------------------------------|
| Signed out                      | 401 `{"error":"Authentication required"}` |
| Signed in, not a crew member    | 403 `{"error":"Not an org member"}`       |
| Inactive member or officer      | 403 `{"error":"Membership inactive"}`     |
| Suspended member or officer     | 403 `{"error":"Account suspended"}`       |
| Member below officer            | 403 `{"error":"Officer access required"}` |
| Officer, own id                 | 200, whatever their setting               |
| Officer, unknown member id      | 404 `{"error":"Member not found"}`        |
| Officer, member allows officers | 200                                       |
| Officer, member hides matches   | 403 `hidden_by_member` (below)            |

Members read their own games on `/api/stats/me/matches`, not on these routes.
A member who is not an officer gets 403 `Officer access required` here, even
for their own id.

The member check does not look at `is_active`, so officers can read a
deactivated member's games when that member's setting allows it. A
deactivated `private` member still answers 403 `hidden_by_member`.

On the single-match route, a key that does not exist, a malformed key and a
game that belongs to another member are all 404 `{"error":"Match not found"}`.

### Audit log

Each 200 read of another member's games writes one audit row (fire and
forget, a failed write is logged and does not fail the read):

| Route                    | `action`                | `target_type` | `target_id` | `details`    |
|--------------------------|-------------------------|---------------|-------------|--------------|
| `.../matches`            | `viewed_member_matches` | `member`      | member id   | none         |
| `.../matches/{match_id}` | `viewed_member_match`   | `member`      | member id   | the match id |

`actor_id` is the officer's member id. The list writes one row per page
read. An officer reading their own games, and any refused read (401, 403,
404), writes no row.

### Visibility to officer mapping

Decided by `StatsVisibility::officer_match_denial`:

| `stats_visibility` | Officer reads matches |
|--------------------|-----------------------|
| `public`           | Allowed               |
| `members`          | Allowed               |
| `private`          | Blocked, 403          |

**Decision (2026-11-03, FrozenTear).** For a `private` member, officers keep
seeing the totals on the aggregate member routes above, but not the
individual matches: the match routes answer 403 `hidden_by_member`. The
Settings copy for `private` will read:

> Only you can see your single matches. Officers can see your totals.

The app text changes in its own PR.

### Blocked body

When the member's setting hides their games from officers, both routes answer
403 with exactly this body (`StatsHiddenError` in
`crates/types/src/api/stats.rs`):

```json
{"error": "hidden_by_member", "message": "This member keeps their stats private."}
```

The check runs before any match query. The body has no other keys: no match
count, no last played date. An unknown `match_id` on a hidden member gets the
same 403, so a blocked read does not reveal whether a game exists.
