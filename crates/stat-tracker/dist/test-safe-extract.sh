#!/usr/bin/env bash
# tar xzf follows a symlink already in the destination and will happily write
# outside the extract dir. safe_extract must refuse absolute paths, '..', and
# any symlink that leaves the extract dir — and must not create the escaped file.
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$DIST/bootstrap.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*" >&2; }

# shellcheck disable=SC1090
STAT_TRACKER_BOOTSTRAP_LIB_ONLY=1 source "$BOOTSTRAP"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── benign tree, including an in-tree symlink, matches tar czf ───────────────

src="$TMP/src"
mkdir -p "$src/scuffed-stat-tracker-linux-x86_64/bin"
printf '#!/bin/sh\necho install\n' > "$src/scuffed-stat-tracker-linux-x86_64/install.sh"
printf 'daemon\n' > "$src/scuffed-stat-tracker-linux-x86_64/bin/scuffed-stat-tracker"
chmod 755 "$src/scuffed-stat-tracker-linux-x86_64/install.sh" \
    "$src/scuffed-stat-tracker-linux-x86_64/bin/scuffed-stat-tracker"
ln -s ../install.sh "$src/scuffed-stat-tracker-linux-x86_64/bin/install-link"
tar -C "$src" -czf "$TMP/good.tar.gz" scuffed-stat-tracker-linux-x86_64
mkdir -p "$TMP/good-out"
safe_extract "$TMP/good.tar.gz" "$TMP/good-out"
[[ -f "$TMP/good-out/scuffed-stat-tracker-linux-x86_64/install.sh" ]] \
    || fail "benign extract dropped install.sh"
[[ -f "$TMP/good-out/scuffed-stat-tracker-linux-x86_64/bin/scuffed-stat-tracker" ]] \
    || fail "benign extract dropped the daemon"
[[ "$(readlink "$TMP/good-out/scuffed-stat-tracker-linux-x86_64/bin/install-link")" == "../install.sh" ]] \
    || fail "in-tree symlink was rewritten"
pass "benign archive extracts, in-tree symlink kept"

expect_refuse() {
    local name="$1" archive="$2" dest="$3" canary="$4"
    mkdir -p "$dest"
    set +e
    local err
    err="$(safe_extract "$archive" "$dest" 2>&1)"
    local code=$?
    set -e
    [[ "$code" -ne 0 ]] || fail "$name was extracted (exit 0)"
    [[ "$err" == *"refusing to extract"* ]] || fail "$name error was: $err"
    if [[ -e "$canary" ]]; then
        fail "$name wrote $canary"
    fi
    pass "$name refused"
}

python3 - "$TMP" << 'PY'
import io, os, sys, tarfile
tmp = sys.argv[1]

def write(path, members):
    with tarfile.open(path, "w:gz") as tar:
        for name, kind, payload in members:
            info = tarfile.TarInfo(name=name)
            if kind == "file":
                data = payload.encode()
                info.size = len(data)
                info.mode = 0o644
                info.type = tarfile.REGTYPE
                tar.addfile(info, io.BytesIO(data))
            elif kind == "symlink":
                info.type = tarfile.SYMTYPE
                info.linkname = payload
                tar.addfile(info)
            else:
                raise SystemExit("bad kind %s" % kind)

canary = os.path.join(tmp, "canary-abs")
write(os.path.join(tmp, "abs.tar.gz"), [
    ("file", "file", "ok\n"),
    (canary + "/pwn-abs", "file", "pwn\n"),
])
write(os.path.join(tmp, "dotdot.tar.gz"), [
    ("../canary-dot/pwn-dot", "file", "pwn\n"),
])
write(os.path.join(tmp, "link.tar.gz"), [
    ("escape", "symlink", os.path.join(tmp, "canary-link")),
    ("escape/pwn-link", "file", "pwn\n"),
])
write(os.path.join(tmp, "rel.tar.gz"), [
    ("out/pwned", "file", "pwn\n"),
])
PY

expect_refuse "absolute path" "$TMP/abs.tar.gz" "$TMP/abs-out" "$TMP/canary-abs/pwn-abs"
expect_refuse "dotdot path" "$TMP/dotdot.tar.gz" "$TMP/dot-out" "$TMP/canary-dot/pwn-dot"
expect_refuse "symlink escape" "$TMP/link.tar.gz" "$TMP/link-out" "$TMP/canary-link/pwn-link"

# Pre-existing symlink in the dest dir. Some GNU tar builds write through it.
# Others exit with "Invalid cross-device link" and write nothing. Either way
# our extractor must refuse and must not create the file. Replacing
# safe_extract with tar xzf fails this: tar's message is not "refusing to
# extract", and a tar that follows the link leaves the canary behind.
mkdir -p "$TMP/canary-follow" "$TMP/follow-out"
ln -s "$TMP/canary-follow" "$TMP/follow-out/out"
mkdir -p "$TMP/naive"
ln -s "$TMP/canary-follow" "$TMP/naive/out"
set +e
tar -C "$TMP/naive" -xzf "$TMP/rel.tar.gz" >/dev/null 2>&1
set -e
if [[ -f "$TMP/canary-follow/pwned" ]]; then
    rm -f "$TMP/canary-follow/pwned"
fi
expect_refuse "pre-existing symlink" "$TMP/rel.tar.gz" "$TMP/follow-out" "$TMP/canary-follow/pwned"

echo "All safe extract checks passed."
