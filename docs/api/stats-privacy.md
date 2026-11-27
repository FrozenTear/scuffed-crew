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

| Route                    | `action`                | `target_type` | `target_id` | `details`                                    |
|--------------------------|-------------------------|---------------|-------------|----------------------------------------------|
| `.../matches`            | `viewed_member_matches` | `member`      | member id   | `offset=<n> limit=<n> season=<id or all>`    |
| `.../matches/{match_id}` | `viewed_member_match`   | `member`      | member id   | the match id                                 |

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

## Teammate hero data (planned, 0.8)

**Decision (2026-11-19, FrozenTear).** Rules for teammate hero and pairing
stats, before any of it is built:

- An active crew member whose `stats_visibility` is `public` or `members` may
  have the hero they played in a shared game counted in another member's
  stats, including hero pairing win rates. Their name is not shown there.
- A `private` member's games are never used in another member's stats.
- Teammates who are not crew members (random teammates) appear with their
  hero only. These stats never read, store or show their names.
- Pairing stats show aggregates only, never a list of individual shared
  games.

How the rules apply:

- **Who sees it.** Pairing stats are only for active signed-in crew members
  (the `OrgMember` check). They are never on a signed-out route, including
  `/api/public/...`, and are never cached in a slot that a signed-out caller
  can be served from. The subject member's own setting decides who sees their
  pairing totals, like their other totals (`StatsVisibility::denial`).
- **Minimum games.** The floor applies to every row the server returns,
  including all-time rows. A row is one own hero, one teammate hero, one map,
  and one season or all time. A row with fewer than 10 shared decided games
  is not returned at all: no counts, no record, no win rate. The floor is
  applied on the server after the season and map filters, not only in the
  app. Every total (across maps, seasons, heroes, or all time) is the sum of
  the rows that passed the floor only, so subtracting rows cannot reveal a
  row that was held back. A hidden name is not anonymity in a crew this
  size, since a hero only one member plays points at that member. The floor
  stops a row from describing one or a few games. It does not hide who the
  teammate is.
- **When changes apply.** Read time. The teammate's current
  `stats_visibility` applies to all past shared games. Nothing from another
  member's games is stored in a member's own rows. A cached pairing result is
  dropped when any member's visibility or active status changes, and when an
  upload deletes games. When any member deletes a game, that game drops out
  of every pairing count on the next read: the deleting member's row, and
  every other member's row for the same game, with all its slots.
- **Former members.** A member who is inactive (`is_active = false`; a ban
  sets it) cannot change their setting, so the setting they had when they
  left applies. If it is `private`, it stays in force for all their past
  games after they leave, go inactive or are banned: their rows are not read
  for pairing, and a slot matched to them is dropped. Their games never turn
  into random-teammate data. If it is `public` or `members`, they are treated
  as a random teammate: hero only, and their games stop counting as crew
  games.
- **Which games `private` covers.** Every game where the member can be
  identified: games they uploaded, and slots in another member's upload that
  are matched to their account. This holds whether or not they uploaded the
  game. A `private` member's rows are read only to find and drop their slots.
  A game where they cannot be identified (another member's upload where they
  are not matched to an account) cannot be filtered, and their hero there
  counts as a random teammate's hero.
- **What a response carries.** Rows are keyed only by the member's own hero,
  the teammate's hero and the map, with games, wins and losses. There is no
  draw column: draws are not paired, because "same outcome" cannot tell the
  two teams apart in a draw, and they stay out until a source can. "Decided
  games" in the floor means wins plus losses. No teammate member id, name,
  `session_id` or game time.
- **No settings row.** A member with no settings row counts as `public`, as
  today. The implementing PR updates the value table above and the Settings
  copy for `public` and `members` to mention teammate hero stats, before any
  pairing data shows.
- **Settings copy for `private`.** Before any pairing data shows, the
  `private` copy must say that: the setting stays in force for their past
  games after they leave, go inactive or are banned; it covers every game
  where they can be identified, including games they did not upload; a game
  where they cannot be identified (another member's upload where they are
  not matched to an account) cannot be filtered, so their hero can count
  there as a random teammate's hero; and a game that any member deletes
  drops out of every pairing count.
- **Random teammates phase.** When random teammates' heroes are added (from
  the issue 780 reader), any slot that matches a crew member's uploaded game
  is handled by the crew rules above, so a `private` member's slot is dropped.
  The same holds for a slot matched to a crew member's account in a game
  they did not upload. A slot that cannot be matched to any account cannot
  be told apart from a random teammate and counts as one: hero only, no name.

The routes and views that apply this land in their own PRs, which update
this section.
