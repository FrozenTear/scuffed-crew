#!/usr/bin/env python3
"""Check that a 0.6 set A PR (daemon tests move next to their modules) kept every test name.

Compares two `cargo test -p scuffed-stat-tracker --bin scuffed-stat-tracker -- --list`
outputs, base and head, through a rename map of old to new module prefixes. Where the
tests cannot be built, `list-bin-tests.py` prints the same lines from the source. For
example

    main_tests::splits_and_debug_frames:: -> daemon::tests_debug_dumps::

The check passes only when:

1. base and head list the same number of tests;
2. every map entry's old prefix is used by at least one base test, and no two
   entries share an old or a new prefix;
3. every base test under an old prefix appears in head under the new prefix,
   with the same remainder (the leaf name and any inner module path);
4. every base test under no old prefix appears in head unchanged;
5. head has no test that is not accounted for by 3 or 4.

So the leaf names, the count and every unmoved path are identical, and the only
difference is the documented prefix swap. It reads names only, not test bodies:
`check-test-moves.py` checks that each moved file's text is unchanged.

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

All three checks must print OK. The full account of what each proves is in
`check-test-moves.py`.

Usage (from anywhere in the repo):

    python3 -I crates/stat-tracker/scripts/check-test-renames.py BASE_LIST HEAD_LIST MAP

MAP has one `old -> new` line per moved file; blank lines and `#` comments are
ignored. The list files are raw `--list` output: lines ending in `: test` are
tests, everything else is ignored. Python standard library only.
"""

import sys


def tests(path):
    out = []
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.endswith(": test"):
            out.append(line[: -len(": test")])
    if len(out) != len(set(out)):
        sys.exit(f"{path}: a test is listed twice")
    return out


def rename_map(path):
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
        pairs.append((old, new))
    olds = [o for o, _ in pairs]
    news = [n for _, n in pairs]
    if len(set(olds)) != len(olds) or len(set(news)) != len(news):
        sys.exit(f"{path}: an old or new prefix is listed twice")
    for a in olds:
        for b in olds:
            if a != b and b.startswith(a):
                sys.exit(f"{path}: old prefix {b!r} is inside {a!r}")
    return pairs


def main():
    if len(sys.argv) != 4:
        sys.exit("usage: " + __doc__.split("Usage (from anywhere in the repo):")[1].split("MAP has")[0].strip())
    base, head = tests(sys.argv[1]), tests(sys.argv[2])
    pairs = rename_map(sys.argv[3])
    errors = []
    if len(base) != len(head):
        errors.append(f"count differs: base {len(base)}, head {len(head)}")
    expected = {}
    used = {old: 0 for old, _ in pairs}
    for t in base:
        hit = [(o, n) for o, n in pairs if t.startswith(o)]
        if hit:
            o, n = hit[0]
            used[o] += 1
            expected[n + t[len(o):]] = t
        else:
            expected[t] = t
    for o, n in used.items():
        if n == 0:
            errors.append(f"map entry {o!r} matches no base test")
    head_set = set(head)
    for new, old in sorted(expected.items()):
        if new not in head_set:
            errors.append(f"missing at head: {new} (base {old})")
    for t in sorted(head_set - set(expected)):
        errors.append(f"unmapped at head: {t}")
    moved = sum(used.values())
    print(f"base {len(base)} tests, head {len(head)} tests, {moved} renamed by {len(pairs)} map entries, "
          f"{len(base) - moved} unchanged")
    for o, n in pairs:
        print(f"  {o} -> {n}: {used[o]} tests")
    if errors:
        print("\nFAIL:")
        for e in errors:
            print("  " + e)
        sys.exit(1)
    print("OK: the head list is the base list with only the mapped prefixes renamed")


if __name__ == "__main__":
    main()
