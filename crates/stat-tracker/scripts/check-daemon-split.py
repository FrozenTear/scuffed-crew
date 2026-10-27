#!/usr/bin/env python3
"""Check that the 0.6 daemon split (MAIN-15) only moved code out of `src/main.rs`.

Each split PR moves top-level items from `src/main.rs` into files under
`src/daemon/`. This script proves the move is pure, byte for byte, against a
base revision:

1. Every top-level item in a daemon file is one item of the base `main.rs`,
   once, after undoing the only two edits a move may make:
   - `pub(crate) ` at the start of a line (item, method or field visibility);
   - the log target pin, `target: "scuffed_stat_tracker", ` right after
     `tracing::<level>!(` (single line), or a `target: "scuffed_stat_tracker",`
     line right after a line ending in `tracing::<level>!(`.
   The base has neither, so undoing them cannot hide a change.
   rustfmt may also put a match arm whose pinned log call grew past the
   line width into a block (`=> {` newline, the call, newline, `}`); that is
   undone too, and each such item is listed. A block the base already has
   is kept: each choice of blocks to undo is tried.
   One more difference is allowed, because `pub(crate) ` can push a
   declaration past rustfmt's width: the declaration head (from the item's
   keyword line to the `{` that opens its body, or the whole item when it
   has no body) may differ in whitespace and in a trailing comma before `)`.
   Doc comments, attributes and bodies stay byte-identical. Each such item
   is listed as "reflowed" so a reviewer can look at it.
2. In each daemon file the moved items keep their base order, separated by
   one blank line, below a header of `//!` docs, `use crate::*;`, `mod x;` and
   `pub(crate) use x::*;` lines only. A `mod.rs` header may also declare a
   test file: a `#[cfg(test)]` line, then `mod tests_x;` on the next line.
3. Every `tracing::<level>!(` call in a moved item carries the pin, so moved
   log lines keep the `scuffed_stat_tracker` target they had in `main.rs`.
4. Items that a daemon file already has in the base (an earlier split
   step) are in the same file, in the same order, byte for byte.
5. The head `main.rs`, minus the scaffold block `mod daemon;\\nuse daemon::*;\\n\\n`,
   equals the base `main.rs` with the moved items cut out, and the scaffold
   sits right after `use tracing_subscriber::EnvFilter;` (when the base
   already has the scaffold, the head keeps it where it is). An item is cut with
   the blank line before it (or, failing that, the one after it).
   One exception: a test in `main.rs` named in `TEXT_CHECK_TESTS` reads the
   daemon's source as text (MAIN-14), and a step may repoint that check.
   Such a test must stay a top-level `#[cfg(test)] #[test]` fn, once, in
   the same place. Its text may differ only when the step names it with
   `--repointed NAME`, and the script then says so; a named test that did
   not change fails too, so the flag cannot be left on by habit.
6. Every daemon file is reached from `mod daemon;` through `mod x;` header
   lines, and each `pub(crate) use x::*;` re-exports a child declared there.
   A test file `tests_x.rs` is reached through its `#[cfg(test)]` and
   `mod tests_x;` pair in the `mod.rs` beside it; a `tests_` module declared
   without `#[cfg(test)]`, or re-exported, fails.
7. Moved items use no logging form the pin check does not cover (`event!`,
   spans, `#[instrument]`, a bare `info!(`): moving one would change its
   target.

Test files under `src/daemon/` (named `tests_*.rs`, moved there from
`src/main_tests/` by 0.6 set A) are not moved items: checks 1 to 4 skip
them, here and in the base, and only check 6 looks at them.
`check-test-renames.py` checks the tests they hold.

It proves the move is pure text; it does not prove the tree compiles (a
moved type's private field read from outside its new module, for one).
Build and test as well.

Usage (from anywhere in the repo):

    python3 -I crates/stat-tracker/scripts/check-daemon-split.py --base origin/wip/0.6
    python3 -I crates/stat-tracker/scripts/check-daemon-split.py --base REV --write-cut OUT
    python3 -I crates/stat-tracker/scripts/check-daemon-split.py --base REV --repointed NAME

`--write-cut OUT` writes the expected head `main.rs` (step 5) to OUT and exits;
the split author uses it to make the cut. Exit status 0 means the move is
pure; anything else prints what differs.

Python standard library only. The item splitter is a lexer, not rustc: it
blanks comments, strings and chars, then tracks brace depth at the top level.
"""

