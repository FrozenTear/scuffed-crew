#!/usr/bin/env python3
"""Check that a 0.6 set A PR moved each daemon test file without changing it.

Set A moves test files from `src/main_tests/<name>.rs` to `src/daemon/`,
next to the code they test, as `tests_x.rs` beside a daemon `mod.rs`. Given
the PR's base revision and the same rename MAP that `check-test-renames.py`
takes, for example

    main_tests::poll_cadence:: -> daemon::poll::tests_cadence::

(old file `src/main_tests/poll_cadence.rs`, new file
`src/daemon/poll/tests_cadence.rs`, declared in `src/daemon/poll/mod.rs`),
the check passes only when, for every entry:

1. the old prefix is `main_tests::<name>::` and the new one is
   `daemon::[<dir>::...]tests_<x>::`;
2. the old file exists at the base and is gone at head, and the new file
   exists at head and not at the base;
3. the new file equals the base old file byte for byte, except:
   - its one `use super::*;` line is replaced by exactly the two lines
     `use crate::*;` and `use crate::main_tests::*;` (so it reaches the
     crate root and the shared helpers in `main_tests/mod.rs`);
   - the first line, when it is the generated header (`//! Daemon tests:
     <name>.` and a pointer to `mod.rs`), may be reworded to any one `//!`
     line;
   - a string literal holding a test path under a map entry's old prefix
     (the own-process tests name themselves, for one
     `"main_tests::ocr_off_switch::own_process_tab_pass_with_ocr_off"`)
     is rewritten to the entry's new prefix, with the same remainder.
   Every other line is identical, so a weakened assert, an added
   `#[ignore]`, a changed comment, an extra line or a test path string that
   does not follow the map all fail. Each reworded header and repointed
   string is listed in the output.
4. the parent `mod.rs` is the base one plus exactly a `#[cfg(test)]` line
   followed by `mod tests_<x>;` for each file mapped into it, and nothing
   else;
5. `src/main_tests/mod.rs` is the base one minus exactly the `mod <name>;`
   line of each mapped file, and nothing else.

At head, no string literal in a `.rs` file under `crates/stat-tracker/src`,
and no line in `.github/workflows`, may still name a test under a mapped old
prefix (a CI `--exact` filter on an old path would match nothing).
Mentions of an old file path (`main_tests/<name>.rs`) anywhere under
`crates/` or `.github/` are listed as notes to check by hand (a size cap,
a build id rule), since synthetic test trees use such paths too.

Head is the working tree, as in `check-daemon-split.py`.

How the set A scripts run together, for each set A PR (BASE is the PR's
base, for example origin/wip/0.6; BASE_DIR is a scratch directory; MAP has
one `old -> new` line per moved file):

    git worktree add --detach BASE_DIR BASE
    python3 -I crates/stat-tracker/scripts/list-bin-tests.py \\
        --src BASE_DIR/crates/stat-tracker/src > base-list.txt
    python3 -I crates/stat-tracker/scripts/list-bin-tests.py > head-list.txt
    python3 -I crates/stat-tracker/scripts/check-test-renames.py \\
        base-list.txt head-list.txt MAP
    python3 -I crates/stat-tracker/scripts/check-test-moves.py --base BASE MAP
    python3 -I crates/stat-tracker/scripts/check-daemon-split.py --base BASE
    git worktree remove BASE_DIR

check-test-renames.py proves the names (same count, only the mapped
prefixes renamed, and the lister evaluates cfg, so a test compiled out
changes the count). This script proves each moved file's text and its
`mod` lines. check-daemon-split.py proves `main.rs` and every daemon file
that is not a test file are unchanged apart from the split's own moves.
All three must print OK. Where the tests can be built, real
`cargo test -p scuffed-stat-tracker --bin scuffed-stat-tracker -- --list`
outputs of base and head can replace the two list files.

Usage (from anywhere in the repo):

    python3 -I crates/stat-tracker/scripts/check-test-moves.py --base REV MAP

MAP has one `old -> new` line per moved file; blank lines and `#` comments
are ignored. Exit status 0 means every move is pure. Python standard
library only.
"""

import argparse
import difflib
import pathlib
import re
import subprocess
import sys

REL_SRC = "crates/stat-tracker/src"
OLD_PREFIX = re.compile(r"^main_tests::([a-z_][a-z0-9_]*)::$")
NEW_PREFIX = re.compile(r"^daemon::((?:[a-z_][a-z0-9_]*::)*)(tests_[a-z0-9_]+)::$")
GENERATED_HEADER = re.compile(r"^//! Daemon tests: [a-z0-9_]+\. See `mod\.rs`\.$")
OLD_IMPORT = "use super::*;"
NEW_IMPORT = ["use crate::*;", "use crate::main_tests::*;"]
CFG_TEST = "#[cfg(test)]"
LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')


def git(*args, cwd, check=True):
    return subprocess.run(["git", *args], cwd=cwd, check=check, capture_output=True, text=True)


