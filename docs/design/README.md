# Tracker design specs

Read this file before building from `spec.md` or `switch-spec.md`. It records where the two specs came from, what has shipped, a decision that overrides part of `switch-spec.md`, and the places where the specs no longer match the code.

## Provenance

| File here | Copy of | Written against |
|---|---|---|
| `spec.md` | UX Spec's status pill and pinned Settings bar spec, `/workspace/scuffed/tracker-ux/spec.md` on the bot computer | `wip/0.6` at `9ec2bb662cbd3894d4e399006d3958351474e820` |
| `switch-spec.md` | Design System's Settings switch spec, `/workspace/scuffed/tracker-design/switch-spec.md` on the bot computer | `wip/0.6` at `9ec2bb662cbd3894d4e399006d3958351474e820` |

Both were copied byte for byte in #895 (commit `2a7fb9dcda1beb0cfbe8055a9444169671196ef0`). Original sizes and MD5s (md5sum):

| File | Bytes | MD5 |
|---|---|---|
| `spec.md` (original) | 18369 | `ad61e73e3bd597ed6deb969fa075b02b` |
| `switch-spec.md` (original) | 9870 | `55406e7beb439821c991643a9132dc44` |

The copies now differ from the originals only by a personal name redaction on line 3 of each file: a person's name became `FrozenTear` in the prose of `switch-spec.md`, and a directory name in a path in `spec.md` became the placeholder `<home>`. Nothing else was changed. Sizes and MD5s after the redaction:

| File | Bytes | MD5 |
|---|---|---|
| `spec.md` (in repo) | 18369 | `256592ab83360de9b3554d0684556bea` |
| `switch-spec.md` (in repo) | 9874 | `35f3214fc4a7b8ae33bdfb5d9fbd05e8` |

No `cmp` against the external originals was possible when this README was written: the original files are not reachable from the repo or from the review. The original numbers above are the ones stated when the copies were made, and they match the files at `2a7fb9dc`.

## Status

- `spec.md` section 1 (status pill): shipped as #904 (for #775), merged into `wip/0.7` as `4aa6c41243aa068ab23111af960e1e9ee7e41631`.
- `spec.md` section 2 (pinned Settings bar): #776, target 0.7 (plan item T7). Not built yet.
- `switch-spec.md` (one switch control): #777, planned for 0.7.1, after the bar.

## Decided: Q7 = A, switches stay instant

FrozenTear decided on 27 Nov 2026 at 11:43 Oslo: switches stay instant in 0.7.0. `spec.md` holds as written (sections 1.5 and 2.1 to 2.5).

Not in force, do not build:

- `switch-spec.md` lines 112 and 113: "Flipping a switch marks the form dirty and waits for the pinned save bar" and "Discard on the save bar puts the switch back to the saved value."
- `switch-spec.md` line 51, the aside about the "dirty-and-save behaviour" going through `edit_switch`.

What this means:

- A switch flip writes at once, as it does today. It cannot be discarded; it is undone by flipping back.
- #776 counts and saves the three typed draft groups plus `changes_waiting`, as `spec.md` 2.2 and 2.3 say. No switch call site changes.
- #777 is look and keyboard only, with no tie to the Settings bar.

The spec text is left as copied. This section is the override.

## Known conflicts: do not build these literally

The specs were written against `wip/0.6`. Some passages conflict with code merged since, or with each other. The full list (R3 to R26) and the evidence are in the #895 review: https://github.com/FrozenTear/scuffed-crew/pull/895#issuecomment-7216135399. The ones that matter most for #776:

- **2.7 bottom padding against #894 / #806.** `spec.md` line 223 wants the page to gain bottom padding while the bar is visible. #894 (the fix for #806) made the bottom inset fixed and keeps the prompt layer always in the tree, because a layer that comes and goes resets the page scroll. A bar built as 2.7 reads brings #806 back. The bar also does not fit in today's inset alongside a toast and the restart prompt.
- **2.8 Tab order.** `spec.md` line 230 says Tab from the last Settings control moves to Discard, then the primary button. iced 0.14 buttons and togglers take no keyboard focus, and Tab with no field focused already goes to the status pill. The bar needs its own focus handling and an order against the pill; the spec gives none.
- **The count against the stopped-tracker rule.** `spec.md` line 136 counts the two kinds separately (copy at lines 147 to 151 shows N and M), but line 153 defines "the count" as their sum. A draft group counts as 1 but becomes one per key in `changes_waiting` once saved. With the tracker stopped and saved changes waiting, line 143 and line 203 keep the bar, line 181 hides it, and line 87 keeps the count.
- **Stale line references.** Several `file:line` references in `spec.md` (`nav.rs`, `app.rs`) and all ten `settings_cards.rs` references in `switch-spec.md` no longer land on `wip/0.7`. `switch-spec.md` line 9 says to build against its file:line list; look the symbols up by name instead. The review lists the current lines.

Track the resolution of these in #776 (and #777 for the switch).

## Referenced but not in the repo

- `/workspace/scuffed/tracker-ux/frames/`: the frames (`spec.md` line 3). There is no frame for the Settings bar in the repo.
- `/workspace/scuffed/tracker-ux/<home>/`: the phone review sheets (`spec.md` line 3, path redacted).
- `/workspace/scuffed/tracker-ux/audit.md`: the audit with file:line refs at `wip/0.6` `9ec2bb66`, cited by #775, #776 and #777.
- `switch-spec.png`: the switch state sheet (`spec.md` line 243).
- `/workspace/scuffed/tracker-design/switch-spec.md` (`spec.md` line 243): this is `switch-spec.md` in this folder.

The redesign shots that `switch-spec.md` line 9 cites are in the repo, at `crates/stat-tracker-ui/docs/redesign-shots/`.
