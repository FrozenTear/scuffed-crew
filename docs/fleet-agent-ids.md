# Fleet Agent Identity Scheme

**Status: PROPOSAL** — drafted 2026-07-25 (claude), reviewed same day (grok:
4 findings, all applied), pending Q1 + USER sign-off. On acceptance,
`docs/fleet-protocol.md` references this doc and the Usage matrix below
becomes binding.

## Why

`agent_id` today is a free-form string that collapses four different things:
which model is running, which configured agent it is, which live session holds
a lock, and whose authority it acts under. That worked at fleet size two, but
it gives us no answer for concurrent same-model sessions, and a restarted
session silently inherits its predecessor's leases — the exact failure mode
actor systems solved decades ago (a restarted Erlang process gets a *new* pid;
the registered name is re-bound explicitly, never inherited).

Design sources: Erlang registered names / Akka incarnations (two-tier split),
Kubernetes StatefulSet ordinals (concurrent instances of one spec), A2A Agent
Card + Entra blueprints (metadata lives in a card, not the ID string). Research
summary 2026-07-24; ask claude for the cited report.

## The scheme

```
agent_ref   = name [ "-" ordinal ] [ "#" incarnation ]

name        = registered kebab-case identity ("claude", "grok")
              MUST NOT end in "-" followed by digits (see below)
ordinal     = 0-based integer, only when ≥2 sessions of the same name
              run concurrently in one shift
incarnation = 4 lowercase hex chars, minted fresh at session start
              (first 4 hex of the session UUID)
```

Examples: `claude` · `claude-1` · `claude#a3f2` · `grok-0#09be`

Parsing rule: strip `#...` to get the session-stable ref; then strip a trailing
`-<digits>` group to get the registered name. Anything left must be a
registered name — peers should treat unregistered names as a protocol error,
not a new colleague.

**Names keep their hyphens; only a trailing hyphen-digits group is reserved.**
Kebab-case names are explicitly allowed (`code-reviewer` parses fine, ordinal
or not), but a name ending in `-<digits>` would be indistinguishable from a
name plus ordinal — `claude-2` cannot be both a registered name and
`claude` ordinal 2. Registering such a name is a protocol error, rejected at
the Registry PR. This is the narrowest rule that makes the grammar
unambiguous; banning hyphens outright would contradict kebab-case and cost us
multi-word names for nothing.

### Tier 1 — the stable name (who said it)

The registered name is the durable identity: it is what appears in reviews,
verdicts, audit trails, docs, and memory. It answers "who is accountable for
this judgment." Names are a flat namespace, registered in this doc (see
Registry), collision = pick a different name. A name outlives every session
that carries it.

### Tier 2 — the incarnation (who holds it)

The incarnation suffix scopes *ownership of live state*: leases, edit intents,
locks. It answers "is the holder of this lock still alive." Rules:

- Minted fresh at every session start. Never reused, never persisted.
- A restarted session is a **new incarnation**: it must re-acquire leases and
  re-publish intents. It never continues a dead incarnation's claims.
- A lease or intent whose incarnation is not the current holder's is **stale
  by definition** once its TTL lapses — any peer may treat it as released.
  (Intents already TTL at 120s; this makes the expiry semantics explicit.)
- Ordinals are assigned at shift start, lowest free first, checked against
  live intents. Single session of a name → no ordinal (`claude`, not
  `claude-0`).
- **Ordinals are sticky for the whole shift**, not per task (K8s StatefulSet
  semantics). A session keeps its ordinal across every task it picks up; it
  changes only at the next shift start. Per-task reassignment would make
  `claude-1` mean a different session in each PR, destroying the review-pairing
  guarantee ("claude-0 authors, claude-1 reviews") that the ordinal exists to
  provide. Restart within a shift: same ordinal if still free, new incarnation
  always.
- **Taking over a stale lease is TTL-lapse only — no announcement required.**
  Once the TTL has lapsed on a lease or intent whose incarnation is not a live
  holder's, any peer may take it. Requiring a `fleet::channel` note first would
  make takeover depend on a transport that is itself unreliable (it was
  unavailable for the 07-25 shift), reintroducing the deadlock the TTL removes.
  Courtesy note when the channel is up; never a precondition.

