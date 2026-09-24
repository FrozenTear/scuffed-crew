#!/usr/bin/env bash
# Download a prebuilt Linux x86_64 release and run its in-tarball installer.
#
# Fresh install (stable entrypoint — older GUIs curl this too):
#   curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
#
# That `main` copy only resolves the release tag, then re-execs bootstrap.sh
# from that tag. A later change on main does not become the installer for a
# pinned tag. install.sh is the copy inside the release tarball.
#
# Pin the script URL itself (what the GUI copies when it knows the version).
# The tag must be assigned on bash, not curl: `VAR=x curl | bash` does not
# export VAR into bash.
#   TAG=stat-tracker-v0.4.14
#   curl -fsSL "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/${TAG}/crates/stat-tracker/dist/bootstrap.sh" \
#     | STAT_TRACKER_TAG="$TAG" bash
#
# Env:
#   STAT_TRACKER_REPO   default FrozenTear/scuffed-crew
#   STAT_TRACKER_TAG    optional release tag (default: latest with matching asset)
#   STAT_TRACKER_CHANNEL stable|prerelease — which release channel to install
#                       when no tag is pinned (default: stable). If unset and a
#                       prerelease newer than the latest stable exists, an
#                       interactive terminal is prompted (default: stable);
#                       without a tty the stable release is used silently.
#   STAT_TRACKER_PREFIX install prefix passed as PREFIX to install.sh (default ~/.local)
#   STAT_TRACKER_DIR    extract directory (default: mktemp -d, removed after install)
#   STAT_TRACKER_BOOTSTRAP_PINNED
#                       set to 1 once this process is already the tag's script
#                       (skips the re-exec). The GUI sets this after it has
#                       downloaded that tag's bootstrap itself.
#   STAT_TRACKER_MINISIGN_PUB
#                       optional path to a minisign .pub file, or the file
#                       contents. Empty default: no key is shipped in git.
#   SKIP_INTEGRATION    non-empty ⇒ install binaries/libs only (no desktop entry
#                       or systemd unit); use for throwaway-PREFIX smoke tests
#   STAT_TRACKER_BOOTSTRAP_FETCH_CMD
#                       test hook (URL dest). Unsupported for real installs.
#   STAT_TRACKER_BOOTSTRAP_LIB_ONLY
#                       source this file to load functions and return.
set -euo pipefail

