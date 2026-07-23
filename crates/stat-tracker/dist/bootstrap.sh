#!/usr/bin/env bash
# Download the latest prebuilt Linux x86_64 release and run its in-tarball installer.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
#   # or pin a tag:
#   STAT_TRACKER_TAG=stat-tracker-v0.1.0 bash bootstrap.sh
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
#   SKIP_INTEGRATION    non-empty ⇒ install binaries/libs only (no desktop entry
#                       or systemd unit); use for throwaway-PREFIX smoke tests
set -euo pipefail

REPO="${STAT_TRACKER_REPO:-FrozenTear/scuffed-crew}"
ASSET_NAME="scuffed-stat-tracker-linux-x86_64.tar.gz"
PREFIX="${STAT_TRACKER_PREFIX:-$HOME/.local}"
API="https://api.github.com/repos/${REPO}/releases"
GH_HEADERS=(-H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")
if [[ -n "${GITHUB_TOKEN:-}${GH_TOKEN:-}" ]]; then
    GH_HEADERS+=(-H "Authorization: Bearer ${GITHUB_TOKEN:-$GH_TOKEN}")
fi

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

CHANNEL="${STAT_TRACKER_CHANNEL:-}"
case "$CHANNEL" in
    ""|stable|prerelease) ;;
    *)
        error "invalid STAT_TRACKER_CHANNEL='${CHANNEL}' (expected 'stable' or 'prerelease')"
        exit 1
        ;;
esac

need() {
    command -v "$1" &>/dev/null || { error "need '$1' on PATH"; exit 1; }
}
need curl
need tar
need mktemp

case "$(uname -s)" in
    Linux) ;;
    *) error "bootstrap is Linux-only"; exit 1 ;;
esac

arch="$(uname -m)"
if [[ "$arch" != "x86_64" && "$arch" != "amd64" ]]; then
    error "prebuilt asset is x86_64 only (this machine: $arch)"
    exit 1
fi

# Resolve asset download URL (and optional sha256 companion).
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

        channel="$CHANNEL"
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
                elif [[ -z "$CHANNEL" ]]; then
                    warn "No stable release ships ${ASSET_NAME}; falling back to prerelease ${pre_tag}."
                    chosen="$pre_tag"
                else
                    error "no stable release with asset ${ASSET_NAME} (prerelease ${pre_tag} exists — set STAT_TRACKER_CHANNEL=prerelease or pin STAT_TRACKER_TAG)"
                    exit 1
                fi
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
url=sha=None
for a in r.get("assets") or []:
    if a.get("name")==name:
        url=a["browser_download_url"]
    if a.get("name")==name+".sha256":
        sha=a["browser_download_url"]
if not url:
    sys.stderr.write("release %s has no asset %s\n" % (r.get("tag_name"), name))
    sys.exit(1)
print(r.get("tag_name",""))
print(url)
print(sha or "")
' "$ASSET_NAME" <<<"$json"
}

mapfile -t _rel < <(resolve_release)
TAG="${_rel[0]}"
URL="${_rel[1]}"
SHA_URL="${_rel[2]:-}"

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

info "Extracting…"
tar xzf "$ASSET_NAME"
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
