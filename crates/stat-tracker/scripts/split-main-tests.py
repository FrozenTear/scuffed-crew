#!/usr/bin/env python3
"""Move the daemon's unit tests out of `src/main.rs` into `src/main_tests/`.

`src/main.rs` ends with one big `#[cfg(test)] mod tests { ... }`. This script
cuts that block out, leaves `#[cfg(test)] mod main_tests;` in its place, and
writes the block's items to `src/main_tests/`:

* `mod.rs`  - `use super::*;`, every item of the block that is not a test
              (shared helpers, `use` lines, statics, test-only types), in
              their original order, then one `mod <area>;` per area file.
* `<area>.rs` - `use super::*;`, then the tests of that area, in their
              original order.

Items in a parent module are visible to its child modules, so the helpers in
`mod.rs` and every private item of `main.rs` stay reachable from each area
file through `use super::*;` (glob imports pass private imports down to
descendants). No item is renamed and no visibility changes.

The move is pure text: each item is copied verbatim, only dedented by the
four spaces of the old `mod tests {` wrapper (lines that start inside a
string literal are never touched). On top of that the script applies a short,
fixed list of REWRITES (see below), each of which must match exactly as often
as declared, or the script aborts. Re-running it on a newer `main.rs` (one
that still has the inline `mod tests`) reproduces the move, so fixes made on
the 0.5.x line are carried over by re-running, not by hand-merging.

Usage (from anywhere):

  split-main-tests.py                 # split the working-tree main.rs in place
  split-main-tests.py --check REV     # prove the working tree is the split of
                                      # REV:crates/stat-tracker/src/main.rs
  split-main-tests.py --from FILE --tests-only
                                      # regenerate src/main_tests/ from FILE (an
                                      # old-layout main.rs), main.rs untouched

Bringing a newer main (old layout) into a branch that has the split:

  git merge origin/main               # main.rs: conflicts only where main
                                      # touched the test block
  git show origin/main:crates/stat-tracker/src/main.rs > /tmp/main-new.rs
  # resolve main.rs: keep every non-test hunk as git merged it, and keep the
  # `#[cfg(test)] mod main_tests;` side for the test block, then
  crates/stat-tracker/scripts/split-main-tests.py --from /tmp/main-new.rs --tests-only
  # If the branch's main.rs (outside the tests) equals main's, this also holds:
  crates/stat-tracker/scripts/split-main-tests.py --check origin/main

Regenerating overwrites src/main_tests/: test edits made only on the branch
inside src/main_tests/ must be re-applied (or made on main first).

`--check` re-runs the split in memory from the old main.rs at REV and
verifies: (1) the new main.rs is byte-identical to the old one with the
block replaced by the one `mod` line; (2) the items in main_tests/,
concatenated in original order and with the wrapper/indent and the declared
REWRITES undone, equal the old block byte-for-byte; (3) every file in
main_tests/ equals what the split produces; (4) every test lands in exactly
one area and no item is lost or duplicated. `cargo fmt` is run on the result
in split mode; --check accepts only layout changes from it (whitespace,
trailing commas, braces around a one-expression closure body) and lists the
non-whitespace ones.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

CRATE = Path(__file__).resolve().parent.parent
REPO = CRATE.parent.parent
MAIN_RS = CRATE / "src" / "main.rs"
OUT_DIR = CRATE / "src" / "main_tests"
REL_MAIN = "crates/stat-tracker/src/main.rs"

OPEN_BLOCK = "#[cfg(test)]\nmod tests {\n"
NEW_DECL = "#[cfg(test)]\nmod main_tests;\n"
INDENT = "    "

# ---------------------------------------------------------------------------
# Areas. A test goes to the first area whose pattern matches its name
# (`re.search`); a test no pattern matches goes to `misc`. Order matters.
# A new test on main falls into an area by its name, so re-running the script
# never needs a hand edit. Keep patterns broad; `--check` reports area sizes.
# ---------------------------------------------------------------------------
AREAS: list[tuple[str, str]] = []  # filled in below
FALLBACK_AREA = "misc"


# ---------------------------------------------------------------------------
# Lexer: just enough Rust to know, for every character, whether it is code,
# a comment or inside a literal, and to track bracket depth.
# ---------------------------------------------------------------------------
@dataclass
class Tok:
    kind: str  # "open", "close", "semi", "comment", "str", "char", "other", "ident"
    text: str
    start: int
    end: int


IDENT_START = re.compile(r"[A-Za-z_]")
IDENT_CHAR = re.compile(r"[A-Za-z0-9_]")


def lex(src: str) -> tuple[list[Tok], list[str]]:
    """Tokens of `src` (whitespace dropped) and, per line, the lexer state at
    the line's first character: "code", "comment", "str" or "rawstr"."""
    toks: list[Tok] = []
    n = len(src)
    i = 0
    # state at each line start
    line_state: list[str] = ["code"]

    def mark_lines(a: int, b: int, state: str) -> None:
        # every newline in src[a:b] starts a line in `state`
        for _ in range(src.count("\n", a, b)):
            line_state.append(state)

    while i < n:
        c = src[i]
        if c == "\n":
            line_state.append("code")
            i += 1
            continue
        if c in " \t\r":
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            toks.append(Tok("comment", src[i:j], i, j))
            i = j
            continue
        if src.startswith("/*", i):
            depth = 0
            j = i
            while j < n:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                    if depth == 0:
                        break
                else:
                    j += 1
            mark_lines(i, j, "comment")
            toks.append(Tok("comment", src[i:j], i, j))
            i = j
            continue
        # raw strings: r"..", r#".."#, br"..", cr".."
        m = re.compile(r"(?:b|c)?r(#*)\"").match(src, i)
        if m and (i == 0 or not IDENT_CHAR.match(src[i - 1])):
            hashes = m.group(1)
            close = '"' + hashes
            j = src.find(close, m.end())
            if j < 0:
                raise SystemExit(f"unterminated raw string at {i}")
            j += len(close)
            mark_lines(i, j, "rawstr")
            toks.append(Tok("str", src[i:j], i, j))
            i = j
            continue
        # strings: "..", b"..", c".."
        m = re.compile(r"(?:b|c)?\"").match(src, i)
        if m and (i == 0 or not IDENT_CHAR.match(src[i - 1]) or src[i] == '"'):
            j = m.end()
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            mark_lines(i, j, "str")
            toks.append(Tok("str", src[i:j], i, j))
            i = j
            continue
        # char literal vs lifetime / label: b'x', 'x', '\n', '\u{..}'
        m = re.compile(r"b?'").match(src, i)
        if m and (i == 0 or not IDENT_CHAR.match(src[i - 1]) or src[i] == "'"):
            j = m.end()
            if j < n and src[j] == "\\":
                k = src.find("'", j + 2)
                toks.append(Tok("char", src[i : k + 1], i, k + 1))
                i = k + 1
                continue
            # one (possibly multi-byte) char then a closing quote
            if j + 1 < n and src[j + 1] == "'" and src[j] != "\n":
                toks.append(Tok("char", src[i : j + 2], i, j + 2))
                i = j + 2
                continue
            # lifetime or label
            k = j
            while k < n and IDENT_CHAR.match(src[k]):
                k += 1
            toks.append(Tok("other", src[i:k], i, k))
            i = k
            continue
        if IDENT_START.match(c):
            j = i + 1
            while j < n and IDENT_CHAR.match(src[j]):
                j += 1
            toks.append(Tok("ident", src[i:j], i, j))
            i = j
            continue
        if c in "{([":
            toks.append(Tok("open", c, i, i + 1))
        elif c in "})]":
            toks.append(Tok("close", c, i, i + 1))
        elif c == ";":
            toks.append(Tok("semi", c, i, i + 1))
        else:
            toks.append(Tok("other", c, i, i + 1))
        i += 1
    return toks, line_state


