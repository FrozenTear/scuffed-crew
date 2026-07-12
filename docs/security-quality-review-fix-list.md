# Security / quality review fix list

**Source:** consolidated multi-agent review  
**Updated:** second wave — remaining A-batch + high #4/#6/#7

---

## Fixed

### Wave 1
| # | Issue | Status |
|---|--------|--------|
| C1 | Admin tournament URLs/status | **fixed** |
| C2 | Double-elim generator | **fixed** |
| H3 | Officer moderates admins | **fixed** |
| H5 | Officer-channel full roster encrypt | **fixed** |

### Wave 2
| # | Issue | Status |
|---|--------|--------|
| A1 / H4 | Forum `min_role` never enforced | **fixed** — read+write gated; tree hides restricted boards |
| A2 | Wiki `%q%` + CONTAINS | **fixed** — bind raw `q` |
| A3 | Tournament edit clears game_id | **fixed** — open_edit uses `game_id` |
| A4 | Hardcoded attendance date | **fixed** — local calendar today |
| A5 | Wiki stale after save | **fixed** — refresh signal + success toast |
| A6 / H7 | Lists cap 25 + Loading forever | **fixed** — auto-follow cursor pages (limit 100 × 10); `error` signal + members retry |
| A7 | Dead `/api/auth/login` link | **fixed** → `Route::Login` |
| A8 | Match date empty → 1970 | **fixed** — require played-at date |
| H6 | Multi-tab collab keyed by user id | **fixed** — connection UUID keys; UserLeft only when last tab |
| A9 | Orphan blog_article XSS | **deleted** dead file |
| A10 | Duplicate poll_card/poll_create | **deleted** dead duplicates |
| A11 | secret_hex not zeroized in chat auth | **fixed** |

---

### Wave 3
| # | Issue | Status |
|---|--------|--------|
| B3 | nostr_pubkey via PUT members | **fixed** — reject; challenge/verify only |
| B5 | DB errors to clients (key paths) | **fixed** — tournaments/forum/members log + generic message |
| B7 | send_encrypted ignore relay fail | **fixed** — 502 if any gift-wrap publish fails |
| B9 | Upload trust Content-Type / 2MB body | **fixed** — magic-byte sniff; DefaultBodyLimit 6MB |
| B6 partial | WS idle ghosts | **fixed** — 120s idle timeout on strategy WS |

## Still open

| ID | Issue | Needs |
|----|--------|--------|
| B2 | Member stats visibility | Product: intentional transparency assumed |
| B6 remainder | Await DB persist before collab ack | Larger design |
| B8 | Non-atomic bracket gen | Surreal transaction story |
| A12 | relay-policy gift-wrap allowlist | Before relay deploy |

---

## Decisions taken without asking

- **Pagination (B1):** Auto-fetch pages with `limit=100`, max 10 pages (~1000 rows).
- **Forum min_role (B10):** Enforce on **read and write**; hide restricted boards from tree when below role.
- **Multi-tab (B4):** Connection-id keys; multi-tab OK.
- **nostr_pubkey (B3):** Challenge/verify only — no admin override on PUT members.
- **Stats (B2):** Left as member-visible (review noted may be intentional).