import argparse
import pathlib
import re
import subprocess
import sys

REL_MAIN = "crates/stat-tracker/src/main.rs"
REL_DAEMON = "crates/stat-tracker/src/daemon"
SCAFFOLD = "mod daemon;\nuse daemon::*;\n\n"
SCAFFOLD_AFTER = "use tracing_subscriber::EnvFilter;\n\n"
PIN = 'target: "scuffed_stat_tracker",'
LEVELS = r"(?:trace|debug|info|warn|error)"
HEADER_LINE = re.compile(
    r"^(//!.*|use crate::\*;|mod [a-z_][a-z0-9_]*;|pub\(crate\) use [a-z_][a-z0-9_]*::\*;|)$"
)
# A test file in a daemon mod.rs header: this line, then `mod tests_x;`.
CFG_TEST = "#[cfg(test)]"
TEST_MOD = re.compile(r"^mod (tests_[a-z0-9_]*);$")


def is_test_file(path):
    """A test file under src/daemon (`tests_*.rs`): not a moved item."""
    return pathlib.PurePosixPath(str(path)).name.startswith("tests_") and str(path).endswith(".rs")

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"

# Tests left in main.rs that read the daemon's source as text. A split step
# may repoint such a check (step 5).
TEXT_CHECK_TESTS = ("reused_frame_is_not_a_confirm_and_a_new_pair_is",)


def blank(text):
    """Replace comments, string and char literals by spaces (keep newlines)."""
    out = list(text)
    i, n = 0, len(text)

    def wipe(a, b):
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            wipe(i, j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            wipe(i, j)
            i = j
        elif c in "rbc" and re.match(r'(br|r|b|c|cr)?#*"', text[i:i + 6]) and (
            i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")
        ):
            m = re.match(r'(br|rb|cr|r|b|c)?(#*)"', text[i:])
            if not m:
                i += 1
                continue
            prefix, hashes = m.group(1) or "", m.group(2)
            j = i + m.end()
            if "r" in prefix:
                end = '"' + hashes
                k = text.find(end, j)
                k = n if k < 0 else k + len(end)
            else:
                k = j
                while k < n and text[k] != '"':
                    k += 2 if text[k] == "\\" else 1
                k += 1
            wipe(i, k)
            i = k
        elif c == '"':
            k = i + 1
            while k < n and text[k] != '"':
                k += 2 if text[k] == "\\" else 1
            k += 1
            wipe(i, k)
            i = k
        elif c == "'":
            m = re.match(r"'(\\u\{[0-9a-fA-F]+\}|\\x[0-9a-fA-F]{2}|\\.|[^\\'])'", text[i:])
            if m:
                wipe(i, i + m.end())
                i += m.end()
            else:
                i += 1  # lifetime / label
        else:
            i += 1
    return "".join(out)


KW = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+(?=fn))?"
    r"(fn|struct|enum|impl|const|static|mod|type|trait|use|union|macro_rules!)\b"
)
NAME = re.compile(
    r"^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+(?=fn))?"
    r"(?:fn|struct|enum|const|static|mod|type|trait|union)\s+(" + IDENT + r")"
)


