#!/usr/bin/env python3
"""List the daemon binary's tests without building it (0.6 set A).

Prints the same test lines as

    cargo test -p scuffed-stat-tracker --bin scuffed-stat-tracker -- --list

(one `path::to::test: test` line per test, sorted), read from the source
instead of a test build. Set A PRs move test files from `src/main_tests/` to
`src/daemon/`; this script gives their base and head lists for
`check-test-renames.py` on a machine that cannot run the tests.

How it reads the tree:

- It starts at `src/main.rs`, the bin's crate root, and follows every
  `mod x;` declaration to `x.rs` or `x/mod.rs` (beside a `main.rs` or
  `mod.rs`, or in the `foo/` directory of a `foo.rs`), or to the file a
  `#[path = "..."]` attribute names. Inline `mod x { ... }` blocks nest the
  module path and the directory the same way.
- Modules under `#[cfg(test)]` are followed (the list is of a test build);
  a module under `#[cfg(not(test))]` is not.
- A test is a `fn` or `async fn` at module level (in a file or a mod block,
  not inside a function or impl) whose attributes include `#[test]` or
  `#[tokio::test]` (with or without arguments). `#[ignore]` tests are
  listed, as `--list` lists them.
- Comments, strings and chars are blanked first, so a needle string that
  holds `#[test]` or `mod x;` is not read as code.

It does not expand macros: a macro that defines tests, or any other test
attribute, fails the run instead of being left out (`--list` would show
the test). Any other `cfg` on a module is followed and reported on stderr.

Usage (from anywhere in the repo):

    python3 -I crates/stat-tracker/scripts/list-bin-tests.py > head-list.txt
    python3 -I crates/stat-tracker/scripts/list-bin-tests.py --src DIR

`--src` reads another checkout's `crates/stat-tracker/src` (for the base of
a PR, check it out in a throwaway worktree). Python standard library only.
"""

import argparse
import pathlib
import re
import sys

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
TEST_ATTRS = ("test", "tokio::test")


def blank(text):
    """Replace comments, string and char literals by spaces (keep newlines
    and length, so offsets in the result are offsets in `text`)."""
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


ATTR = re.compile(r"#\s*\[")
ITEM = re.compile(
    r"(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:async|unsafe|const|extern(?:\s+\"[^\"]*\")?)\s+)*"
    r"(fn|mod)\s+(" + IDENT + r")\s*([;{(<])"
)
STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')


def attr_end(code, at):
    """End offset of the attribute whose `#` is at `at` (after its `]`)."""
    k = code.index("[", at)
    depth = 0
    while k < len(code):
        if code[k] in "[(":
            depth += 1
        elif code[k] in "])":
            depth -= 1
            if depth == 0:
                return k + 1
        k += 1
    raise SystemExit("unterminated attribute")


def skip_ws(code, at):
    while at < len(code) and code[at].isspace():
        at += 1
    return at


