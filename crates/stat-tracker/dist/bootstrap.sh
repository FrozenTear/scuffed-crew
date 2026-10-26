#!/usr/bin/env bash
# Download a prebuilt Linux x86_64 release and run its in-tarball installer.
#
# Fresh install (stable entrypoint, older GUIs curl this too):
#   curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
#
# That `main` copy only resolves the release tag, then re-execs bootstrap.sh
# from that tag. A later change on main does not become the installer for a
# pinned tag. install.sh is the copy inside the release tarball.
#
# Pin the script URL itself (what the GUI copies when it knows the version).
# The tag must be assigned on bash, not curl: `VAR=x curl | bash` does not
# export VAR into bash.
#   TAG=stat-tracker-v0.5.0
#   curl --proto '=https' -fsSL "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/${TAG}/crates/stat-tracker/dist/bootstrap.sh" \
#     | STAT_TRACKER_TAG="$TAG" bash
#
# Env:
#   STAT_TRACKER_REPO   default FrozenTear/scuffed-crew
#   STAT_TRACKER_TAG    optional release tag (default: latest with matching asset)
#   STAT_TRACKER_CHANNEL stable|prerelease: which release channel to install
#                       when no tag is pinned (default: stable). An alpha or rc
#                       is not offered unless this is prerelease. Pin
#                       STAT_TRACKER_TAG to install one release by tag.
#   STAT_TRACKER_PREFIX install prefix passed as PREFIX to install.sh (default ~/.local)
#   STAT_TRACKER_DIR    extract directory (default: mktemp -d, removed after install)
#   STAT_TRACKER_BOOTSTRAP_PINNED
#                       set to 1 once this process is already the tag's script
#                       (skips the re-exec). The GUI sets this after it has
#                       downloaded that tag's bootstrap itself.
#   STAT_TRACKER_MINISIGN_PUB
#                       optional path to a minisign .pub file, or the file
#                       contents. Empty default: no key is shipped in git.
#   STAT_TRACKER_BOOTSTRAP_STARTED
#                       set by this script (epoch seconds) for install.sh,
#                       which starts a stopped tracker again only when it
#                       stopped right before the install began.
#   STAT_TRACKER_KEEP_STOPPED
#                       1 from the app's updater when the tracker was not
#                       running as the update began: install.sh then never
#                       starts it. Passed through unchanged (the re-exec and
#                       the install.sh run inherit the environment).
#   SKIP_INTEGRATION    non-empty ⇒ install binaries/libs only (no desktop entry
#                       or systemd unit); use for throwaway-PREFIX smoke tests
#   STAT_TRACKER_BOOTSTRAP_FETCH_CMD
#                       test hook (URL dest). Unsupported for real installs.
#   STAT_TRACKER_BOOTSTRAP_LIB_ONLY
#                       source this file to load functions and return.
#
# Uninstall a user-level bootstrap install. install.sh writes
# install-manifest.txt. uninstall.sh removes exactly those paths.
#   bash bootstrap.sh --uninstall
#   bash bootstrap.sh --uninstall --purge
# A pacman/AUR or apt/dpkg owner of the GUI binary is left in place.
# The script prints `sudo pacman -R <pkg>` or `sudo apt remove <pkg>`.
# An AppImage is a script install, not a package.
set -euo pipefail
# The in-app updater runs this script on pipes. If the app exits mid-install
# (tray Quit), a write to its closed pipe must not kill the install, so
# SIGPIPE is ignored and the log helpers below ignore a failed write.
trap '' PIPE

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
# A failed write (stderr closed by an app that quit) is not fatal.
info()  { echo -e "${GRN}[bootstrap]${NC} $*" >&2 || true; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*" >&2 || true; }
error() { echo -e "${RED}[error ]${NC} $*" >&2 || true; }

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

# HTTPS only. --proto '=https' refuses http, file, and a redirect onto them.
# A stalled connection must fail instead of hanging: the in-app update stops
# the tracker before it runs this script, so a curl that never returns left
# the tracker stopped. Give up when the connect takes over 15 s, or when the
# transfer stays under 1 KiB/s for 60 s. A slow download that still moves is
# not cut off.
CURL_TIMEOUT_ARGS=(--connect-timeout 15 --speed-limit 1024 --speed-time 60)
curl_https() {
    curl --proto '=https' -fsSL "${CURL_TIMEOUT_ARGS[@]}" "$@"
}

fetch_to() {
    local url="$1" dest="$2"
    if [[ -n "${STAT_TRACKER_BOOTSTRAP_FETCH_CMD:-}" ]]; then
        "$STAT_TRACKER_BOOTSTRAP_FETCH_CMD" "$url" "$dest"
        return
    fi
    curl_https -o "$dest" "$url"
}

# Newest stable tag, then a newer prerelease tag, one per line (blank if none).
# A prerelease suffix counts even when GitHub left the flag off. Drafts and
# releases without $1's asset are skipped. JSON release list on stdin.
choose_release_tags() {
    python3 -c '
import json,sys
releases=json.load(sys.stdin)
name=sys.argv[1]

def version_of(tag):
    if tag.startswith("stat-tracker-v"):
        return tag[len("stat-tracker-v"):]
    if tag.startswith("v"):
        return tag[1:]
    return tag

def is_digits(piece):
    return bool(piece) and piece.isascii() and piece.isdigit()

def parse(ver):
    ver=ver.strip()
    if not ver or any(ch.isspace() for ch in ver):
        return None
    ver=ver.split("+",1)[0]
    pre_ids=[]
    if "-" in ver:
        core, pre = ver.split("-",1)
        if pre=="":
            return None
        for piece in pre.split("."):
            if piece=="" or any(ch.isspace() for ch in piece):
                return None
            if is_digits(piece):
                pre_ids.append((0, int(piece)))
            else:
                pre_ids.append((1, piece))
    else:
        core=ver
    parts=core.split(".")
    if len(parts)<2 or len(parts)>3:
        return None
    try:
        nums=[int(p) for p in parts]
    except ValueError:
        return None
    while len(nums)<3:
        nums.append(0)
    return (nums[0], nums[1], nums[2], tuple(pre_ids))

def newer(left, right):
    if left[:3]!=right[:3]:
        return left[:3]>right[:3]
    lp, rp = left[3], right[3]
    if not lp and rp:
        return True
    if lp and not rp:
        return False
    for x,y in zip(lp, rp):
        if x!=y:
            return x>y
    return len(lp)>len(rp)

def offered_pre(pre_ids):
    kind, first = pre_ids[0]
    if kind != 1:
        return False
    first = first.lower()
    for word in ("alpha", "rc"):
        if first.startswith(word):
            rest = first[len(word):]
            return rest == "" or (rest.isascii() and rest.isdigit())
    return False

stable=None
pre=None
for r in releases:
    if r.get("draft"):
        continue
    if not any(a.get("name")==name for a in r.get("assets") or []):
        continue
    tag=r.get("tag_name") or ""
    ver=parse(version_of(tag))
    if ver is None:
        continue
    flagged=bool(r.get("prerelease")) or bool(ver[3])
    # Only alpha and rc builds go out on the prerelease channel. A test
    # build (stat-tracker-v0.5.0-test.1) is never installed without a pin.
    if ver[3] and not offered_pre(ver[3]):
        continue
    if flagged:
        if pre is None or newer(ver, pre[0]):
            pre=(ver, tag)
    else:
        if stable is None or newer(ver, stable[0]):
            stable=(ver, tag)
if stable is None and pre is None:
    sys.stderr.write("no published release with asset %s\n" % name)
    sys.exit(1)
stable_tag=stable[1] if stable else ""
pre_tag=""
if pre is not None and (stable is None or newer(pre[0], stable[0])):
    pre_tag=pre[1]
print(stable_tag)
print(pre_tag)
' "$1"
}

# Print the tag to install. $3 empty means stable. Does not prompt.
pick_release_tag() {
    local stable_tag="$1" pre_tag="$2" channel="${3:-}"
    case "$channel" in
        prerelease)
            if [[ -n "$pre_tag" ]]; then
                printf '%s\n' "$pre_tag"
            else
                info "No prerelease newer than stable; using ${stable_tag}."
                printf '%s\n' "$stable_tag"
            fi
            ;;
        stable|"")
            if [[ -n "$stable_tag" ]]; then
                printf '%s\n' "$stable_tag"
            elif [[ -z "$channel" ]]; then
                warn "No stable release ships ${ASSET_NAME}; falling back to prerelease ${pre_tag}."
                printf '%s\n' "$pre_tag"
            else
                error "no stable release with asset ${ASSET_NAME} (prerelease ${pre_tag} exists, set STAT_TRACKER_CHANNEL=prerelease or pin STAT_TRACKER_TAG)"
                return 1
            fi
            ;;
        *)
            error "invalid STAT_TRACKER_CHANNEL='${channel}' (expected 'stable' or 'prerelease')"
            return 1
            ;;
    esac
}

