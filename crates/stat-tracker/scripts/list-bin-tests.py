#!/usr/bin/env python3

Known limit: a test attribute is recognised as `#[test]`, `#[tokio::test]`
or `#[tokio::test(...)]`. Path forms such as `#[core::prelude::v1::test]`
and a renamed import (`use core::prelude::v1::test as check;`) are not
recognised. This crate uses neither, and in a set A PR check-test-moves.py
keeps every test file byte-identical apart from the move, so none can
appear there unnoticed.
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
- `cfg` is evaluated for the build that `--list` describes: a test build
  (`test` holds) of the dev profile, which `test` inherits (`debug_assertions`
  holds), on Linux x86_64 (`unix`, `target_os = "linux"`, `target_family =
  "unix"`, `target_endian = "little"`, `target_pointer_width = "64"`,
  `target_arch = "x86_64"`; `windows` does not hold), with no cargo
  feature on (the crate has no `[features]` table; if it gets one, a
  `feature = "..."` predicate cannot be evaluated). `not(...)`, `any(...)`
  and `all(...)` nest as in rustc, `any()` is false and `all()` is true.
  It is applied to `#[cfg(...)]` on a `mod` (a false one is not followed),
  to an inner `#![cfg(...)]` of a module file or mod block (a false one
  drops the whole module), and to `#[cfg(...)]` on a test fn (a false one
  is not listed). `#[cfg_attr(pred, attrs)]` on a mod or fn adds `attrs`
  when `pred` holds, so `#[cfg_attr(test, test)]` makes a test.
  A predicate it cannot evaluate there (another name or key, a feature
  when the crate has features, a malformed predicate) fails the run with
  its file and line, rather than the test being listed or dropped by a
  guess. When any or all already decides it, the unknown part does not
  matter (`any(test, loom)` holds). A cfg_attr it cannot evaluate fails
  the run only if its attributes could change the list (`cfg`, `cfg_attr`,
  `path` or a test attribute); `#[cfg_attr(not(test), allow(dead_code))]`
  never matters. Every cfg other than plain `test` or `not(test)` that
  decides a mod, a module or a test is reported on stderr with its result.
- A test is a `fn` or `async fn` at module level (in a file or a mod block,
  not inside a function or impl) whose attributes include `#[test]` or
  `#[tokio::test]` (with or without arguments). `#[ignore]` tests are
  listed, as `--list` lists them.
- Comments, strings and chars are blanked first, so a needle string that
  holds `#[test]` or `mod x;` is not read as code.

It does not expand macros: a macro that defines tests, or any other test
attribute, fails the run instead of being left out (`--list` would show
the test).

Usage (from anywhere in the repo):

    python3 -I crates/stat-tracker/scripts/list-bin-tests.py > head-list.txt
    python3 -I crates/stat-tracker/scripts/list-bin-tests.py --src DIR
    python3 -I crates/stat-tracker/scripts/list-bin-tests.py --self-test

`--src` reads another checkout's `crates/stat-tracker/src` (for the base of
a PR, check it out in a throwaway worktree); `Cargo.toml` beside it says
whether the crate has features. `--self-test` runs the lister on small
synthetic trees in a temporary directory (cfg forms, cfg_attr, inner cfg,
comments and strings that look like code, and each failing form) and exits
0 only if every case gives the expected list or error.

For a set A PR, its base and head lists feed `check-test-renames.py`; the
docstring of `check-test-moves.py` shows the whole run of set A checks.

Python standard library only.
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


# The build `cargo test --bin scuffed-stat-tracker -- --list` describes.
CFG_NAMES = {"test": True, "debug_assertions": True, "unix": True, "windows": False}
CFG_KEYS = {
    "target_os": "linux",
    "target_family": "unix",
    "target_endian": "little",
    "target_pointer_width": "64",
    "target_arch": "x86_64",
}
CFG_TOKEN = re.compile(r'\s*(?:(' + IDENT + r')|("(?:[^"\\]|\\.)*")|([(),=]))')