REPO="${STAT_TRACKER_REPO:-FrozenTear/scuffed-crew}"
ASSET_NAME="scuffed-stat-tracker-linux-x86_64.tar.gz"
PREFIX="${STAT_TRACKER_PREFIX:-$HOME/.local}"
API="https://api.github.com/repos/${REPO}/releases"
GH_HEADERS=(-H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")
if [[ -n "${GITHUB_TOKEN:-}${GH_TOKEN:-}" ]]; then
    GH_HEADERS+=(-H "Authorization: Bearer ${GITHUB_TOKEN:-$GH_TOKEN}")
fi

# Minisign public key (full .pub file). Empty until the maintainer generates a
# key offline and embeds the public half here. Never commit a secret key.
STAT_TRACKER_MINISIGN_PUB_DEFAULT=""

RED='\033[0;31m'
YLW='\033[1;33m'
GRN='\033[0;32m'
NC='\033[0m'
# All logging goes to stderr: resolve_release's stdout is machine-parsed
# (mapfile below), so a single echo to stdout shifts tag/url/sha by one line
# and the tag ends up passed to curl as a hostname.
info()  { echo -e "${GRN}[bootstrap]${NC} $*" >&2; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*" >&2; }
error() { echo -e "${RED}[error ]${NC} $*" >&2; }

need() {
    command -v "$1" &>/dev/null || { error "need '$1' on PATH"; exit 1; }
}

bootstrap_url_for_ref() {
    local ref="$1"
    local repo="${STAT_TRACKER_REPO:-FrozenTear/scuffed-crew}"
    printf 'https://raw.githubusercontent.com/%s/%s/crates/stat-tracker/dist/bootstrap.sh\n' "$repo" "$ref"
}

assert_safe_tag() {
    local tag="$1"
    if [[ -z "$tag" || ! "$tag" =~ ^[A-Za-z0-9._-]+$ ]]; then
        error "refusing unsafe release tag '${tag}'"
        return 1
    fi
}

fetch_to() {
    local url="$1" dest="$2"
    if [[ -n "${STAT_TRACKER_BOOTSTRAP_FETCH_CMD:-}" ]]; then
        "$STAT_TRACKER_BOOTSTRAP_FETCH_CMD" "$url" "$dest"
        return
    fi
    curl -fsSL -o "$dest" "$url"
}

# Resolve asset download URL plus optional sha256 and minisign companions.
resolve_release() {
    local json url tag
    if [[ -n "${STAT_TRACKER_TAG:-}" ]]; then
        info "Fetching release ${STAT_TRACKER_TAG}…"
        json="$(curl -fsSL "${GH_HEADERS[@]}" "${API}/tags/${STAT_TRACKER_TAG}")"
    else
        info "Fetching latest GitHub releases for ${REPO}…"
        json="$(curl -fsSL "${GH_HEADERS[@]}" "${API}?per_page=20")"
        # Two candidates that ship our asset: the newest stable, and the newest
        # prerelease that is newer than it (list is newest-first; drafts skipped).
        local stable_tag pre_tag chosen channel
        { read -r stable_tag; read -r pre_tag; } < <(python3 -c '
import json,sys
releases=json.load(sys.stdin)
name=sys.argv[1]
stable=pre=""
for r in releases:
    if r.get("draft"):
        continue
    if not any(a.get("name")==name for a in r.get("assets") or []):
        continue
    if r.get("prerelease"):
        if not pre:
            pre=r.get("tag_name","")
    else:
        stable=r.get("tag_name","")
        break
if not (stable or pre):
    sys.stderr.write("no published release with asset %s\n" % name)
    sys.exit(1)
print(stable)
print(pre)
' "$ASSET_NAME" <<<"$json")
        if [[ -z "$stable_tag" && -z "$pre_tag" ]]; then
            exit 1
        fi

        channel="${STAT_TRACKER_CHANNEL:-}"
        # Prompt only with a real controlling terminal (curl|bash keeps stdin
        # busy, so talk to /dev/tty; opening it is the only reliable tty test).
        if [[ -z "$channel" && -n "$pre_tag" && -n "$stable_tag" ]] \
            && { exec 3<>/dev/tty; } 2>/dev/null; then
            local reply=""
            printf '%b' "${YLW}[bootstrap]${NC} Prerelease ${pre_tag} is available (stable: ${stable_tag}). Install [s]table or [p]rerelease? [S/p] " >&3
            IFS= read -r reply <&3 || reply=""
            exec 3>&-
            case "$reply" in
                [pP]*) channel=prerelease ;;
                *)     channel=stable ;;
            esac
        fi

        case "$channel" in
            prerelease)
                if [[ -n "$pre_tag" ]]; then
                    chosen="$pre_tag"
                else
                    info "No prerelease newer than stable; using ${stable_tag}."
                    chosen="$stable_tag"
                fi
                ;;
            stable|"")
                if [[ -n "$stable_tag" ]]; then
                    chosen="$stable_tag"
                elif [[ -z "${STAT_TRACKER_CHANNEL:-}" ]]; then
                    warn "No stable release ships ${ASSET_NAME}; falling back to prerelease ${pre_tag}."
                    chosen="$pre_tag"
                else
                    error "no stable release with asset ${ASSET_NAME} (prerelease ${pre_tag} exists — set STAT_TRACKER_CHANNEL=prerelease or pin STAT_TRACKER_TAG)"
                    exit 1
                fi
                ;;
            *)
                error "invalid STAT_TRACKER_CHANNEL='${STAT_TRACKER_CHANNEL}' (expected 'stable' or 'prerelease')"
                exit 1
                ;;
        esac

        # Reduce the list to the chosen release object for the extractor below.
        json="$(python3 -c '
import json,sys
releases=json.load(sys.stdin)
tag=sys.argv[1]
for r in releases:
    if r.get("tag_name")==tag:
        json.dump(r, sys.stdout)
        sys.exit(0)