def items(text):
    """Top-level items as dicts: kind, name, start/end line (1-based,
    inclusive; start includes the doc comments and attributes right above),
    code_line (the keyword line) and text."""
    b = blank(text)
    lines = text.split("\n")
    blines = b.split("\n")
    out = []
    depth = 0
    pending_start = None
    cur = None
    attr_depth = 0
    for ln, (raw, bl) in enumerate(zip(lines, blines), start=1):
        s = bl.strip()
        if depth == 0 and cur is None:
            rs = raw.strip()
            if not rs:
                pending_start = None
                continue
            if attr_depth > 0 or rs.startswith("//") or rs.startswith("#[") or rs.startswith("#!["):
                if pending_start is None:
                    pending_start = ln
                attr_depth += s.count("[") - s.count("]")
                continue
            m = KW.match(s)
            mm = re.match(r"^(" + IDENT + r")!", s)
            if m:
                kind = m.group(1)
                if kind == "impl":
                    nm = re.match(r"^impl(?:<[^{]*?>)?\s+(.*?)\s*(?:where\b.*)?\{?$", s)
                    name = "impl " + (nm.group(1) if nm else s).replace("{", "").strip()
                elif kind == "use":
                    name = "use " + s[s.index("use") + 4:].rstrip(";{")
                else:
                    nm = NAME.match(s)
                    name = nm.group(1) if nm else s
            elif mm:
                kind, name = "macro", mm.group(1) + "!"
            else:
                raise SystemExit(f"line {ln}: unexpected top-level text {s[:80]!r}")
            cur = {"kind": kind, "name": name, "start": pending_start or ln, "code_line": ln}
            pending_start = None
        for ch in bl:
            if ch in "{([":
                depth += 1
            elif ch in "})]":
                depth -= 1
        if cur is not None and depth == 0 and (s.endswith(";") or s.endswith("}")):
            cur["end"] = ln
            cur["text"] = "\n".join(lines[cur["start"] - 1:ln])
            out.append(cur)
            cur = None
    if cur is not None:
        raise SystemExit(f"unterminated item {cur['name']} at line {cur['start']}")
    return out


PIN_CODE = re.escape(blank(PIN))  # the pin as blank() shows it (its string wiped)


ARM_BLOCK = re.compile(
    r"=> \{\n[ \t]+(tracing::" + LEVELS + r"!\(" + re.escape(PIN) + r" [^\n]*\))\n[ \t]*\}"
)


def unwrap_arms(text, which=None):
    """Undo rustfmt wrapping a match arm in a block because its pinned log
    call grew past the line width: `=> {\n    tracing::x!(target: .., ..)\n}`
    back to `=> tracing::x!(target: .., ..),`. The base may already have
    such a block (its call was long before the pin), so `which` picks the
    blocks to undo by index; None undoes all. Returns (text, how many)."""
    code = blank(text)
    out, last, n, k = [], 0, 0, -1
    for m in ARM_BLOCK.finditer(text):
        if code[m.start():m.start() + 4] != "=> {" or code[m.end() - 1] != "}":
            continue
        k += 1
        if which is not None and k not in which:
            continue
        out.append(text[last:m.start()] + "=> " + m.group(1) + ",")
        last = m.end()
        n += 1
    out.append(text[last:])
    return "".join(out), n


def unpin_variants(text):
    """(unpinned text, blocks undone) for every choice of arm blocks to undo
    (see `unwrap_arms`), fewest first. A moved item matches the base when
    one of them does."""
    _, n = unwrap_arms(text)
    if n > 8:
        sys.exit(f"{n} arm blocks in one item; extend unpin_variants")
    subsets = sorted(range(1 << n), key=lambda s: bin(s).count("1"))
    return [
        (unpin(text, {k for k in range(n) if s >> k & 1}), bin(s).count("1")) for s in subsets
    ]


def unpin(text, which=None):
    """Undo the edits a move may make (see the module docs). Matches are
    found in code only (not in comments or string literals), then cut from
    the text at the same offsets. `which`: the arm blocks to undo, as in
    `unwrap_arms`."""
    text, _ = unwrap_arms(text, which)
    code = blank(text)
    spans = [m.span(1) for m in re.finditer(r"(?m)^\s*(pub\(crate\) )", code)]
    for m in re.finditer(r"tracing::" + LEVELS + r"!\((" + PIN_CODE + " )", code):
        if text[m.start(1):m.end(1)] == PIN + " ":
            spans.append(m.span(1))
    for m in re.finditer(r"tracing::" + LEVELS + r"!\(\n([ \t]*" + PIN_CODE + r"\n)", code):
        if text[m.start(1):m.end(1)].strip() == PIN:
            spans.append(m.span(1))
    for a, b in sorted(spans, reverse=True):
        text = text[:a] + text[b:]
    return text


def split_decl(text):
    """(docs and attributes, declaration head, body) of one item's text.
    The head runs from the keyword line to the first `{` outside parens,
    brackets and strings, inclusive; the body is the rest."""
    lines = text.split("\n")
    k = 0
    while k < len(lines) and (
        lines[k].lstrip().startswith(("//", "#[", "#!["))
    ):
        k += 1
    pre = "\n".join(lines[:k])
    rest = "\n".join(lines[k:])
    code = blank(rest)
    depth = 0
    for i, ch in enumerate(code):
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif ch == "{" and depth == 0:
            return pre, rest[: i + 1], rest[i + 1:]
    return pre, rest, ""