class CfgError(Exception):
    pass


def cfg_tokens(text):
    out, at = [], 0
    while at < len(text):
        if text[at:].strip() == "":
            break
        m = CFG_TOKEN.match(text, at)
        if not m:
            raise CfgError(f"cannot read {text[at:]!r}")
        out.append(m.group(1) or m.group(2) or m.group(3))
        at = m.end()
    return out


def cfg_parse(toks, i=0):
    """One predicate from toks[i:]: (node, next index)."""
    if i >= len(toks) or not re.fullmatch(IDENT, toks[i]):
        raise CfgError("expected a cfg name")
    name = toks[i]
    if name in ("any", "all", "not") and i + 1 < len(toks) and toks[i + 1] == "(":
        i += 2
        args = []
        while toks[i:i + 1] != [")"]:
            node, i = cfg_parse(toks, i)
            args.append(node)
            if toks[i:i + 1] == [","]:
                i += 1
            elif toks[i:i + 1] != [")"]:
                raise CfgError("expected `,` or `)`")
        if name == "not" and len(args) != 1:
            raise CfgError("not(...) takes one predicate")
        return (name, args), i + 1
    if toks[i + 1:i + 2] == ["="]:
        if i + 2 >= len(toks) or not toks[i + 2].startswith('"'):
            raise CfgError(f"expected a string after `{name} =`")
        return ("kv", name, toks[i + 2][1:-1]), i + 3
    return ("name", name), i + 1


def cfg_eval(node, features):
    """True, False, or None (cannot be evaluated), with the unknown parts."""
    kind = node[0]
    if kind == "name":
        v = CFG_NAMES.get(node[1])
        return v, ([] if v is not None else [node[1]])
    if kind == "kv":
        key, val = node[1], node[2]
        if key == "feature":
            if features:
                return None, [f'feature = "{val}"']
            return False, []
        if key in CFG_KEYS:
            return CFG_KEYS[key] == val, []
        return None, [f'{key} = "{val}"']
    results = [cfg_eval(a, features) for a in node[1]]
    unknown = [u for _, us in results for u in us]
    vals = [v for v, _ in results]
    if kind == "not":
        return (None if vals[0] is None else not vals[0]), unknown
    if kind == "any":
        if True in vals:
            return True, []
        return (None if None in vals else False), unknown
    if False in vals:  # all
        return False, []
    return (None if None in vals else True), unknown


def cfg_value(pred_text, features):
    """Evaluate a cfg predicate's text: (True | False | None, why)."""
    try:
        toks = cfg_tokens(pred_text)
        node, i = cfg_parse(toks)
        if i != len(toks):
            raise CfgError("text after the predicate")
    except (CfgError, IndexError) as e:
        return None, f"malformed predicate ({e})"
    v, unknown = cfg_eval(node, features)
    return v, ("cannot evaluate " + ", ".join(unknown)) if v is None else ""


def attr_parts(raw):
    """`#[name(args)]` or `#![name(args)]` -> (name, args or None)."""
    m = re.match(r"#\s*!?\s*\[\s*([A-Za-z_:\s]+?)\s*(\(|\]|=)", raw)
    if not m:
        return "", None
    name = re.sub(r"\s+", "", m.group(1))
    if m.group(2) != "(":
        return name, None
    depth, k = 0, m.end() - 1
    for k in range(m.end() - 1, len(raw)):
        if raw[k] == "(":
            depth += 1
        elif raw[k] == ")":
            depth -= 1
            if depth == 0:
                break
    return name, raw[m.end():k]


def split_top(text):
    """Split at commas outside brackets and strings."""
    parts, depth, cur, k = [], 0, "", 0
    while k < len(text):
        c = text[k]
        if c == '"':
            j = k + 1
            while j < len(text) and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            cur += text[k:j + 1]
            k = j + 1
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        if c == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += c
        k += 1
    if cur.strip():
        parts.append(cur)
    return [p.strip() for p in parts]