# The release list is read page by page. Releases that do not ship the
# tracker (site-*) share this list, so the newest stable tracker release can
# sit far down it. /releases/latest is not used: a site-* release can take it.
RELEASE_PAGE_SIZE=100
RELEASE_MAX_PAGES=5

# One page of the release list (JSON array) on stdout.
fetch_release_page() {
    curl_https "${GH_HEADERS[@]}" "${API}?per_page=${RELEASE_PAGE_SIZE}&page=$1"
}

# Number of releases in the JSON array on stdin.
release_page_len() {
    python3 -c '
import json,sys
page=json.load(sys.stdin)
if not isinstance(page, list):
    msg=page.get("message") if isinstance(page, dict) else None
    sys.stderr.write("GitHub release list is not a list%s\n" % (": %s" % msg if msg else ""))
    sys.exit(1)
print(len(page))
'
}

# Concatenated JSON arrays on stdin, one merged array on stdout. A release
# seen twice (the list moved between two page reads) is kept once.
merge_release_pages() {
    python3 -c '
import json,sys
text=sys.stdin.read()
dec=json.JSONDecoder()
merged=[]
seen=set()
pos=0
while True:
    while pos < len(text) and text[pos].isspace():
        pos += 1
    if pos >= len(text):
        break
    page, pos = dec.raw_decode(text, pos)
    if not isinstance(page, list):
        sys.stderr.write("GitHub release list is not a list\n")
        sys.exit(1)
    for r in page:
        key = r.get("id") if isinstance(r, dict) else None
        if key is not None:
            if key in seen:
                continue
            seen.add(key)
        merged.append(r)
json.dump(merged, sys.stdout)
'
}