def squash(head):
    """A declaration head without whitespace and without the trailing comma
    rustfmt adds when it breaks a list over lines (`,` then a line break then
    `)`). String and char literals and comments are kept byte for byte, so
    whitespace inside them still counts."""
    # Mark spaces and tabs first, so blank() shows which of them sit inside a
    # literal or comment (wiped to " ") and which are code (still marked).
    marked = head.replace(" ", "\x01").replace("\t", "\x02")
    code = blank(marked)
    inside = [m != c for m, c in zip(marked, code)]
    code = code.replace("\x01", " ").replace("\x02", "\t")
    out, i = [], 0
    while i < len(head):
        if inside[i]:  # inside a literal or comment: keep as is
            j = i
            while j < len(head) and inside[j]:
                j += 1
            out.append(head[i:j])
            i = j
            continue
        if head[i] == "," and re.match(r",[ \t]*\n\s*\)", code[i:]):
            i += 1
            continue
        if not head[i].isspace():
            out.append(head[i])
        i += 1
    return "".join(out)


def same_but_reflowed(base, head):
    """True when `head` differs from `base` only in its declaration head's
    whitespace and trailing commas (see the module docs)."""
    bp, bh, bb = split_decl(base)
    hp, hh, hb = split_decl(head)
    return bp == hp and bb == hb and bh != hh and squash(bh) == squash(hh)


OTHER_LOGS = re.compile(
    r"(?<![A-Za-z0-9_:])(?:tracing::)?(?:event|span|enabled|"
    + LEVELS[3:-1]
    + r"|(?:trace|debug|info|warn|error)_span)!\s*[\(\[\{]|#\[\s*(?:tracing::)?instrument"
)


def other_log_forms(text):
    """Logging calls the pin check does not cover: `event!`, spans,
    `enabled!`, `#[instrument]`, a bare `info!(`, or `tracing::warn!{`/`[`.
    Moving one would change its target, so the split must not move one
    without extending this script."""
    code = blank(text)
    bad = []
    for m in OTHER_LOGS.finditer(code):
        if re.match(r"tracing::" + LEVELS + r"!\(", code[m.start():]):
            continue
        bad.append(text.count("\n", 0, m.start()) + 1)
    return bad


def unpinned_calls(text):
    """`tracing::<level>!(` calls in code (not comments or strings) without the pin."""
    code = blank(text)
    bad = []
    for m in re.finditer(r"tracing::" + LEVELS + r"!\(", code):
        rest = text[m.end():]
        if rest.startswith(PIN + " "):
            continue
        if re.match(r"\n[ \t]*" + re.escape(PIN) + r"\n", rest):
            continue
        bad.append(text.count("\n", 0, m.start()) + 1)
    return bad


def cut(base_text, moved):
    """`base_text` with the items in `moved` (base item dicts) removed, each
    with the blank line before it, or else the blank line after it. Items
    that touch (no blank line between them) are one run: the run gives up a
    single blank line, so the text around it keeps its spacing."""
    lines = base_text.split("\n")
    drop = set()
    for it in moved:
        s, e = it["start"], it["end"]
        drop.update(range(s, e + 1))
        if s > 1 and lines[s - 2] == "" and (s - 1) not in drop:
            drop.add(s - 1)
        elif s > 1 and (s - 1) in drop:
            pass  # the item above, in the same run, already gave up a blank line
        elif e < len(lines) and lines[e] == "" and (e + 1) not in drop:
            drop.add(e + 1)
    return "\n".join(line for i, line in enumerate(lines, start=1) if i not in drop)


def git(*args, cwd):
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


def split_header(text):
    """(header lines, body text) of a daemon file: the header is the leading
    run of `//!`, `use crate::*;`, `mod x;`, `pub(crate) use x::*;` and blank
    lines, and `#[cfg(test)]` lines each followed by `mod tests_x;`; the body
    is everything after it."""
    lines = text.split("\n")
    n = 0
    while n < len(lines):
        if lines[n] == CFG_TEST and n + 1 < len(lines) and TEST_MOD.match(lines[n + 1]):
            n += 2
        elif HEADER_LINE.match(lines[n]):
            n += 1
        else:
            break
    return lines[:n], "\n".join(lines[n:])