# ---------------------------------------------------------------------------
# Splitting the block into items
# ---------------------------------------------------------------------------
@dataclass
class Item:
    text: str  # verbatim, still indented by the old wrapper
    name: str
    is_test: bool
    first_line: int  # 1-based line in the old main.rs
    area: str = ""
    lines: list[int] = field(default_factory=list)


BRACE_ITEMS = {"fn", "impl", "mod", "struct", "enum", "trait", "union"}


def find_block(src: str) -> tuple[int, int]:
    """Byte range of the test block: from `#[cfg(test)]\\nmod tests {` to the
    end of its closing `}` line (inclusive of the newline)."""
    count = src.count("\n" + OPEN_BLOCK)
    if count != 1:
        raise SystemExit(f"expected one `{OPEN_BLOCK.strip()}` block in main.rs, found {count}")
    start = src.index("\n" + OPEN_BLOCK) + 1
    toks, _ = lex(src)
    body_open = start + len(OPEN_BLOCK) - 2  # the `{`
    depth = 0
    for t in toks:
        if t.start < body_open:
            continue
        if t.kind == "open":
            depth += 1
        elif t.kind == "close":
            depth -= 1
            if depth == 0:
                end = t.end
                if src[end : end + 1] != "\n":
                    raise SystemExit("the block's closing brace is not alone on its line")
                end += 1
                if src[end:].strip():
                    raise SystemExit("main.rs has code after the test block; refusing")
                return start, end
    raise SystemExit("unterminated test block")