# The release list as one JSON array. Reads pages until one holds a stable
# release that ships the asset (as choose_release_tags sees it), a page
# comes back short, or RELEASE_MAX_PAGES pages were read. A failed page
# after the first ends the list there: the pages read are printed and the
# return is 3, so the caller knows the list was cut short.
fetch_release_listing() {
    local page=1 body count all="" stable_tag pre_tag cut=0
    while :; do
        if ! body="$(fetch_release_page "$page")" \
            || ! count="$(release_page_len <<<"$body")"; then
            # Page 1 is the whole answer when it fails. A later page (rate
            # limit, error) only extends the search: use what was read.
            if (( page == 1 )); then
                return 1
            fi
            if (( page == 2 )); then
                warn "Could not read page 2 of the release list; using page 1."
            else
                warn "Could not read page ${page} of the release list; using pages 1 to $((page - 1))."
            fi
            cut=1
            break
        fi
        all="$(printf '%s\n%s\n' "$all" "$body" | merge_release_pages)" || return 1
        if (( count < RELEASE_PAGE_SIZE || page >= RELEASE_MAX_PAGES )); then
            break
        fi
        stable_tag=""
        pre_tag=""
        { read -r stable_tag; read -r pre_tag; } \
            < <(choose_release_tags "$ASSET_NAME" <<<"$all" 2>/dev/null) || true
        if [[ -n "$stable_tag" ]]; then
            break
        fi
        page=$((page + 1))
    done
    printf '%s\n' "$all"
    if (( cut )); then
        return 3
    fi
}

# Resolve asset download URL plus optional sha256 and minisign companions.
resolve_release() {
    local json url tag
    if [[ -n "${STAT_TRACKER_TAG:-}" ]]; then
        info "Fetching release ${STAT_TRACKER_TAG}…"
        json="$(curl_https "${GH_HEADERS[@]}" "${API}/tags/${STAT_TRACKER_TAG}")"
    else
        info "Fetching latest GitHub releases for ${REPO}…"
        local listing_cut=0 listing_rc=0
        json="$(fetch_release_listing)" || listing_rc=$?
        case "$listing_rc" in
            0) ;;
            3) listing_cut=1 ;;
            *) exit 1 ;;
        esac
        # Newest stable that ships our asset, and a prerelease only when it
        # is newer than that stable. Drafts are skipped. No prompt.
        local stable_tag pre_tag chosen
        { read -r stable_tag; read -r pre_tag; } < <(choose_release_tags "$ASSET_NAME" <<<"$json")
        if [[ -z "$stable_tag" && -z "$pre_tag" ]]; then
            exit 1
        fi
        # A list cut short by an error may hide the stable release. Do not
        # fall back to a prerelease for that: only an asked-for prerelease
        # channel may use what was read.
        if (( listing_cut )) && [[ -z "$stable_tag" && "${STAT_TRACKER_CHANNEL:-}" != prerelease ]]; then
            error "GitHub's release list was cut short before a stable release was found. Try again later, or set STAT_TRACKER_CHANNEL=prerelease, or pin STAT_TRACKER_TAG."
            exit 1
        fi
        chosen="$(pick_release_tag "$stable_tag" "$pre_tag" "${STAT_TRACKER_CHANNEL:-}")"

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