class Lister:
    def __init__(self, src):
        self.src = src
        self.tests = []
        self.notes = []
        self.errors = []

    def where(self, path, text, at):
        line = text.count("\n", 0, at) + 1
        return f"{path.relative_to(self.src).as_posix()}:{line}"

    def file(self, path, modpath, child_dir):
        """Read one module file. `child_dir`: where its `mod x;` children live."""
        text = path.read_text(encoding="utf-8")
        self.block(path, text, blank(text), 0, len(text), modpath, child_dir, False)

    def block(self, path, text, code, start, end, modpath, child_dir, inline):
        """Walk module-level code in [start, end): attributes, then items.
        Brace blocks of non-module items are skipped whole. `inline`: the
        code is a `mod x { ... }` block's."""
        at = start
        attrs = []
        while True:
            at = skip_ws(code, at)
            if at >= end:
                return
            if code.startswith("#!", at):
                at = attr_end(code, at)
                continue
            if ATTR.match(code, at):
                e = attr_end(code, at)
                attrs.append((code[at:e], text[at:e], at))
                at = e
                continue
            m = ITEM.match(code, at)
            if m and m.group(1) == "mod":
                at = self.module(path, text, code, m, attrs, modpath, child_dir, inline)
            elif m and m.group(1) == "fn":
                self.function(path, text, m, attrs, modpath)
                at = self.item_end(code, m.end() - 1, end)
            else:
                for a, _, a_at in attrs:
                    if self.test_attr(a):
                        self.errors.append(
                            f"{self.where(path, text, a_at)}: test attribute on something not a fn"
                        )
                if re.match(r"macro_rules\s*!", code[at:]):
                    self.errors.append(
                        f"{self.where(path, text, at)}: macro_rules! (not expanded; it may define tests)"
                    )
                at = self.item_end(code, at, end)
            attrs = []

    def item_end(self, code, at, end):
        """Offset right after the item starting at `at`: its first `;` at
        depth 0, or the `}` closing its first top-level brace block."""
        depth = 0
        while at < end:
            c = code[at]
            if c in "([":
                depth += 1
            elif c in ")]":
                depth -= 1
            elif c == "{":
                k = self.close(code, at)
                if depth == 0:
                    # `struct X { .. }`, `fn f() { .. }`, `impl X { .. }`: done.
                    # A `where`-less item ends here; `const X: T = T { .. };`
                    # goes on to its `;`.
                    rest = skip_ws(code, k + 1)
                    if rest < end and code[rest] == ";":
                        return rest + 1
                    return k + 1
                at = k
            elif c == ";" and depth == 0:
                return at + 1
            at += 1
        return end

    def close(self, code, at):
        """Offset of the `}` matching the `{` at `at`."""
        depth = 0
        for k in range(at, len(code)):
            if code[k] == "{":
                depth += 1
            elif code[k] == "}":
                depth -= 1
                if depth == 0:
                    return k
        raise SystemExit("unbalanced braces")

    def test_attr(self, code_attr):
        m = re.match(r"#\s*\[\s*([A-Za-z_:\s]+?)\s*(?:\(|\])", code_attr)
        name = re.sub(r"\s+", "", m.group(1)) if m else ""
        return name in TEST_ATTRS or name.endswith("::test") or name == "rstest"

    def function(self, path, text, m, attrs, modpath):
        names = []
        for a, _, a_at in attrs:
            mm = re.match(r"#\s*\[\s*([A-Za-z_:\s]+?)\s*(?:\(|\])", a)
            names.append(re.sub(r"\s+", "", mm.group(1)) if mm else "")
        if any(n in TEST_ATTRS for n in names):
            self.tests.append("::".join(modpath + [m.group(2)]))
        elif any(self.test_attr(a) for a, _, _ in attrs):
            self.errors.append(
                f"{self.where(path, text, m.start())}: fn {m.group(2)}: unknown test attribute"
            )

    def module(self, path, text, code, m, attrs, modpath, child_dir, inline):
        name = m.group(2)
        cfgs = [raw for a, raw, _ in attrs if re.match(r"#\s*\[\s*cfg\s*\(", a)]
        for raw in cfgs:
            inner = re.sub(r"\s+", "", raw)
            if "not(test)" in inner:
                return self.item_end(code, m.end() - 1, len(code))
            if inner != "#[cfg(test)]":
                self.notes.append(f"{self.where(path, text, m.start())}: mod {name}: {raw} followed")
        pathattr = None
        for a, raw, _ in attrs:
            if re.match(r"#\s*\[\s*path\s*=", a):
                s = STRING.search(raw)
                pathattr = s.group(1) if s else None
        sub = modpath + [name]
        if m.group(3) == "{":
            open_at = m.end() - 1
            close_at = self.close(code, open_at)
            inner_dir = child_dir / (pathattr if pathattr else name)
            self.block(path, text, code, open_at + 1, close_at, sub, inner_dir, True)
            return close_at + 1
        if m.group(3) != ";":
            raise SystemExit(f"{self.where(path, text, m.start())}: mod {name}: unexpected text")
        if pathattr:
            # Outside a mod block, `#[path]` is relative to the directory of
            # the file it is written in; inside one, to the block's directory.
            candidates = [(child_dir if inline else path.parent) / pathattr]
        else:
            candidates = [child_dir / f"{name}.rs", child_dir / name / "mod.rs"]
        found = [c for c in candidates if c.is_file()]
        if len(found) != 1:
            self.errors.append(
                f"{self.where(path, text, m.start())}: mod {name}: "
                f"{'no file' if not found else 'both files'} among "
                + ", ".join(c.relative_to(self.src).as_posix() for c in candidates)
            )
            return m.end()
        target = found[0]
        if target.name == "mod.rs" or pathattr:
            # A mod.rs, or a #[path] file, keeps its children beside it.
            # (rustc: a #[path] file is treated as a mod-rs file.)
            self.file(target, sub, target.parent)
        else:
            self.file(target, sub, target.parent / target.stem)
        return m.end()


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument(
        "--src",
        help="the crate's src/ directory (default: crates/stat-tracker/src of this checkout)",
    )
    args = ap.parse_args()
    src = pathlib.Path(args.src) if args.src else pathlib.Path(__file__).resolve().parent.parent / "src"
    src = src.resolve()
    lister = Lister(src)
    root = src / "main.rs"
    lister.file(root, [], src)
    for note in lister.notes:
        print(f"note: {note}", file=sys.stderr)
    if len(lister.tests) != len(set(lister.tests)):
        lister.errors.append("a test name is listed twice")
    if lister.errors:
        for e in lister.errors:
            print(f"error: {e}", file=sys.stderr)
        sys.exit(1)
    for t in sorted(lister.tests):
        print(f"{t}: test")


if __name__ == "__main__":
    main()