### Transport and escaping

Refs travel as **JSON string parameters** in MCP calls (`fleet_publish_intent`,
`fleet_acquire_lease`, and friends) — no shell is involved on the primary path,
so `#` needs no escaping there.

When a ref is passed on a command line instead, quote it: `'claude-1#a3f2'`.
In bash, zsh, and nu, `#` only opens a comment at the start of a word, so an
unquoted mid-word `#` already survives — the quoting rule is belt-and-braces
for wrappers that re-split arguments.

`#` was kept over the reviewed alternative `~` deliberately: `~` is git
revision syntax (`HEAD~1`), and refs land near git commands far more often
than they land in shell word-initial position. `#` has no meaning in git refs.

## Usage matrix

| Surface | What to use | Example |
|---|---|---|
| Fleet findings / verdicts / APPROVE on ydoc | name (+ ordinal if concurrent) | `claude-1: APPROVE NS2-5` |
| `fleet_publish_intent`, `fleet_acquire_lease`, edit locks | full ref with incarnation | `claude-1#a3f2` |
| Presence intent at session start | full ref + assignment sentence | `claude-1#a3f2 taking NS2-5` |
| Worktree path | name-ordinal + topic | `.claude/worktrees/claude-1-ns2-5` |
| Git commit trailer | model identity, unchanged | `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>` |
| PR author / review pairing | name (+ ordinal) | "claude-0 authors, claude-1 reviews" |
| Audit / docs / memory references | name only | "claude reviewed PR #20" |

Git trailers stay on **model** identity, not fleet names: provenance metadata
documents which model drafted the code; accountability rides on the human's
own committer identity (per current IETF direction — trailers are
documentation, not attestation). Fleet names never appear in git history.

## Registry

| Name | Vendor / model | Harness | Notes |
|---|---|---|---|
| `claude` | Anthropic (Claude Code sessions; model per trailer) | Claude Code CLI | may run ordinals 0..N |
| `grok` | xAI grok-4.5 | Grok Build | single-session to date |

Reserved, never valid as agent names: `user`, `human`, `system`, `admin`,
`agent`, `fleet`, and PID-style `agent-<digits>` (the daemon's fallback —
seeing one means a session forgot to pass `agent_id`).

Model, version, harness, and capabilities live **here in the registry row**
(the "card"), never encoded into the ref string. Model upgrades change the
card, not the name — history stays continuous.

Adding a name: PR against this table, one peer review. Names are permanent
once used in audit history (retire, don't recycle).

## Rules that do not change

- Cross-review remains symmetric and exception-free; author never merges own
  branch. Ordinals of the same name may review each other **only for
  low-stakes changes** — same-model review is measurably weaker (correlated
  blind spots; judges favor similar models), so anything high-stakes gets
  cross-model review (`grok`) or USER.
- Worktrees only; shared checkout read-only (IRON LAW).
- `agent_id` is honor-system today: the daemon accepts any string. Server-side
  registry validation is a future Memtrace item, out of scope here.

## Open questions for review

1. **Incarnation in the `agent_id` param vs. in intent metadata.** This draft
   puts it in the id string for lease-type calls (the daemon is a pass-through,
   so the string is the only enforced field), with `#` as the separator and the
   escaping rules under *Transport and escaping* above. Residual question for
   **grok**: does **Grok Build**'s tooling pass `#` through `agent_id`
   unmangled? One round-trip through `fleet_publish_intent` answers it. If it
   does not, the fallback is to move the incarnation into intent metadata and
   leave the id string at `name[-ordinal]` — not to change the separator.

Questions 2 (sticky ordinals) and 3 (lease takeover) were answered in review
and now live in *Tier 2* as rules, with the reasoning inline. Nothing else is
open.

**Promotion gate:** this doc stays a PROPOSAL until Q1 is answered on the
record. Merging it does not make it binding — `docs/fleet-protocol.md`
references it only once Q1 is closed and USER signs off.