error_no_release() {
    error "No release to install: could not read the release list from GitHub (offline or rate-limited), or no release matched (see above)."
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
    if [[ -z "$tag" ]]; then
        error_no_release
        exit 1
    fi
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
    # An ignored SIGPIPE carries over exec. The tag's script and the
    # installer it unpacks may predate the trap at the top of this file and
    # read lists with early-closing pipes; ignoring SIGPIPE there turns a
    # quiet stop into "write error: Broken pipe" lines. Hand over the
    # default, as before this file set the trap. A tag that has the trap
    # sets it again itself.
    trap - PIPE
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
    # fall back when the .minisig asset is missing, that would let a stripped
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
    if ! fetch_to "$sig_url" "$sigfile"; then
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

minisign_key_configured() {
    local pub
    pub="$(configured_minisign_pub)"
    [[ -n "${pub//[[:space:]]/}" ]]
}

# sha256 of the tarball. A missing .sha256 asset stops the install.
# A missing sha256sum is always a hard failure: the install never skips the
# integrity check.
verify_release_checksum() {
    local asset="$1"
    local sha_url="${SHA_URL:-}"
    local dir name sumfile
    dir="$(cd "$(dirname "$asset")" && pwd)"
    name="$(basename "$asset")"
    sumfile="${dir}/${name}.sha256"
    if [[ -z "$sha_url" ]]; then
        error "This release has no .sha256 asset. Refusing to install."
        exit 1
    fi
    if ! command -v sha256sum >/dev/null 2>&1; then
        error "sha256sum is not installed, so the download cannot be checked. Refusing to install. Install coreutils (it provides sha256sum) and run this again."
        exit 1
    fi
    info "Verifying sha256…"
    if ! fetch_to "$sha_url" "$sumfile"; then
        error "could not download .sha256"
        exit 1
    fi
    # File may be either "HASH  name" or just HASH.
    if ! (cd "$dir" && sha256sum -c "${name}.sha256") 2>/dev/null; then
        local expected actual
        expected="$(awk 'NF{print $1; exit}' "$sumfile")"
        actual="$(sha256sum "$asset" | awk '{print $1}')"
        if [[ "$expected" != "$actual" ]]; then
            error "sha256 mismatch (expected $expected, got $actual)"
            exit 1
        fi
    fi
    info "sha256 ok"
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

if [[ "${1:-}" == "--uninstall" ]]; then
    shift
    _here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    if [[ ! -f "$_here/uninstall.sh" || ! -f "$_here/install-paths.sh" ]]; then
        error "uninstall.sh and install-paths.sh must sit next to bootstrap.sh"
        exit 1
    fi
    export PREFIX="${STAT_TRACKER_PREFIX:-${PREFIX:-$HOME/.local}}"
    exec bash "$_here/uninstall.sh" "$@"
fi

CHANNEL="${STAT_TRACKER_CHANNEL:-}"
case "$CHANNEL" in
    ""|stable|prerelease) ;;
    *)
        error "invalid STAT_TRACKER_CHANNEL='${CHANNEL}' (expected 'stable' or 'prerelease')"
        exit 1
        ;;
esac

# When this install began, handed to install.sh: it starts the tracker
# again only when the tracker stopped right before this (the in-app
# updater's stop), never a stop from long before. A re-exec keeps the
# first copy's time.
if [[ ! "${STAT_TRACKER_BOOTSTRAP_STARTED:-}" =~ ^[1-9][0-9]*$ ]]; then
    STAT_TRACKER_BOOTSTRAP_STARTED="$(date +%s)"
fi
export STAT_TRACKER_BOOTSTRAP_STARTED

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
TAG="${_rel[0]:-}"
if [[ -z "$TAG" ]]; then
    error_no_release
    exit 1
fi
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
curl_https -o "$ASSET_NAME" "$URL"

verify_release_checksum "$WORKDIR/$ASSET_NAME"

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

# install.sh records every file it wrote. Fail if that manifest is missing
# so a later uninstall is not left guessing.
# shellcheck source=install-paths.sh
source "$STAGE/install-paths.sh"
_manifest="$(install_path_named "$HOME" "$PREFIX" manifest install-manifest.txt || true)"
if [[ -z "${_manifest}" || ! -f "$_manifest" ]]; then
    error "install.sh did not write the install manifest"
    exit 1
fi
info "Recorded install manifest → $_manifest"

info "Done. Launch with: stat-tracker-gui"