def at_base(root, base, rel):
    """The file's text at `base`, or None when it does not exist there."""
    r = git("show", f"{base}:{rel}", cwd=root, check=False)
    return r.stdout if r.returncode == 0 else None


def rename_map(path):
    """The MAP file, read with the rules of check-test-renames.py."""
    pairs = []
    for n, line in enumerate(open(path, encoding="utf-8"), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        if "->" not in line:
            sys.exit(f"{path}:{n}: expected `old -> new`")
        old, new = (p.strip() for p in line.split("->", 1))
        if not old.endswith("::") or not new.endswith("::"):
            sys.exit(f"{path}:{n}: prefixes must end with `::`")
        pairs.append((old, new, n))
    olds = [o for o, _, _ in pairs]
    news = [w for _, w, _ in pairs]
    if len(set(olds)) != len(olds) or len(set(news)) != len(news):
        sys.exit(f"{path}: an old or new prefix is listed twice")
    if not pairs:
        sys.exit(f"{path}: no map entries")
    return pairs


def repoint(line, pairs):
    """`line` with every string literal under an old prefix rewritten to
    its new prefix: (new line, [(old string, new string)])."""
    changes = []

    def swap(m):
        body = m.group(1)
        for old, new, _ in pairs:
            if body.startswith(old):
                changes.append((body, new + body[len(old):]))
                return '"' + new + body[len(old):] + '"'
        return m.group(0)

    return LITERAL.sub(swap, line), changes


def diff(a, b, a_name, b_name, limit=40):
    out = list(difflib.unified_diff(a, b, a_name, b_name, lineterm="", n=1))
    if len(out) > limit:
        out = out[:limit] + [f"... ({len(out) - limit} more diff lines)"]
    return ["    " + line for line in out]


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--base", required=True, help="base revision, e.g. origin/wip/0.6")
    ap.add_argument("map", metavar="MAP", help="rename map, as for check-test-renames.py")
    args = ap.parse_args()

    here = pathlib.Path(__file__).resolve().parent
    root = pathlib.Path(git("rev-parse", "--show-toplevel", cwd=here).stdout.strip())
    git("rev-parse", "--verify", f"{args.base}^{{commit}}", cwd=root)
    pairs = rename_map(args.map)
    errors, notes, report = [], [], []

    moves = []  # (name, old_rel, new_rel, parent_rel, tests_mod)
    for old, new, n in pairs:
        mo, mn = OLD_PREFIX.match(old), NEW_PREFIX.match(new)
        if not mo or not mn:
            errors.append(
                f"{args.map}:{n}: expected `main_tests::<name>:: -> daemon::[<dir>::...]tests_<x>::`"
            )
            continue
        dirs = "".join(d + "/" for d in mn.group(1).split("::") if d)
        name, tests_mod = mo.group(1), mn.group(2)
        moves.append((
            name,
            f"{REL_SRC}/main_tests/{name}.rs",
            f"{REL_SRC}/daemon/{dirs}{tests_mod}.rs",
            f"{REL_SRC}/daemon/{dirs}mod.rs",
            tests_mod,
        ))

    # 2 and 3: each moved file.
    for name, old_rel, new_rel, _, _ in moves:
        base_text = at_base(root, args.base, old_rel)
        if base_text is None:
            errors.append(f"{old_rel}: not in the base")
        if (root / old_rel).exists():
            errors.append(f"{old_rel}: still there at head")
        if at_base(root, args.base, new_rel) is not None:
            errors.append(f"{new_rel}: already in the base")
        if not (root / new_rel).is_file():
            errors.append(f"{new_rel}: missing at head")
        if base_text is None or not (root / new_rel).is_file():
            continue
        head_bytes = (root / new_rel).read_bytes()
        try:
            head_text = head_bytes.decode("utf-8")
        except UnicodeDecodeError:
            errors.append(f"{new_rel}: not UTF-8")
            continue
        base_lines, head_lines = base_text.split("\n"), head_text.split("\n")
        imports = [i for i, line in enumerate(base_lines) if line == OLD_IMPORT]
        if len(imports) != 1:
            errors.append(f"{old_rel}: base has {len(imports)} `{OLD_IMPORT}` lines, expected 1")
            continue
        expected, repointed = [], []
        for i, line in enumerate(base_lines):
            if i == imports[0]:
                expected.extend(NEW_IMPORT)
                continue
            line, changes = repoint(line, [(o, w, n) for o, w, n in pairs])
            for before, after in changes:
                repointed.append((len(expected) + 1, before, after))
            expected.append(line)
        reworded = None
        if (
            GENERATED_HEADER.match(base_lines[0])
            and head_lines
            and head_lines[0] != base_lines[0]
            and (head_lines[0] == "//!" or head_lines[0].startswith("//! "))
        ):
            reworded = head_lines[0]
            expected[0] = head_lines[0]
        if head_lines != expected:
            errors.append(
                f"{new_rel}: differs from the base {old_rel} beyond the allowed edits "
                "(the import line, the header line, mapped test path strings):"
            )
            errors.extend(diff(expected, head_lines, f"expected {new_rel}", f"head {new_rel}"))
            continue
        report.append(f"  {old_rel} -> {new_rel}: {len(head_lines) - 1} lines, import line swapped")
        if reworded is not None:
            report.append(f"    header reworded: {base_lines[0]!r} -> {reworded!r}")
        for line_no, before, after in repointed:
            report.append(f"    repointed at line {line_no}: \"{before}\" -> \"{after}\"")

    # 4: the parent mod.rs files.
    by_parent = {}
    for _, _, _, parent_rel, tests_mod in moves:
        by_parent.setdefault(parent_rel, []).append(tests_mod)
    for parent_rel, mods in sorted(by_parent.items()):
        base_text = at_base(root, args.base, parent_rel)
        if base_text is None:
            errors.append(f"{parent_rel}: not in the base (set A adds no new daemon module)")
            continue
        if not (root / parent_rel).is_file():
            errors.append(f"{parent_rel}: missing at head")
            continue
        base_lines = base_text.split("\n")
        head_lines = (root / parent_rel).read_text(encoding="utf-8").split("\n")
        rest, ok = list(head_lines), True
        for tests_mod in mods:
            decl = f"mod {tests_mod};"
            if decl in base_lines:
                errors.append(f"{parent_rel}: base already declares `{decl}`")
                ok = False
                continue
            hits = [i for i in range(len(rest) - 1) if rest[i] == CFG_TEST and rest[i + 1] == decl]
            if len(hits) != 1:
                errors.append(
                    f"{parent_rel}: expected one `{CFG_TEST}` line then `{decl}`, found {len(hits)}"
                )
                ok = False
                continue
            del rest[hits[0]:hits[0] + 2]
        if ok and rest != base_lines:
            errors.append(f"{parent_rel}: changed beyond the added test module lines:")
            errors.extend(diff(base_lines, rest, f"base {parent_rel}", f"head {parent_rel} minus them"))
        elif ok:
            report.append(f"  {parent_rel}: + " + ", ".join(f"mod {m};" for m in mods))

    # 5: main_tests/mod.rs.
    hub_rel = f"{REL_SRC}/main_tests/mod.rs"
    base_text = at_base(root, args.base, hub_rel)
    if base_text is None or not (root / hub_rel).is_file():
        errors.append(f"{hub_rel}: missing at the base or at head")
    else:
        rest, ok = base_text.split("\n"), True
        for name, *_ in moves:
            decl = f"mod {name};"
            hits = [i for i, line in enumerate(rest) if line == decl]
            if len(hits) != 1:
                errors.append(f"{hub_rel}: base has {len(hits)} `{decl}` lines, expected 1")
                ok = False
                continue
            del rest[hits[0]]
        head_lines = (root / hub_rel).read_text(encoding="utf-8").split("\n")
        if ok and rest != head_lines:
            errors.append(f"{hub_rel}: changed beyond the removed `mod` lines:")
            errors.extend(diff(rest, head_lines, f"base {hub_rel} minus them", f"head {hub_rel}"))
        elif ok:
            report.append(f"  {hub_rel}: - " + ", ".join(f"mod {m[0]};" for m in moves))

    # Stale references at head.
    listed = git(
        "ls-files", "-co", "--exclude-standard", "--", "crates", ".github", cwd=root
    ).stdout.splitlines()
    olds = [o for o, _, _ in pairs]
    old_files = [f"main_tests/{m[0]}.rs" for m in moves]
    for rel in sorted(set(listed)):
        path = root / rel
        if not path.is_file() or rel.endswith(".py"):
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        in_src = rel.startswith(REL_SRC + "/") and rel.endswith(".rs")
        in_ci = rel.startswith(".github/workflows/")
        for k, line in enumerate(text.split("\n"), 1):
            if in_src:
                for m in LITERAL.finditer(line):
                    if any(re.search(r"(?<![A-Za-z0-9_])" + re.escape(o), m.group(1)) for o in olds):
                        errors.append(f"{rel}:{k}: still names an old test path: \"{m.group(1)}\"")
            elif in_ci and any(re.search(r"(?<![A-Za-z0-9_])" + re.escape(o), line) for o in olds):
                errors.append(f"{rel}:{k}: still names an old test path: {line.strip()}")
            for f in old_files:
                if f in line:
                    notes.append(f"{rel}:{k}: names {f}: {line.strip()}")

    print(f"{len(pairs)} map entries, {len(moves)} moves checked against {args.base}")
    for line in report:
        print(line)
    for note in notes:
        print(f"note: {note}")
    if errors:
        print("\nFAIL:")
        for e in errors:
            print("  " + e)
        sys.exit(1)
    print("OK: each moved test file is its base file with only the allowed edits, "
          "and only the matching mod lines changed")


if __name__ == "__main__":
    main()