class Lister:
    def __init__(self, src, features):
        self.src = src
        self.features = features  # the crate has a [features] table
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
                e = attr_end(code, at)
                if attr_parts(code[at:e])[0] in ("cfg", "cfg_attr"):
                    label = "module " + ("::".join(modpath) or "crate root")
                    inner = self.expand(path, text, [(code[at:e], text[at:e], at)], label)
                    if self.cfg_holds(path, text, inner, label) is not True:
                        # False: the module is compiled out, so none of it
                        # counts. None: the error is recorded already.
                        return
                at = e
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

    def expand(self, path, text, attrs, label):
        """`attrs` with each `cfg_attr(pred, a, b)` replaced by `a`, `b`
        when `pred` holds and dropped when it does not. One whose `pred`
        cannot be evaluated is an error when its attributes could change
        the list, and dropped otherwise."""
        out = []
        for a, raw, a_at in attrs:
            name, args = attr_parts(raw)
            if name != "cfg_attr":
                out.append((a, raw, a_at))
                continue
            where = self.where(path, text, a_at)
            parts = split_top(args or "")
            if len(parts) < 2:
                self.errors.append(f"{where}: {label}: {raw}: malformed cfg_attr")
                continue
            inner = ["#[" + part + "]" for part in parts[1:]]
            v, why = cfg_value(parts[0], self.features)
            if v is None:
                if any(attr_parts(r)[0] in ("cfg", "cfg_attr", "path") or self.test_attr(blank(r))
                       for r in inner):
                    self.errors.append(f"{where}: {label}: {raw}: {why}")
                continue
            if re.sub(r"\s+", "", parts[0]) not in ("test", "not(test)"):
                self.notes.append(f"{where}: {label}: {raw}: {'holds' if v else 'does not hold'}")
            if v:
                out.extend(self.expand(path, text, [(blank(r), r, a_at) for r in inner], label))
        return out

    def cfg_holds(self, path, text, attrs, label):
        """Whether every `cfg` among `attrs` holds: True, False, or None
        after recording an error (a predicate it cannot evaluate)."""
        result = True
        for _, raw, a_at in attrs:
            name, args = attr_parts(raw)
            if name != "cfg":
                continue
            where = self.where(path, text, a_at)
            if args is None:
                self.errors.append(f"{where}: {label}: {raw}: malformed cfg")
                result = None
                continue
            v, why = cfg_value(args, self.features)
            if v is None:
                self.errors.append(f"{where}: {label}: {raw}: {why}")
                result = None
                continue
            if re.sub(r"\s+", "", args) not in ("test", "not(test)"):
                self.notes.append(f"{where}: {label}: {raw}: {'holds' if v else 'does not hold'}")
            if not v and result is True:
                result = False
        return result

    def function(self, path, text, m, attrs, modpath):
        label = f"fn {m.group(2)}"
        attrs = self.expand(path, text, attrs, label)
        names = [attr_parts(a)[0] for a, _, _ in attrs]
        if any(n in TEST_ATTRS for n in names):
            if self.cfg_holds(path, text, attrs, label) is True:
                self.tests.append("::".join(modpath + [m.group(2)]))
        elif any(self.test_attr(a) for a, _, _ in attrs):
            self.errors.append(
                f"{self.where(path, text, m.start())}: fn {m.group(2)}: unknown test attribute"
            )

    def module(self, path, text, code, m, attrs, modpath, child_dir, inline):
        name = m.group(2)
        label = f"mod {name}"
        attrs = self.expand(path, text, attrs, label)
        if self.cfg_holds(path, text, attrs, label) is not True:
            # Not compiled in this build (or an error is recorded): skip
            # `mod x;` or the whole `mod x { ... }` block.
            return self.item_end(code, m.end() - 1, len(code))
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


def has_features(src):
    """Whether the crate's Cargo.toml (beside `src`) has a [features] table."""
    toml = src.parent / "Cargo.toml"
    if not toml.is_file():
        return False
    return any(line.strip() == "[features]" for line in toml.read_text(encoding="utf-8").splitlines())