sys.stderr.write("release %s vanished from listing\n" % tag)
sys.exit(1)
' "$chosen" <<<"$json")"
    fi

    python3 -c '
import json,sys
r=json.load(sys.stdin)
name=sys.argv[1]
url=sha=sig=None
for a in r.get("assets") or []:
    if a.get("name")==name:
        url=a["browser_download_url"]
    if a.get("name")==name+".sha256":
        sha=a["browser_download_url"]
    if a.get("name")==name+".minisig":
        sig=a["browser_download_url"]
if not url:
    sys.stderr.write("release %s has no asset %s\n" % (r.get("tag_name"), name))
    sys.exit(1)
print(r.get("tag_name",""))
print(url)
print(sha or "")
print(sig or "")
' "$ASSET_NAME" <<<"$json"
}

resolve_handoff_tag() {
    if [[ -n "${STAT_TRACKER_TAG:-}" ]]; then
        printf '%s\n' "$STAT_TRACKER_TAG"
        return 0
    fi
    local lines tag
    mapfile -t lines < <(resolve_release)
    tag="${lines[0]:-}"
    printf '%s\n' "$tag"
}

# Older GUIs always download whatever is on main. Re-exec the tag's script so
# the installer body (and the tarball it unpacks) comes from the release.
maybe_reexec_tag_bootstrap() {
    if [[ -n "${STAT_TRACKER_BOOTSTRAP_PINNED:-}" ]]; then
        return 0
    fi
    local tag url dest
    tag="$(resolve_handoff_tag)"
    assert_safe_tag "$tag" || exit 1
    url="$(bootstrap_url_for_ref "$tag")"
    dest="$(mktemp -t scuffed-bootstrap-XXXXXX.sh)"
    if ! fetch_to "$url" "$dest"; then
        rm -f "$dest"
        error "could not download bootstrap.sh from ${url}"
        error "Refusing to run this copy's installer for ${tag}."
        exit 1
    fi
    if [[ ! -s "$dest" ]] || ! head -n 1 "$dest" | grep -q '^#!'; then
        rm -f "$dest"
        error "bootstrap.sh from ${tag} is not a shell script"
        exit 1
    fi
    chmod +x "$dest"
    export STAT_TRACKER_BOOTSTRAP_PINNED=1
    export STAT_TRACKER_TAG="$tag"
    exec bash "$dest"
}

configured_minisign_pub() {
    printf '%s' "${STAT_TRACKER_MINISIGN_PUB:-$STAT_TRACKER_MINISIGN_PUB_DEFAULT}"
}

warn_signature_fallback() {
    warn "Signature check skipped: no published minisign public key. Falling back to sha256 from the same GitHub release (not an independent trust root)."
}

verify_release_signature() {
    local asset="$1"
    local pub sig_url pubfile sigfile
    pub="$(configured_minisign_pub)"
    sig_url="${SIG_URL:-}"
    # No key configured: sha256 is the only check. A configured key must not
    # fall back when the .minisig asset is missing — that would let a stripped
    # signature downgrade the install to a same-origin checksum.
    if [[ -z "${pub//[[:space:]]/}" ]]; then
        warn_signature_fallback
        return 0
    fi
    if [[ -z "$sig_url" ]]; then
        error "A minisign public key is configured, but this release has no .minisig asset. Refusing to install without a signature."
        exit 1
    fi
    if ! command -v minisign >/dev/null 2>&1; then
        error "minisign public key is configured and a .minisig asset is present, but minisign is not installed. Refusing to install without checking the signature."
        exit 1
    fi
    pubfile="$(mktemp)"
    sigfile="${asset}.minisig"
    if [[ -f "$pub" && "$pub" != *$'\n'* ]]; then
        cp -f "$pub" "$pubfile"
    else
        printf '%s\n' "$pub" > "$pubfile"
    fi
    info "Verifying minisign signature…"
    if ! curl -fsSL -o "$sigfile" "$sig_url"; then
        rm -f "$pubfile"
        error "could not download minisign signature"
        exit 1
    fi
    if ! minisign -V -p "$pubfile" -x "$sigfile" -m "$asset"; then
        rm -f "$pubfile"
        error "minisign signature verification failed for ${asset}"
        exit 1
    fi
    rm -f "$pubfile"
    info "minisign ok"
}