def split_items(src: str, start: int, end: int) -> list[Item]:
    body_start = start + len(OPEN_BLOCK)
    body_end = src.rindex("}", start, end)  # closing brace of the block
    body = src[body_start:body_end]
    toks, _ = lex(body)
    items: list[Item] = []
    depth = 0
    item_start = 0  # offset in body where the current item (with trivia) starts
    kw = None  # first keyword that decides how the item ends
    seen_code = False
    pending_brace_end = False
    i = 0
    while i < len(toks):
        t = toks[i]
        if depth == 0 and t.kind == "comment":
            i += 1
            continue
        if depth == 0 and not seen_code:
            seen_code = True
        if t.kind == "open":
            if depth == 0 and t.text == "[" and i > 0 and toks[i - 1].text == "#":
                pass
            depth += 1
        elif t.kind == "close":
            depth -= 1
            if depth == 0 and t.text == "}" and kw in BRACE_ITEMS | {"macro"}:
                nxt = toks[i + 1] if i + 1 < len(toks) else None
                if not (nxt and nxt.kind == "semi"):
                    items.append(_finish(body, item_start, t.end, body_start, src))
                    item_start = _line_end(body, t.end)
                    kw = None
                    seen_code = False
        elif t.kind == "semi" and depth == 0:
            items.append(_finish(body, item_start, t.end, body_start, src))
            item_start = _line_end(body, t.end)
            kw = None
            seen_code = False
        elif t.kind == "ident" and depth == 0 and kw is None:
            # skip attributes: `#` `[` ... `]` are consumed by depth; idents at
            # depth 0 outside attributes decide the kind.
            if t.text in ("pub", "async", "unsafe", "const", "static", "extern", "use", "type", "let"):
                if t.text in ("use", "type", "static", "let"):
                    kw = t.text
                elif t.text == "const":
                    # `const fn` vs `const X`
                    nxt = toks[i + 1]
                    if nxt.kind == "ident" and nxt.text in ("fn", "unsafe", "async"):
                        pass
                    else:
                        kw = "const"
            elif t.text in BRACE_ITEMS:
                kw = t.text
            elif t.text in ("crate", "super", "in", "self"):
                pass  # inside pub(crate) etc. never reached at depth 0
            else:
                # `name!` macro invocation at item level (thread_local!, macro_rules!)
                nxt = toks[i + 1] if i + 1 < len(toks) else None
                if nxt and nxt.text == "!":
                    kw = "macro"
                else:
                    raise SystemExit(f"unexpected item start `{t.text}` at body offset {t.start}")
        i += 1
    if body[item_start:].strip():
        raise SystemExit(f"trailing text in the block: {body[item_start:][:200]!r}")
    return items


def _line_end(body: str, pos: int) -> int:
    j = body.find("\n", pos)
    rest = body[pos:j]
    if rest.strip():
        raise SystemExit(f"item ends mid-line: {body[pos - 60 : j]!r}")
    return j + 1