def run(src):
    """List `src`'s bin tests: (sorted tests, notes, errors)."""
    lister = Lister(src, has_features(src))
    try:
        lister.file(src / "main.rs", [], src)
    except SystemExit as e:
        lister.errors.append(str(e))
    if len(lister.tests) != len(set(lister.tests)):
        lister.errors.append("a test name is listed twice")
    return sorted(lister.tests), lister.notes, lister.errors


# --self-test cases: (name, files, Cargo.toml text, expected tests or None,
# expected error substrings). None: the run must fail with every substring.
SELF_TESTS = [
    (
        "cfg forms on mods and test fns",
        {
            "main.rs": (
                "#[cfg(test)]\nmod a;\n"
                "#[cfg(not(test))]\nmod gone;\n"
                "#[cfg(any(not(test), test))]\nmod b;\n"
                "#[cfg(any(not(test), feature = \"x\"))]\nmod gone2;\n"
                "#[cfg(all(test, unix, debug_assertions))]\nmod c;\n"
                "#[cfg(any(test, loom))]\nmod d;\n"
                "#[cfg(all(not(test), loom))]\nmod gone3;\n"
                "#[cfg_attr(not(test), allow(dead_code))]\n#[cfg(test)]\nmod e;\n"
                "#[cfg(target_os = \"macos\")]\nmod gone4;\n"
                "#[cfg(windows)]\nmod gone5 { #[test] fn w() {} }\n"
                "#[cfg(test)]\nmod inl {\n    #![cfg(any())]\n    #[test]\n    fn never() {}\n}\n"
                "#[cfg(loom)]\nfn helper_not_a_test() {}\n"
                "#[cfg_attr(loom, inline)]\nfn helper2() {}\n"
            ),
            "a.rs": (
                "// #[cfg(foo)] #[test] fn in_comment() {}\n"
                "const S: &str = \"#[cfg(foo)] #[test] fn in_string() {}\";\n"
                "#[test]\nfn plain() {}\n"
                "#[cfg(not(test))]\n#[test]\nfn fn_level_not_test() {}\n"
                "#[cfg(any())]\n#[test]\nfn fn_level_any_empty() {}\n"
                "#[cfg(all())]\n#[test]\nfn fn_level_all_empty() {}\n"
                "#[cfg(debug_assertions)]\n#[tokio::test]\nasync fn debug_only() {}\n"
                "#[cfg(target_endian = \"little\")]\n#[test]\nfn little() {}\n"
                "#[cfg(target_endian = \"big\")]\n#[test]\nfn big() {}\n"
                "#[cfg_attr(test, test)]\nfn via_cfg_attr() {}\n"
                "#[cfg_attr(not(test), test)]\nfn not_via_cfg_attr() {}\n"
                "#[cfg_attr(not(test), ignore)]\n#[test]\nfn ignored_outside_tests() {}\n"
                "#[cfg_attr(test, cfg(not(test)))]\n#[test]\nfn cfg_attr_cfg_false() {}\n"
                "#[test]\n#[ignore]\nfn ignored() {}\n"
            ),
            "gone.rs": "#[test]\nfn x() {}\n",
            "gone2.rs": "#[test]\nfn x() {}\n",
            "gone3.rs": "#[test]\nfn x() {}\n",
            "gone4.rs": "#[test]\nfn x() {}\n",
            "b.rs": "#![cfg(test)]\n#[test]\nfn x() {}\n",
            "c.rs": "#![cfg(any())]\n//! dropped whole\n#[test]\nfn x() {}\n",
            "d.rs": "#![cfg(all(test, not(windows)))]\n#[test]\nfn x() {}\n",
            "e.rs": "#[test]\nfn x() {}\n",
        },
        "[package]\nname = \"t\"\n",
        [
            "a::debug_only",
            "a::fn_level_all_empty",
            "a::ignored",
            "a::ignored_outside_tests",
            "a::little",
            "a::plain",
            "a::via_cfg_attr",
            "b::x",
            "d::x",
            "e::x",
        ],
        [],
    ),
    (
        "unknown cfg on a test fn",
        {"main.rs": "#[cfg(loom)]\n#[test]\nfn t() {}\n"},
        "",
        None,
        ["main.rs:1: fn t: #[cfg(loom)]: cannot evaluate loom"],
    ),
    (
        "unknown inside all() on a test fn",
        {"main.rs": "mod a;\n", "a.rs": "\n#[cfg(all(test, loom))]\n#[test]\nfn t() {}\n"},
        "",
        None,
        ["a.rs:2: fn t: #[cfg(all(test, loom))]: cannot evaluate loom"],
    ),
    (
        "unknown cfg on a mod",
        {"main.rs": "#[cfg(sanitize = \"thread\")]\nmod a;\n", "a.rs": ""},
        "",
        None,
        ["main.rs:1: mod a: #[cfg(sanitize = \"thread\")]: cannot evaluate sanitize = \"thread\""],
    ),
    (
        "unknown inner cfg of a file",
        {"main.rs": "mod a;\n", "a.rs": "#![cfg(loom)]\n#[test]\nfn t() {}\n"},
        "",
        None,
        ["a.rs:1: module a: #![cfg(loom)]: cannot evaluate loom"],
    ),
    (
        "feature cfg when the crate has features",
        {"main.rs": "#[cfg(feature = \"bench\")]\n#[test]\nfn t() {}\n"},
        "[package]\nname = \"t\"\n\n[features]\nbench = []\n",
        None,
        ["main.rs:1: fn t: #[cfg(feature = \"bench\")]: cannot evaluate feature = \"bench\""],
    ),
    (
        "unknown cfg_attr that could make a test",
        {"main.rs": "#[cfg_attr(loom, test)]\nfn t() {}\n"},
        "",
        None,
        ["main.rs:1: fn t: #[cfg_attr(loom, test)]: cannot evaluate loom"],
    ),
    (
        "malformed cfg",
        {"main.rs": "#[cfg(not(test, unix))]\n#[test]\nfn t() {}\n"},
        "",
        None,
        ["main.rs:1: fn t: #[cfg(not(test, unix))]: malformed predicate"],
    ),
]