def header_mods(header):
    """(child modules, test modules, `tests_` modules without `#[cfg(test)]`)
    declared in header lines: a test module is a `mod tests_x;` line right
    after a `#[cfg(test)]` line."""
    mods, tests, bare = [], [], []
    for k, line in enumerate(header):
        m = re.match(r"mod ([a-z_0-9]+);$", line)
        if not m:
            continue
        if TEST_MOD.match(line) and k > 0 and header[k - 1] == CFG_TEST:
            tests.append(m.group(1))
        elif m.group(1).startswith("tests_"):
            bare.append(m.group(1))
        else:
            mods.append(m.group(1))
    return mods, tests, bare


def mask_text_checks(text, errors, where):
    """`text` with each `TEXT_CHECK_TESTS` fn replaced by a stub, and the
    names of those fns, with their texts."""
    found = {}
    for it in items(text):
        if it["name"] not in TEXT_CHECK_TESTS:
            continue
        if it["kind"] != "fn" or not it["text"].startswith("#[cfg(test)]\n#[test]\nfn "):
            errors.append(f"{where}: {it['name']} must stay a top-level #[cfg(test)] #[test] fn")
        if it["name"] in found:
            errors.append(f"{where}: {it['name']} is there more than once")
        found[it["name"]] = it["text"]
        text = text.replace(it["text"], f"#[cfg(test)]\n#[test]\nfn {it['name']}() {{}}", 1)
    for name in TEXT_CHECK_TESTS:
        if name not in found:
            errors.append(f"{where}: {name} is missing")
    return text, found


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--base", required=True, help="base revision, e.g. origin/wip/0.6")
    ap.add_argument("--write-cut", metavar="OUT", help="write the expected head main.rs and exit")
    ap.add_argument(
        "--repointed",
        metavar="NAME",
        action="append",
        default=[],
        choices=TEXT_CHECK_TESTS,
        help="a TEXT_CHECK_TESTS test in main.rs whose source-text check this step repoints",
    )
    args = ap.parse_args()

    here = pathlib.Path(__file__).resolve().parent
    root = pathlib.Path(git("rev-parse", "--show-toplevel", cwd=here).strip())
    base_text = git("show", f"{args.base}:{REL_MAIN}", cwd=root)
    if "pub(crate) " in base_text or PIN in base_text:
        sys.exit("base main.rs already has pub(crate) or a target pin; extend unpin() first")
    base_items = items(base_text)
    by_text = {}
    for idx, it in enumerate(base_items):
        by_text.setdefault(it["text"], []).append(idx)
    # Daemon files the base already has (from earlier split steps): their
    # items must come through byte for byte, in the same file and order.
    base_daemon = {}
    for rel in git("ls-tree", "-r", "--name-only", args.base, "--", REL_DAEMON, cwd=root).split():
        if rel.endswith(".rs") and not is_test_file(rel):
            _, body = split_header(git("show", f"{args.base}:{rel}", cwd=root).rstrip("\n"))
            base_daemon[rel] = [it["text"] for it in items(body)] if body.strip() else []
    has_scaffold = SCAFFOLD.rstrip("\n") + "\n" in base_text

    errors = []
    moved_idx = {}
    summary = []
    reflowed = []
    rewrapped = []
    daemon_dir = root / REL_DAEMON
    all_files = sorted(daemon_dir.rglob("*.rs")) if daemon_dir.is_dir() else []
    # Test files are reached (check 6) but hold no moved items (checks 1-4).
    files = [path for path in all_files if not is_test_file(path)]
    # Every file must be reached from `mod daemon;` through header `mod x;`
    # lines (x.rs beside the declaring mod.rs, or x/mod.rs), and every
    # `pub(crate) use x::*;` must re-export a child declared in that header.
    # A test file is reached through `#[cfg(test)]` + `mod tests_x;` in the
    # mod.rs beside it, and is not walked further.
    reached, todo = set(), [daemon_dir / "mod.rs"]
    while todo:
        path = todo.pop()
        if path in reached or not path.is_file():
            continue
        reached.add(path)
        if is_test_file(path):
            continue
        where = path.relative_to(root).as_posix()
        header, _ = split_header(path.read_text().rstrip("\n"))
        mods, tests, bare = header_mods(header)
        uses = [m.group(1) for line in header for m in [re.match(r"pub\(crate\) use ([a-z_0-9]+)::\*;$", line)] if m]
        for u in uses:
            if u in tests:
                errors.append(f"{where}: re-exports the test module {u}")
            elif u not in mods:
                errors.append(f"{where}: re-exports {u}, which it does not declare")
        for t in bare:
            errors.append(f"{where}: mod {t} needs a {CFG_TEST} line right above it")
        if path.name != "mod.rs" and (mods or tests or bare):
            errors.append(f"{where}: only a mod.rs may declare child modules")
        for m in mods:
            flat, nested = path.parent / f"{m}.rs", path.parent / m / "mod.rs"
            if flat.is_file() == nested.is_file():
                errors.append(f"{where}: mod {m} needs exactly one of {m}.rs or {m}/mod.rs")
            todo += [flat, nested]
        for t in tests + bare:
            flat = path.parent / f"{t}.rs"
            if not flat.is_file() or (path.parent / t).exists():
                errors.append(f"{where}: test mod {t} needs {t}.rs beside it, and no {t}/ directory")
            todo.append(flat)
    for path in all_files:
        if path not in reached:
            errors.append(f"{path.relative_to(root).as_posix()}: not reached from `mod daemon;`")
    tests_seen = [path.relative_to(root).as_posix() for path in all_files if is_test_file(path)]
    head_rels = {path.relative_to(root).as_posix() for path in files}
    for rel in sorted(set(base_daemon) - head_rels):
        errors.append(f"{rel}: in the base, missing at head")
    kept_total = 0
    for path in files:
        rel = path.relative_to(root).as_posix()
        text = path.read_text()
        if not text.endswith("\n") or text.endswith("\n\n"):
            errors.append(f"{rel}: must end with exactly one newline")
        header, body = split_header(text.rstrip("\n"))
        if not body.strip():
            if base_daemon.get(rel):
                errors.append(f"{rel}: lost the {len(base_daemon[rel])} items it has in the base")
            summary.append(f"{rel}: header only ({len(header)} lines)")
            continue
        file_items = items(body)
        rebuilt = "\n\n".join(it["text"] for it in file_items)
        if rebuilt != body:
            errors.append(f"{rel}: text outside items, or items not separated by one blank line")
        last = -1
        lines_moved = 0
        kept_from = base_daemon.get(rel, [])
        kept_at = 0
        kept = 0
        for it in file_items:
            if kept_at < len(kept_from) and it["text"] == kept_from[kept_at]:
                kept_at += 1
                kept += 1
                continue
            for line in unpinned_calls(it["text"]):
                errors.append(f"{rel}: {it['name']}: tracing call without the target pin (item line {line})")
            for line in other_log_forms(it["text"]):
                errors.append(f"{rel}: {it['name']}: logging form the pin check does not cover (item line {line})")
            variants = unpin_variants(it["text"])
            hits, arms = [], 0
            for want, arms in variants:
                hits = [i for i in by_text.get(want, []) if i not in moved_idx]
                if hits:
                    break
            if not hits:
                for want, arms in variants:
                    hits = [
                        i
                        for i, b in enumerate(base_items)
                        if i not in moved_idx
                        and b["name"] == it["name"]
                        and same_but_reflowed(b["text"], want)
                    ]
                    if hits:
                        reflowed.append(f"{rel}: {it['name']}")
                        break
            if hits and arms:
                rewrapped.append(f"{rel}: {it['name']} ({arms})")
            if not hits:
                label = it["name"] if it["kind"] == "impl" else f"{it['kind']} {it['name']}"
                errors.append(f"{rel}: {label}: not a byte-identical base item")
                continue
            i = hits[0]
            moved_idx[i] = rel
            if i < last:
                errors.append(f"{rel}: {it['name']}: out of base order")
            last = i
            lines_moved += base_items[i]["end"] - base_items[i]["start"] + 1
        if kept_at < len(kept_from):
            errors.append(
                f"{rel}: base item {kept_at + 1} of {len(kept_from)} is missing, changed or out of order"
            )
        kept_total += kept
        moved_here = len(file_items) - kept
        note = f"{kept} kept, " if rel in base_daemon else ""
        summary.append(f"{rel}: {note}{moved_here} moved items, {lines_moved} base lines")

    moved = [base_items[i] for i in sorted(moved_idx)]
    expected = cut(base_text, moved)
    if args.write_cut:
        if has_scaffold:
            out = expected
        elif SCAFFOLD_AFTER not in expected:
            sys.exit(f"no {SCAFFOLD_AFTER!r} in main.rs to put the scaffold after")
        else:
            out = expected.replace(SCAFFOLD_AFTER, SCAFFOLD_AFTER + SCAFFOLD, 1)
        pathlib.Path(args.write_cut).write_text(out)
        print(f"wrote {args.write_cut}: {len(moved)} items cut")
        return
    head_text = (root / REL_MAIN).read_text()
    # A repointed source-text test may differ; nothing else around it may.
    head_text, head_checks = mask_text_checks(head_text, errors, f"{REL_MAIN} (head)")
    expected, base_checks = mask_text_checks(expected, errors, f"{REL_MAIN} (base)")
    changed_checks = [n for n in TEXT_CHECK_TESTS if head_checks.get(n) != base_checks.get(n)]
    for name in TEXT_CHECK_TESTS:
        if name in changed_checks and name not in args.repointed:
            errors.append(f"{REL_MAIN}: {name} changed; pass --repointed {name} if this step repoints it")
        elif name in args.repointed and name not in changed_checks:
            errors.append(f"{REL_MAIN}: --repointed {name}, but it is unchanged")
    if has_scaffold:
        if head_text != expected:
            a, b = head_text.split("\n"), expected.split("\n")
            k = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            errors.append(
                f"{REL_MAIN}: differs from the base minus the moved items, first at line {k + 1}: "
                f"head {a[k] if k < len(a) else '<eof>'!r} vs expected {b[k] if k < len(b) else '<eof>'!r}"
            )
    elif expected.count(SCAFFOLD_AFTER) != 1:
        errors.append(f"{REL_MAIN}: base has no single {SCAFFOLD_AFTER!r} to put the scaffold after")
    elif head_text != expected.replace(SCAFFOLD_AFTER, SCAFFOLD_AFTER + SCAFFOLD, 1):
        if head_text.count(SCAFFOLD) == 1 and head_text.replace(SCAFFOLD, "", 1) == expected:
            errors.append(f"{REL_MAIN}: the scaffold block must come right after {SCAFFOLD_AFTER!r}")
        else:
            a = head_text.replace(SCAFFOLD, "", 1).split("\n")
            b = expected.split("\n")
            k = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            errors.append(
                f"{REL_MAIN}: differs from the base minus the moved items, first at line {k + 1}: "
                f"head {a[k] if k < len(a) else '<eof>'!r} vs expected {b[k] if k < len(b) else '<eof>'!r}"
            )

    for line in summary:
        print(line)
    for line in reflowed:
        print(f"reflowed declaration (rustfmt after pub(crate)): {line}")
    for line in rewrapped:
        print(f"match arm put in a block (rustfmt after the target pin): {line}")
    for name in changed_checks:
        print(f"source-text test in main.rs changed (MAIN-14 repoint, review its diff): {name}")
    print(f"moved: {len(moved)} items, {sum(i['end'] - i['start'] + 1 for i in moved)} base lines")
    if tests_seen:
        print(f"test files under src/daemon/ (reached, not item-checked): {len(tests_seen)}")
    if base_daemon:
        print(f"kept: {kept_total} items already under src/daemon/ in the base, byte for byte")
    if errors:
        print("\nFAIL:")
        for e in errors:
            print("  " + e)
        sys.exit(1)
    print(
        f"OK: every moved item matches the base byte for byte ({len(reflowed)} with a rustfmt-reflowed "
        f"declaration head, {len(rewrapped)} with a match arm rustfmt put in a block), "
        "and main.rs is the base minus them"
    )


if __name__ == "__main__":
    main()