# Reject absolute paths and '..', and do not follow a symlink out of dest.
safe_extract() {
    local archive="$1" dest="$2"
    python3 - "$archive" "$dest" <<'PY'
import os
import shutil
import sys
import tarfile

class Unsafe(Exception):
    pass

def reject(msg):
    raise Unsafe(msg)

def clean_parts(name):
    if name is None:
        return None
    raw = name.replace("\\", "/")
    if raw.startswith("/") or raw.startswith("~"):
        return None
    if len(raw) >= 2 and raw[1] == ":":
        return None
    parts = []
    for part in raw.split("/"):
        if part in ("", "."):
            continue
        if part == ".." or "\x00" in part:
            return None
        parts.append(part)
    if not parts:
        return None
    return parts

def symlink_stays_inside(member_parts, linkname):
    raw = (linkname or "").replace("\\", "/")
    if raw.startswith("/") or raw.startswith("~"):
        return False
    if len(raw) >= 2 and raw[1] == ":":
        return False
    if "\x00" in raw:
        return False
    base = list(member_parts[:-1])
    for part in raw.split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            if not base:
                return False
            base.pop()
            continue
        base.append(part)
    return True

def within_dest(dest, path):
    try:
        real = os.path.realpath(path)
    except OSError:
        return False
    return real == dest or real.startswith(dest + os.sep)

def ensure_parent(dest, parts):
    cur = dest
    for part in parts[:-1]:
        nxt = os.path.join(cur, part)
        if os.path.islink(nxt):
            reject("symlink in extract path: %s" % nxt)
        if os.path.isdir(nxt):
            cur = nxt
            continue
        if os.path.exists(nxt):
            reject("extract path is not a directory: %s" % nxt)
        os.mkdir(nxt, 0o755)
        cur = nxt
    if not within_dest(dest, cur):
        reject("parent escapes extract dir: %s" % cur)
    return cur

def extract(archive, dest):
    dest = os.path.realpath(dest)
    os.makedirs(dest, exist_ok=True)
    if not os.path.isdir(dest) or os.path.islink(dest):
        reject("extract dir is not a real directory")
    with tarfile.open(archive, "r:gz") as tar:
        checked = []
        for m in tar.getmembers():
            parts = clean_parts(m.name)
            if parts is None:
                reject("unsafe path %r" % (m.name,))
            if m.issym():
                if not symlink_stays_inside(parts, m.linkname):
                    reject("symlink escapes extract dir: %s -> %s" % (m.name, m.linkname))
            elif m.islnk():
                if clean_parts(m.linkname) is None:
                    reject("hardlink escapes extract dir: %s -> %s" % (m.name, m.linkname))
            elif not (m.isdir() or m.isreg()):
                reject("special file %r" % (m.name,))
            checked.append((m, parts))
        for m, parts in checked:
            parent = ensure_parent(dest, parts)
            leaf = os.path.join(parent, parts[-1])
            if os.path.islink(leaf):
                reject("refusing to follow symlink %s" % leaf)
            if m.isdir():
                os.makedirs(leaf, exist_ok=True)
                continue
            if os.path.lexists(leaf):
                reject("refusing to replace %s" % leaf)
            if m.issym():
                os.symlink(m.linkname, leaf)
                continue
            if m.islnk():
                target_parts = clean_parts(m.linkname)
                target = os.path.join(dest, *target_parts)
                if os.path.islink(target) or not os.path.isfile(target):
                    reject("hardlink target missing: %s" % m.linkname)
                if not within_dest(dest, target):
                    reject("hardlink target escapes: %s" % m.linkname)
                os.link(target, leaf)
                continue
            flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
            if hasattr(os, "O_NOFOLLOW"):
                flags |= os.O_NOFOLLOW
            fd = os.open(leaf, flags, 0o644)
            try:
                src = tar.extractfile(m)
                if src is None:
                    reject("no file body for %s" % m.name)
                with os.fdopen(fd, "wb") as out:
                    fd = None
                    shutil.copyfileobj(src, out)
            finally:
                if fd is not None:
                    os.close(fd)
            os.chmod(leaf, m.mode & 0o777)

def main():
    if len(sys.argv) != 3:
        sys.stderr.write("usage: safe_extract ARCHIVE DEST\n")
        sys.exit(2)
    try:
        extract(sys.argv[1], sys.argv[2])
    except Unsafe as exc:
        sys.stderr.write("refusing to extract: %s\n" % exc)
        sys.exit(1)

if __name__ == "__main__":
    main()
PY
}