ATTR_TEST = re.compile(r"^\s*#\[(test|tokio::test\b[^\]]*)\]\s*$", re.M)
FN_NAME = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z0-9_]+)", re.M)
ITEM_NAME = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|struct|enum|trait|union|mod|const|static|type|impl(?:<[^>]*>)?)\s+([^\s(<{:;=]+)",
    re.M,
)


def _finish(body: str, a: int, b: int, body_start: int, src: str) -> Item:
    end = _line_end(body, b)
    text = body[a:end]
    first_line = src.count("\n", 0, body_start + a) + 1
    is_test = bool(ATTR_TEST.search(_attrs_part(text)))
    m = FN_NAME.search(text) if is_test else ITEM_NAME.search(text)
    name = m.group(1) if m else text.strip().splitlines()[-1][:40]
    return Item(text=text, name=name, is_test=is_test, first_line=first_line)


def _attrs_part(text: str) -> str:
    """The item's leading attribute lines (before its first code line)."""
    out = []
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith("//") or s.startswith("#["):
            out.append(line)
        else:
            break
    return "\n".join(out)




# ---------------------------------------------------------------------------
# Areas: contiguous runs of the block, each starting at an anchor test. The
# block was grown area by area (each run sits next to its helpers), so a run
# is an area. A test added on main lands in the area of the test before it.
# A missing anchor aborts the script (pick the next test of that run).
# ---------------------------------------------------------------------------
AREAS = [
    ("tab_keyboard_pid", "board_names_takes_every_name_window_and_skips_blanks"),
    ("poll_cadence", "slow_mode_only_for_mature_unfinished_game"),
    ("session_recovery", "tab_with_no_game_starts_fresh"),
    ("reader_switch_maps", "a_skipped_capture_report_uses_the_plan_outcome_and_the_callers_career_flag"),
    ("splits_and_debug_frames", "stat_regression_detects_real_game_boundaries"),
    ("new_reader_capture", "new_reader_drop_from_11_to_1_stays_11_and_flags_elims"),
    ("ocr_off_switch", "with_ocr_off_a_tab_frame_is_analysed_without_ocr_and_is_not_a_failed_capture"),
    ("hero_authority", "a_disagreeing_5v5_tab_keeps_the_earlier_confident_hero"),
    ("own_row_and_hero_seeds", "the_own_name_seeds_open_exactly_when_the_new_reader_is_in_effect"),
    ("local_rows_and_sync", "blank_map_and_unknown_hero_stay_local_until_picked"),
    ("run_loop_and_shutdown", "startup_takes_the_pid_guard_before_the_site_and_always_starts_the_pack_keeper"),
    ("night_sessions", "a_fallback_board_yields_to_two_matching_clean_reads_and_a_later_ghost_is_rate_capped"),
    ("map_hints", "an_armed_different_map_end_screen_seals_the_old_hint"),
    ("night_replays_quiet_close", "busan_defeat_and_junkertown_victory_stay_two_games"),
    ("upload_batches", "deathmatch_rows_stay_local"),
    ("shutdown_and_restart", "shutdown_keeps_an_unfinished_game_so_a_restart_resumes_it"),
    ("untrusted_maps", "a_stat_reset_on_untrusted_guillard_stays_unknown_and_does_not_upload_as_busan"),
    ("commit_and_command_tick", "commit_capture_rows_writes_the_current_row_then_the_carried_board"),
    ("cell_trust", "the_new_reader_reads_no_stat_cell_and_calibrates_nothing"),
    ("unread_columns", "new_reader_stores_no_row_while_a_stat_column_is_unread"),
]


def assign_areas(items: list[Item]) -> None:
    tests = [it for it in items if it.is_test]
    names = [t.name for t in tests]
    if len(set(names)) != len(names):
        raise SystemExit("duplicate test names in the block")
    starts = []
    for area, anchor in AREAS:
        if anchor not in names:
            raise SystemExit(f"anchor test `{anchor}` of area `{area}` is gone; update AREAS")
        starts.append(names.index(anchor))
    if starts != sorted(starts) or starts[0] != 0:
        raise SystemExit("AREAS anchors are out of order or the first is not the first test")
    k = 0
    for idx, t in enumerate(tests):
        while k + 1 < len(starts) and idx >= starts[k + 1]:
            k += 1
        t.area = AREAS[k][0]
    for it in items:
        if not it.is_test:
            it.area = "mod"


# ---------------------------------------------------------------------------
# Dedent: drop the four spaces of the old `mod tests {` wrapper from every
# line that starts in code or in a comment. A line that starts inside a string
# literal is part of that literal's value and is left alone, unless it follows
# a `\` line continuation (its leading whitespace is then not part of the
# value).
# ---------------------------------------------------------------------------
def dedent(text: str) -> str:
    _, states = lex(text)
    lines = text.split("\n")
    out = []
    for n, line in enumerate(lines):
        state = states[n] if n < len(states) else "code"
        if line == "" or state == "rawstr" or (state == "str" and not _continued(lines, n)):
            out.append(line)
        elif line.startswith(INDENT):
            out.append(line[len(INDENT) :])
        elif line.strip() == "":
            raise SystemExit(f"whitespace-only line in the block: {line!r}")
        else:
            raise SystemExit(f"line not indented by the wrapper: {line!r}")
    return "\n".join(out)


def _continued(lines: list[str], n: int) -> bool:
    """Line `n` starts inside a string right after a `\\`-newline escape.
    Rust drops all leading whitespace of such a line from the string's
    value, so its indent is layout only and moves with the code."""
    if n == 0:
        return False
    prev = lines[n - 1]
    trailing = len(prev) - len(prev.rstrip("\\"))
    return trailing % 2 == 1


def reindent(text: str) -> str:
    _, states = lex(text)
    lines = text.split("\n")
    out = []
    for n, line in enumerate(lines):
        state = states[n] if n < len(states) else "code"
        keep = line == "" or state == "rawstr" or (state == "str" and not _continued(lines, n))
        out.append(line if keep else INDENT + line)
    return "\n".join(out)


# ---------------------------------------------------------------------------
# REWRITES: the only non-whitespace edits. Each one is applied to the moved
# test text and must match exactly `count` times or the script aborts.
# ---------------------------------------------------------------------------
@dataclass
class Rewrite:
    why: str
    old: str
    new: str
    count: int


REWRITES = [
    Rewrite(
        "source-text tests read the daemon source, now one directory up",
        'include_str!("main.rs")',
        'include_str!("../main.rs")',
        21,
    ),
    Rewrite(
        "main.rs no longer holds this assertion's text, so the daemon's one call is the only match",
        "    // one call in the daemon; the text of this assertion is the second match\n"
        "    assert_eq!(\n"
        '        src.matches("recognize_scoreboard_names_pre_cropped(")\n'
        "            .count(),\n"
        "        2\n"
        "    );\n",
        "    // one call in the daemon (the tests no longer live in main.rs, so this\n"
        "    // assertion's own text is not a match)\n"
        "    assert_eq!(\n"
        '        src.matches("recognize_scoreboard_names_pre_cropped(")\n'
        "            .count(),\n"
        "        1\n"
        "    );\n",
        1,
    ),
]
# Plus one computed rewrite: a string literal "tests::<test name>" (a test that
# runs another test of this binary by its path) becomes the new path
# "main_tests::<area>::<test name>". The number found is reported.
TEST_PATH = re.compile(r'"tests::([A-Za-z0-9_]+)"')


def apply_rewrites(items: list[Item], report: list[str]) -> None:
    by_name = {it.name: it for it in items if it.is_test}
    for rw in REWRITES:
        hits = 0
        for it in items:
            c = it.text.count(rw.old)
            if c:
                for _ in range(c):
                    line = it.first_line + it.text[: it.text.index(rw.old)].count("\n")
                    report.append(f"  {it.area}.rs  {it.name}  (old main.rs:{line}): {rw.why}")
                    it.text = it.text.replace(rw.old, rw.new, 1)
                hits += c
        if hits != rw.count:
            raise SystemExit(f"rewrite {rw.old[:60]!r} matched {hits} times, expected {rw.count}")

    def sub(m: re.Match) -> str:
        name = m.group(1)
        if name not in by_name:
            raise SystemExit(f'"tests::{name}" names no test of the block')
        return f'"main_tests::{by_name[name].area}::{name}"'

    for it in items:
        new, k = TEST_PATH.subn(sub, it.text)
        if k:
            report.append(f"  {it.area}.rs  {it.name}: {k} test path literal(s) -> main_tests::<area>::")
            it.text = new


# ---------------------------------------------------------------------------
# Generation
# ---------------------------------------------------------------------------
MOD_HEADER = """\
//! Unit tests of the daemon binary (`main.rs`), split by area.
//!
//! Generated by `crates/stat-tracker/scripts/split-main-tests.py` from the
//! old inline `mod tests` of `main.rs`: shared helpers live here, tests in
//! the area files. Every test reaches `main.rs` and these helpers through
//! `use super::*;`.

"""
AREA_HEADER = "//! Daemon tests: {area}. See `mod.rs`.\n\nuse super::*;\n\n"


def generate(old_src: str) -> tuple[str, dict[str, str], list[Item], list[str]]:
    a, b = find_block(old_src)
    items = split_items(old_src, a, b)
    body = old_src[a + len(OPEN_BLOCK) : b - 2]
    if "".join(it.text for it in items) != body:
        raise SystemExit("the items do not add up to the block")
    assign_areas(items)
    report: list[str] = []
    for it in items:
        # the item's text without its separating blank lines, dedented
        stripped = it.text.lstrip("\n")
        d = dedent(stripped)
        if reindent(d) != stripped:
            raise SystemExit(f"dedent of {it.name} is not reversible")
        it.text = d
    apply_rewrites(items, report)

    new_main = old_src[:a] + NEW_DECL
    files: dict[str, str] = {}
    helpers = [it for it in items if not it.is_test]
    areas = [name for name, _ in AREAS]
    mod_rs = MOD_HEADER + "\n".join(it.text for it in helpers)
    mod_rs += "\n" + "".join(f"mod {name};\n" for name in sorted(areas))
    files["mod.rs"] = mod_rs
    for name in areas:
        tests = [it for it in items if it.area == name]
        files[f"{name}.rs"] = AREA_HEADER.format(area=name) + "\n".join(it.text for it in tests)
    return new_main, files, items, report


def rustfmt(files: dict[str, str]) -> dict[str, str]:
    out = {}
    for name, text in files.items():
        r = subprocess.run(
            ["rustfmt", "--edition", "2024", "--emit", "stdout", "--quiet"],
            input=text, capture_output=True, text=True,
        )
        if r.returncode != 0:
            raise SystemExit(f"rustfmt failed on {name}: {r.stderr}")
        out[name] = r.stdout
    return out


def summary(files: dict[str, str], items: list[Item], report: list[str]) -> str:
    lines = []
    tests = sum(it.is_test for it in items)
    lines.append(f"block items: {len(items)} ({tests} tests, {len(items) - tests} helpers/uses)")
    for name, text in files.items():
        n = sum(1 for it in items if it.is_test and f"{it.area}.rs" == name)
        lines.append(f"  {name:32} {text.count(chr(10)):6} lines {n:4} tests")
    lines.append("rewrites:")
    lines.extend(report)
    return "\n".join(lines)


def do_split(source: Path | None, tests_only: bool) -> None:
    old = (source or MAIN_RS).read_text()
    new_main, files, items, report = generate(old)
    files = rustfmt(files)
    OUT_DIR.mkdir(exist_ok=True)
    for f in OUT_DIR.glob("*.rs"):
        f.unlink()
    for name, text in files.items():
        (OUT_DIR / name).write_text(text)
    if not tests_only:
        MAIN_RS.write_text(new_main)
    print(summary(files, items, report))


def tokens_loose(text: str) -> list[str]:
    """Token texts, whitespace dropped and trailing commas before a closing
    bracket dropped: what rustfmt may change."""
    toks = [t.text for t in lex(text)[0]]
    out = []
    for i, t in enumerate(toks):
        if t == "," and i + 1 < len(toks) and toks[i + 1] in ")]}":
            continue
        out.append(t)
    return out


def fmt_token_changes(before: str, after: str) -> list[str] | None:
    """What rustfmt changed beyond whitespace and trailing commas. The only
    other change allowed is dropping the `{ }` around a closure body that is
    one expression (`|x| { e }` -> `|x| e`). None if anything else changed."""
    import difflib

    a, b = tokens_loose(before), tokens_loose(after)
    notes: list[str] = []
    opened = closed = 0
    for op, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if op == "equal":
            continue
        if op == "delete" and a[i1:i2] == ["{"] and a[i1 - 1] == "|":
            opened += 1
            notes.append(f"dropped closure-body braces: `... {' '.join(a[i1 - 6 : i1 + 4])} ...`")
        elif op == "delete" and a[i1:i2] == ["}"]:
            closed += 1
        else:
            return None
    if opened != closed:
        return None
    if before != after:
        notes.insert(0, "reflowed lines (whitespace only otherwise)")
    return notes


def do_check(rev: str) -> None:
    old = subprocess.run(
        ["git", "-C", str(REPO), "show", f"{rev}:{REL_MAIN}"],
        capture_output=True, text=True, check=True,
    ).stdout
    new_main, raw, items, report = generate(old)
    ok = True
    # (1) main.rs: the old one, the block replaced by the one mod line
    cur_main = MAIN_RS.read_text()
    a, b = find_block(old)
    if cur_main != new_main or old[:a] + NEW_DECL != cur_main or old[b:] != "":
        print("FAIL main.rs is not the old main.rs minus the block plus the mod line")
        ok = False
    else:
        print(f"ok   main.rs == old[:{a}] + {NEW_DECL!r} (old block: bytes {a}..{b}, "
              f"lines {old.count(chr(10), 0, a) + 1}..{old.count(chr(10), 0, b)})")
    # (2) rustfmt of the generated files changed whitespace/trailing commas only
    fmt = rustfmt(raw)
    for name in raw:
        extra = fmt_token_changes(raw[name], fmt[name])
        if extra is None:
            print(f"FAIL rustfmt changed tokens of {name} beyond layout")
            ok = False
        else:
            for e in extra:
                print(f"note rustfmt in {name}: {e}")
    # (3) the files on disk are exactly the formatted split
    on_disk = {f.name: f.read_text() for f in OUT_DIR.glob("*.rs")}
    if set(on_disk) != set(fmt):
        print(f"FAIL main_tests/ files differ: {sorted(set(on_disk) ^ set(fmt))}")
        ok = False
    for name in fmt:
        if on_disk.get(name) != fmt[name]:
            print(f"FAIL main_tests/{name} is not the split's output")
            ok = False
    # (4) independent re-read: split every file back into items and compare
    # them, in order, with the old block's items (dedented; rewrites undone)
    old_items = split_items(old, a, b)
    undo = {}
    for it in items:
        t = it.text
        for rw in REWRITES:
            t = t.replace(rw.new, rw.old)
        t = re.sub(r'"main_tests::[a-z0-9_]+::([A-Za-z0-9_]+)"', r'"tests::\1"', t)
        undo[it.first_line] = t
    for o in old_items:
        if reindent(undo[o.first_line]) != o.text.lstrip("\n"):
            print(f"FAIL item {o.name} (old line {o.first_line}) changed beyond the rewrites")
            ok = False
    reread: list[str] = []
    for name in ["mod.rs"] + [f"{n}.rs" for n, _ in AREAS]:
        text = raw[name]
        header = MOD_HEADER if name == "mod.rs" else AREA_HEADER.format(area=name[:-3])
        assert text.startswith(header)
        rest = text[len(header):]
        if name == "mod.rs":
            decls = "".join(f"mod {n};\n" for n in sorted(n for n, _ in AREAS))
            assert rest.endswith("\n" + decls)
            rest = rest[: -len(decls)]
        wrapped = OPEN_BLOCK + reindent(rest) + "}\n"
        reread += [it.text.lstrip("\n") for it in split_items(wrapped, 0, len(wrapped))]
    expect = [reindent(it.text) for it in items if not it.is_test]
    expect += [reindent(it.text) for n, _ in AREAS for it in items if it.is_test and it.area == n]
    if reread != expect:
        print("FAIL re-reading main_tests/ does not give back the block's items")
        ok = False
    else:
        tests = sum(it.is_test for it in items)
        print(f"ok   main_tests/ re-reads as the old block's {len(items)} items "
              f"({tests} tests), each verbatim but for the dedent and the rewrites below")
    print(summary(fmt, items, report))
    if not ok:
        sys.exit(1)
    print("CHECK PASSED")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", metavar="REV", help="verify the working tree against REV's main.rs")
    ap.add_argument("--from", dest="source", type=Path, metavar="FILE",
                    help="read the old main.rs (with the inline block) from FILE")
    ap.add_argument("--tests-only", action="store_true",
                    help="write only src/main_tests/, leave src/main.rs alone (merges)")
    ap.add_argument("--dump", action="store_true", help="list the block's items and exit")
    args = ap.parse_args()
    if args.dump:
        src = MAIN_RS.read_text()
        a, b = find_block(src)
        for it in split_items(src, a, b):
            print(f"{it.first_line}\t{'T' if it.is_test else 'H'}\t{it.name}\t{it.text.count(chr(10))}")
    elif args.check:
        do_check(args.check)
    else:
        do_split(args.source, args.tests_only)


if __name__ == "__main__":
    main()