def self_test():
    import tempfile

    failed = 0
    for name, files, toml, want, want_errors in SELF_TESTS:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            src = root / "src"
            for rel, body in files.items():
                (src / rel).parent.mkdir(parents=True, exist_ok=True)
                (src / rel).write_text(body, encoding="utf-8")
            if toml:
                (root / "Cargo.toml").write_text(toml, encoding="utf-8")
            tests, _, errors = run(src.resolve())
        if want is not None:
            ok = not errors and tests == sorted(want)
            got = f"tests {tests}, errors {errors}"
        else:
            ok = bool(errors) and all(any(w in e for e in errors) for w in want_errors)
            got = f"errors {errors}"
        print(f"{'ok  ' if ok else 'FAIL'} {name}" + ("" if ok else f": {got}"))
        failed += not ok
    print(f"{len(SELF_TESTS) - failed}/{len(SELF_TESTS)} self-test cases pass")
    sys.exit(1 if failed else 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument(
        "--src",
        help="the crate's src/ directory (default: crates/stat-tracker/src of this checkout)",
    )
    ap.add_argument("--self-test", action="store_true", help="run the synthetic cases and exit")
    args = ap.parse_args()
    if args.self_test:
        self_test()
    src = pathlib.Path(args.src) if args.src else pathlib.Path(__file__).resolve().parent.parent / "src"
    tests, notes, errors = run(src.resolve())
    for note in notes:
        print(f"note: {note}", file=sys.stderr)
    if errors:
        for e in errors:
            print(f"error: {e}", file=sys.stderr)
        sys.exit(1)
    for t in tests:
        print(f"{t}: test")


if __name__ == "__main__":
    main()