if [[ "${STAT_TRACKER_BOOTSTRAP_LIB_ONLY:-}" == 1 ]]; then
    return 0 2>/dev/null || exit 0
fi

CHANNEL="${STAT_TRACKER_CHANNEL:-}"
case "$CHANNEL" in
    ""|stable|prerelease) ;;
    *)
        error "invalid STAT_TRACKER_CHANNEL='${CHANNEL}' (expected 'stable' or 'prerelease')"
        exit 1
        ;;
esac

need curl
need tar
need mktemp
need python3

case "$(uname -s)" in
    Linux) ;;
    *) error "bootstrap is Linux-only"; exit 1 ;;
esac

arch="$(uname -m)"
if [[ "$arch" != "x86_64" && "$arch" != "amd64" ]]; then
    error "prebuilt asset is x86_64 only (this machine: $arch)"
    exit 1
fi

maybe_reexec_tag_bootstrap

mapfile -t _rel < <(resolve_release)
TAG="${_rel[0]}"
URL="${_rel[1]}"
SHA_URL="${_rel[2]:-}"
SIG_URL="${_rel[3]:-}"

info "Using ${TAG}: ${URL}"

WORKDIR="${STAT_TRACKER_DIR:-$(mktemp -d -t scuffed-stat-tracker-XXXXXX)}"
CLEANUP_WORKDIR=0
if [[ -z "${STAT_TRACKER_DIR:-}" ]]; then
    CLEANUP_WORKDIR=1
fi
trap '[[ $CLEANUP_WORKDIR -eq 1 ]] && rm -rf "$WORKDIR"' EXIT

mkdir -p "$WORKDIR"
cd "$WORKDIR"
info "Downloading ${ASSET_NAME}…"
curl -fL --progress-bar -o "$ASSET_NAME" "$URL"

if [[ -n "$SHA_URL" ]] && command -v sha256sum &>/dev/null; then
    info "Verifying sha256…"
    curl -fsSL -o "${ASSET_NAME}.sha256" "$SHA_URL"
    # File may be either "HASH  name" or just HASH.
    if ! sha256sum -c "${ASSET_NAME}.sha256" 2>/dev/null; then
        expected="$(awk 'NF{print $1; exit}' "${ASSET_NAME}.sha256")"
        actual="$(sha256sum "$ASSET_NAME" | awk '{print $1}')"
        if [[ "$expected" != "$actual" ]]; then
            error "sha256 mismatch (expected $expected, got $actual)"
            exit 1
        fi
    fi
    info "sha256 ok"
else
    warn "No .sha256 asset or sha256sum missing — skipping integrity check."
fi

verify_release_signature "$ASSET_NAME"

info "Extracting…"
safe_extract "$WORKDIR/$ASSET_NAME" "$WORKDIR"
# Tarball root is scuffed-stat-tracker-linux-x86_64/
STAGE="$(find "$WORKDIR" -maxdepth 1 -type d -name 'scuffed-stat-tracker-linux-x86_64*' | head -1)"
if [[ -z "$STAGE" || ! -f "$STAGE/install.sh" ]]; then
    error "tarball missing install.sh at package root"
    exit 1
fi

chmod +x "$STAGE/install.sh" "$STAGE"/bin/* 2>/dev/null || true
info "Running in-tarball installer (PREFIX=$PREFIX)…"
# Pass SKIP_INTEGRATION through so a throwaway-PREFIX smoke test can install
# binaries only without polluting the real $HOME (desktop entry + systemd unit).
PREFIX="$PREFIX" SKIP_INTEGRATION="${SKIP_INTEGRATION:-}" bash "$STAGE/install.sh"

info "Done. Launch with: stat-tracker-gui"
