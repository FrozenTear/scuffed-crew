#!/usr/bin/env bash
# Scuffed Crew — publish recognizer packs into PACKS_DIR
#
# Takes the folder packgen wrote (one <name>.tar per pack plus manifest.json)
# and makes those packs downloadable: on the VPS through ssh, or in a volume
# or folder on this machine. Packs are game art. This script never writes
# them anywhere except the destination, and refuses a folder inside a git
# work tree. Full guide: docs/tracker-packs.md.
#
# Usage:
#   publish-packs.sh [--dry-run] [--update] [--first-publish]
#                    [--host user@host | --local-volume] <outdir>
#   publish-packs.sh --check <outdir>
#
#   --check           Check <outdir> (step 1 below) and stop. No destination
#                     is read or needed.
#   --host user@host  Destination is on that host, reached with ssh. The user
#                     must own the podman volumes (root on the VPS).
#   --local-volume    Destination is on this machine.
#   --dry-run         Read the destination, print every step, change nothing.
#   --update          Allow a pack that is already published to get new bytes
#                     under the same name. Its version must be a new one: a
#                     tracker downloads a pack again only when the version
#                     changed. Same name, other bytes and the same version is
#                     always refused.
#   --first-publish   The destination has no manifest.json yet and that is
#                     right. Without it such a folder is refused. With it the
#                     folder must hold nothing else.
#
# What it does, in order:
#   1. Checks <outdir>: every tar is listed, every listed tar is there, sha256
#      and size match, names are [a-z0-9-]+.tar, every tar is a plain tar, and
#      none holds an absolute path, "..", a link or a device node. Tars are
#      listed, never unpacked.
#   2. Reads what is published and merges: packs that <outdir> does not
#      mention stay listed, in their old order. New packs go last. This script
#      never takes a pack off the list.
#   3. Uploads each tar under a temp name, checks sha256 and size at the
#      destination, copies the old manifest (and, for an update, the old tar)
#      to the backups folder, then renames the new manifest over manifest.json
#      last. A failure before that rename leaves the old list and its tars as
#      they were. An update is two renames, tar then manifest, back to back.
#   4. Reads the manifest back and checks every listed pack at the destination.
#      After a failure it does the same and says what is live.
#
# Environment (destination):
#   PACKS_DEST        A plain directory to publish into, on this machine or on
#                     --host. With it set and no --host, --local-volume is
#                     implied. Unset: the podman volume is used.
#   PACKS_VOLUME      Full podman volume name. Unset: the one volume named
#                     packs-data or <project>_packs-data.
#   PACKS_BACKUP_DIR  Where old manifests and old tars are kept, on the
#                     destination machine (default: ~/scuffed-crew-backups
#                     there). Must be outside the destination.
#
# Environment (optional check through the site, after the publish):
#   PACKS_SITE_URL       Site origin, for example https://example.org
#   PACKS_TRACKER_TOKEN  A tracker token. Read from the environment only. It
#                        is never printed, not accepted as an argument, and
#                        not in any command line or trace.
#
# The destination needs bash 4.4+, GNU coreutils 8.24+ and flock (util-linux).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT="${SCRIPT_DIR}/lib/packs-dest.sh"

MAX_PACKS=32
MAX_MANIFEST_BYTES=65536
SAFE_ARG_RE='^[A-Za-z0-9_./:@=+-]+$'
# Pack names that a released tracker asks for by name. A list without one of
# them breaks that release. 0.5.0-alpha.2 to alpha.6 ask for heroes-v1.tar
# only. Add the other five here when alpha.7 is released.
RELEASED_TRACKERS_NEED=(heroes-v1.tar)

die() {
    echo "error: $*" >&2
    exit 1
}

usage_line() {
    echo "usage: publish-packs.sh [--dry-run] [--update] [--first-publish] [--host user@host | --local-volume] <outdir>"
    echo "       publish-packs.sh --check <outdir>"
}

# The comment block at the top of this file, on stdout.
help_text() {
    sed -n '2,/^$/{ /^$/d; s/^# \{0,1\}//; p; }' "${BASH_SOURCE[0]}"
}

usage() {
    usage_line >&2
    echo "  --help prints the full text." >&2
    exit 2
}

CHECK_ONLY=0
DRY_RUN=0
UPDATE=0
FIRST_PUBLISH=0
HOST=""
LOCAL_VOLUME=0
OUTDIR=""

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --check) CHECK_ONLY=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --update) UPDATE=1; shift ;;
        --first-publish) FIRST_PUBLISH=1; shift ;;
        --local-volume) LOCAL_VOLUME=1; shift ;;
        --host)
            [[ "$#" -ge 2 ]] || usage
            HOST="$2"
            shift 2
            ;;
        --replace)
            die "--replace is gone. To change a published pack, build it with a new version and publish it with --update."
            ;;
        --keep)
            die "--keep is gone. This script never takes a pack off the list; see \"Taking a pack off the list\" in docs/tracker-packs.md."
            ;;
        -h | --help)
            help_text
            exit 0
            ;;
        -*)
            echo "error: unknown option: $1" >&2
            usage
            ;;
        *)
            [[ -z "${OUTDIR}" ]] || usage
            OUTDIR="$1"
            shift
            ;;
    esac
done

[[ -n "${OUTDIR}" ]] || usage
if [[ -n "${HOST}" && "${LOCAL_VOLUME}" == "1" ]]; then
    die "--host and --local-volume are two different destinations; give one"
fi
if [[ -n "${HOST}" && ! "${HOST}" =~ ^[A-Za-z0-9_][A-Za-z0-9_.@:-]*$ ]]; then
    die "--host must look like user@host"
fi
if [[ "${CHECK_ONLY}" != "1" && -z "${HOST}" && "${LOCAL_VOLUME}" != "1" && -z "${PACKS_DEST:-}" ]]; then
    die "no destination: give --host user@host, --local-volume, or PACKS_DEST=/some/dir"
fi

if [[ "${BASH_VERSINFO[0]}" -lt 4 || ( "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -lt 4 ) ]]; then
    die "bash 4.4 or newer is required"
fi
for tool in python3 tar sha256sum base64 stat; do
    command -v "${tool}" >/dev/null 2>&1 || die "${tool} is required"
done
[[ -f "${AGENT}" ]] || die "missing ${AGENT}"

WORK="$(mktemp -d)"
chmod 700 "${WORK}"
trap 'rm -rf "${WORK}"' EXIT

# ------------------------------------------------------------ the token
#
# Read once, with tracing off, into a mode-600 curl config. After this block
# no line of the script holds the token, so `bash -x` has nothing to print,
# and no child process (ssh, tar, curl) inherits it in its environment.
HAVE_TOKEN=0
SITE_AUTH="${WORK}/site-auth"
{
    was_tracing=0
    case "$-" in
        *x*)
            was_tracing=1
            set +x
            ;;
    esac
    if [[ -n "${PACKS_TRACKER_TOKEN:-}" ]]; then
        if [[ ! "${PACKS_TRACKER_TOKEN}" =~ ^[A-Za-z0-9._~+/=-]+$ ]]; then
            HAVE_TOKEN=bad
        elif (
            umask 077
            printf 'header = "Authorization: Bearer %s"\n' "${PACKS_TRACKER_TOKEN}" > "${SITE_AUTH}"
        ); then
            HAVE_TOKEN=1
        else
            HAVE_TOKEN=unwritten
        fi
    fi
    unset PACKS_TRACKER_TOKEN
    [[ "${was_tracing}" == "0" ]] || set -x
} 2>/dev/null

SITE_BASE="${PACKS_SITE_URL:-}"
SITE_BASE="${SITE_BASE%/}"
if [[ "${HAVE_TOKEN}" != "0" && "${CHECK_ONLY}" != "1" ]]; then
    # Everything the check needs is looked at now, before anything is changed.
    [[ "${HAVE_TOKEN}" != "bad" ]] \
        || die "PACKS_TRACKER_TOKEN has characters a token cannot have. Nothing was done."
    [[ "${HAVE_TOKEN}" == "1" ]] \
        || die "could not write the curl config for the site check in ${WORK}. Nothing was done."
    [[ -n "${SITE_BASE}" ]] \
        || die "PACKS_TRACKER_TOKEN is set but PACKS_SITE_URL is not. Nothing was done."
    [[ "${SITE_BASE}" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ || "${SITE_BASE}" =~ ^http://(127\.0\.0\.1|localhost)(:[0-9]+)?$ ]] \
        || die "PACKS_SITE_URL must be an https:// origin (plain http only for 127.0.0.1). Nothing was done."
    command -v curl >/dev/null 2>&1 || die "curl is required for the check through the site"
fi

# ------------------------------------------------------------ signals
#
# Before the publish starts, a signal stops the script and nothing was
# changed. Once a destination half that writes is running it cannot be called
# back, so the signal is only noted: the script waits for the outcome, prints
# the Backup and Rollback lines, and then exits with the signal's status.
PHASE=check
GOT_SIGNAL=""
GOT_STATUS=0
on_signal() {
    if [[ "${PHASE}" == "check" ]]; then
        echo "error: stopped by SIG$1. The destination was not changed." >&2
        exit "$2"
    fi
    if [[ "${PHASE}" == "site" ]]; then
        echo "error: stopped by SIG$1 during the check through the site. The packs are published and verified at the destination." >&2
        exit "$2"
    fi
    GOT_SIGNAL="$1"
    GOT_STATUS="$2"
    echo "SIG$1 received. The destination half cannot be called back; waiting to see how it ended..." >&2
}
trap 'on_signal TERM 143' TERM
trap 'on_signal INT 130' INT
trap 'on_signal HUP 129' HUP

# ------------------------------------------------------------ small helpers

sha_of() {
    sha256sum -- "$1" | cut -d' ' -f1
}

size_of() {
    stat -c %s -- "$1"
}

# True when the folder, or any folder above it, holds a .git entry.
in_git_tree() {
    local dir
    dir="$(cd "$1" && pwd -P)" || return 1
    while :; do
        [[ -e "${dir}/.git" ]] && return 0
        [[ "${dir}" == "/" ]] && return 1
        dir="$(dirname "${dir}")"
    done
}

# A manifest as tab-separated lines: name, version, sha256, size.
# Mode "outdir" is the strict rule for packs being published. Mode "server"
# is what the site accepts (crates/site-server/src/packs.rs), for the list
# that is already published. On a bad file: the reason on stdout, exit 1.
PY_MANIFEST='
import json, re, sys
path, mode = sys.argv[1], sys.argv[2]
def fail(msg):
    print(msg)
    sys.exit(1)
raw = open(path, "rb").read()
if len(raw) > 65536:
    fail("it is larger than 64 KiB")
try:
    data = json.loads(raw.decode("utf-8"))
except (ValueError, UnicodeDecodeError):
    fail("it is not valid JSON")
if not isinstance(data, list):
    fail("it is not a JSON array")
if len(data) > 32:
    fail("it has more than 32 entries")
name_re = r"[a-z0-9][a-z0-9-]*\.tar" if mode == "outdir" else r"[A-Za-z0-9][A-Za-z0-9._-]*"
seen = set()
for entry in data:
    if not isinstance(entry, dict) or set(entry) != {"name", "version", "sha256", "size"}:
        fail("every entry needs exactly name, version, sha256 and size")
    name, version, sha, size = entry["name"], entry["version"], entry["sha256"], entry["size"]
    if not all(isinstance(v, str) for v in (name, version, sha)):
        fail("name, version and sha256 must be strings")
    if not re.fullmatch(name_re, name) or len(name) > 64 or ".." in name or name == "manifest.json":
        if mode == "outdir":
            fail("pack name %r is not [a-z0-9-]+.tar" % name)
        fail("pack name %r is not a single file name" % name)
    if not re.fullmatch(r"[A-Za-z0-9._-]{1,32}", version):
        fail("%s has a bad version" % name)
    if not re.fullmatch(r"[0-9a-f]{64}", sha):
        fail("%s has a bad sha256" % name)
    if isinstance(size, bool) or not isinstance(size, int) or size < 0:
        fail("%s has a bad size" % name)
    if mode == "outdir" and not 0 < size <= 32 * 1024 * 1024:
        fail("%s is empty or larger than 32 MiB, which the tracker refuses" % name)
    if name in seen:
        fail("%s is listed twice" % name)
    seen.add(name)
    print("\t".join((name, version, sha, str(size))))
'

# The manifest.json inside a pack, on stdin. The tracker refuses a pack whose
# inner name or version differs from the list entry.
PY_INNER='
import json, sys
name, version = sys.argv[1], sys.argv[2]
raw = sys.stdin.buffer.read()
if not raw:
    print("it has no top-level manifest.json")
    sys.exit(1)
try:
    inner = json.loads(raw.decode("utf-8"))
except (ValueError, UnicodeDecodeError):
    print("its inner manifest.json is not valid JSON")
    sys.exit(1)
if not isinstance(inner, dict) or inner.get("name") != name:
    print("its inner manifest.json does not name %s" % name)
    sys.exit(1)
if inner.get("version") != version:
    print("its inner manifest.json has version %r, the list says %r" % (inner.get("version"), version))
    sys.exit(1)
'

# Refuse a file that is not a plain tar, and a tar with an absolute path,
# "..", a link, or a device node. The archive is listed. It is never unpacked.
check_tar_members() {
    local tar="$1" name="$2" names verbose line magic
    # tar -t also reads gzip, xz and zstd without being asked to. The tracker
    # does not, so the header is looked at first: "ustar" at byte 257.
    magic="$(head -c 262 -- "${tar}" | tail -c 5 | tr -d '\000')"
    [[ "${magic}" == "ustar" ]] \
        || die "refused: ${name} is not a plain tar (it is compressed, or not a tar at all)"
    names="$(LC_ALL=C tar -P -tf "${tar}" 2>/dev/null)" \
        || die "refused: ${name} is not a readable tar"
    [[ -n "${names}" ]] || die "refused: ${name} is an empty tar"
    while IFS= read -r line; do
        if [[ "${line}" == /* ]]; then
            die "refused: ${name} holds an absolute path (${line})"
        fi
        case "/${line}/" in
            */../*) die "refused: ${name} holds a path with \"..\" (${line})" ;;
        esac
    done <<< "${names}"
    verbose="$(LC_ALL=C tar -P --numeric-owner -tvf "${tar}" 2>/dev/null)" \
        || die "refused: ${name} is not a readable tar"
    while IFS= read -r line; do
        case "${line:0:1}" in
            - | d) ;;
            l | h) die "refused: ${name} holds a link (${line##* [0-9][0-9]:[0-9][0-9] })" ;;
            c | b) die "refused: ${name} holds a device node (${line##* [0-9][0-9]:[0-9][0-9] })" ;;
            *) die "refused: ${name} holds a special entry (${line##* [0-9][0-9]:[0-9][0-9] })" ;;
        esac
    done <<< "${verbose}"
}

# Run the destination script: here, or on --host. Stdin carries the script
# and then, for `apply`, the upload stream.
run_dest() {
    local verb="$1"
    shift
    if [[ -n "${HOST}" ]]; then
        ssh -T -- "${HOST}" bash -s -- "${verb}" "${DEST_ARGS[@]}" "$@"
    else
        bash -s -- "${verb}" "${DEST_ARGS[@]}" "$@"
    fi
}

# Quote one path for a command line that is printed for the operator.
shown() {
    if [[ "$1" =~ ${SAFE_ARG_RE} ]]; then
        printf '%s' "$1"
    else
        printf '%q' "$1"
    fi
}

# A printed command for the destination machine. It is handed to `sh -c`, so
# it does not matter which login shell the far side has, or which shell the
# line is pasted into here (the `&&` in it is not every shell's syntax).
for_dest() {
    if [[ "$1" == *"'"* ]]; then
        # A path that needed quoting of its own. Printed as it is, for bash.
        printf '%s' "$1"
    elif [[ -n "${HOST}" ]]; then
        printf "ssh %s \"sh -c '%s'\"" "${HOST}" "$1"
    else
        printf "sh -c '%s'" "$1"
    fi
}

# Last value of one response header, lower-cased name, from a curl -D file.
header_value() {
    awk -v want="$2" '
        { sub(/\r$/, "") }
        tolower($0) ~ "^" want ":" { sub(/^[^:]*:[ \t]*/, ""); value = $0 }
        END { print value }
    ' "$1"
}

# ------------------------------------------------------------ 1. the outdir

[[ -d "${OUTDIR}" ]] || die "not a directory: ${OUTDIR}"
OUTDIR="$(cd "${OUTDIR}" && pwd -P)"
if in_git_tree "${OUTDIR}"; then
    die "refused: ${OUTDIR} is inside a git work tree. Packs are game art and must never sit in a repo."
fi
[[ -f "${OUTDIR}/manifest.json" && ! -L "${OUTDIR}/manifest.json" ]] \
    || die "refused: ${OUTDIR} has no manifest.json"

declare -a OUT_NAMES=()
declare -A OUT_VER=() OUT_SHA=() OUT_SIZE=()
if ! python3 -I -c "${PY_MANIFEST}" "${OUTDIR}/manifest.json" outdir > "${WORK}/out.tsv"; then
    die "refused: ${OUTDIR}/manifest.json: $(cat "${WORK}/out.tsv")"
fi
while IFS=$'\t' read -r name version sha size; do
    OUT_NAMES+=("${name}")
    OUT_VER["${name}"]="${version}"
    OUT_SHA["${name}"]="${sha}"
    OUT_SIZE["${name}"]="${size}"
done < "${WORK}/out.tsv"
[[ "${#OUT_NAMES[@]}" -gt 0 ]] || die "refused: ${OUTDIR}/manifest.json lists no packs"

for f in "${OUTDIR}"/*.tar; do
    [[ -e "${f}" || -L "${f}" ]] || continue
    name="${f##*/}"
    [[ -n "${OUT_SHA[${name}]:-}" ]] \
        || die "refused: ${name} is in ${OUTDIR} but manifest.json does not list it"
done

for name in "${OUT_NAMES[@]}"; do
    f="${OUTDIR}/${name}"
    [[ -f "${f}" && ! -L "${f}" ]] \
        || die "refused: manifest.json lists ${name} but that file is not in ${OUTDIR}"
    actual="$(size_of "${f}")"
    [[ "${actual}" == "${OUT_SIZE[${name}]}" ]] \
        || die "refused: ${name} is ${actual} bytes, manifest.json says ${OUT_SIZE[${name}]}"
    actual="$(sha_of "${f}")"
    [[ "${actual}" == "${OUT_SHA[${name}]}" ]] \
        || die "refused: ${name} has sha256 ${actual:0:12}..., manifest.json says ${OUT_SHA[${name}]:0:12}..."
    check_tar_members "${f}" "${name}"
    if ! reason="$(LC_ALL=C tar -xOf "${f}" manifest.json 2>/dev/null | head -c 1048576 \
        | python3 -I -c "${PY_INNER}" "${name}" "${OUT_VER[${name}]}")"; then
        die "refused: ${name}: ${reason:-it has no top-level manifest.json}"
    fi
done
echo "Checked ${#OUT_NAMES[@]} pack(s) in ${OUTDIR}"
if [[ "${CHECK_ONLY}" == "1" ]]; then
    for name in "${OUT_NAMES[@]}"; do
        printf '  %-22s version %-10s %10s bytes  %s\n' "${name}" "${OUT_VER[${name}]}" "${OUT_SIZE[${name}]}" "${OUT_SHA[${name}]:0:12}"
    done
    echo "--check: the outdir is good to publish. No destination was read."
    exit 0
fi

# ------------------------------------------------------------ 2. destination

declare -a DEST_ARGS=()
if [[ -n "${PACKS_DEST:-}" ]]; then
    DEST_ARGS+=(--dir "${PACKS_DEST}")
    WHERE="directory ${PACKS_DEST}"
else
    DEST_ARGS+=(--volume "${PACKS_VOLUME:-auto}")
    WHERE="podman volume ${PACKS_VOLUME:-*packs-data}"
fi
if [[ -n "${PACKS_BACKUP_DIR:-}" ]]; then
    DEST_ARGS+=(--backups "${PACKS_BACKUP_DIR}")
fi
if [[ -n "${HOST}" ]]; then
    command -v ssh >/dev/null 2>&1 || die "ssh is required for --host"
    # ssh hands these words to a shell on the host. Refuse what it would split.
    for arg in "${DEST_ARGS[@]}"; do
        [[ "${arg}" =~ ${SAFE_ARG_RE} ]] \
            || die "with --host, PACKS_DEST, PACKS_VOLUME and PACKS_BACKUP_DIR may only use letters, digits and _ . / : @ = + -"
    done
    WHERE="${WHERE} on ${HOST}"
elif [[ -n "${PACKS_DEST:-}" ]]; then
    [[ -d "${PACKS_DEST}" ]] || die "PACKS_DEST is not a directory: ${PACKS_DEST}"
    if in_git_tree "${PACKS_DEST}"; then
        die "refused: PACKS_DEST ${PACKS_DEST} is inside a git work tree"
    fi
    [[ "$(cd "${PACKS_DEST}" && pwd -P)" != "${OUTDIR}" ]] \
        || die "refused: PACKS_DEST is the outdir itself"
fi

# What one answer of the destination half says. parse_state fills these and
# writes the manifest it carries to the file named in $2 (or removes it).
# It returns 1 when the answer was cut short.
DEST_PATH=""
BACKUPS_PATH=""
STATE_REPORTED_SHA="absent"
STATE_MANIFEST_SHA="absent"
LOCK_STATE=""
JOURNAL_STATE=""
JOURNAL_RUN=""
SAVED_MANIFEST=""
declare -A FILE_SIZE=() FILE_SHA=() HALF_SIZE=() HALF_SHA=() HALF_SAVED=() HALF_PATH=() SAVED_TAR=()
declare -a HALF_NAMES=() EXTRAS=() LEFTOVERS=() SAVED_MANIFESTS=()
parse_state() {
    local kind rest a b c d e complete=0
    DEST_PATH=""
    BACKUPS_PATH=""
    STATE_REPORTED_SHA="absent"
    STATE_MANIFEST_SHA="absent"
    LOCK_STATE=""
    JOURNAL_STATE=""
    JOURNAL_RUN=""
    FILE_SIZE=()
    FILE_SHA=()
    HALF_NAMES=()
    HALF_SIZE=()
    HALF_SHA=()
    HALF_SAVED=()
    HALF_PATH=()
    EXTRAS=()
    LEFTOVERS=()
    SAVED_MANIFESTS=()
    SAVED_TAR=()
    rm -f -- "$2"
    while IFS=' ' read -r kind rest; do
        case "${kind}" in
            DEST) DEST_PATH="${rest}" ;;
            BACKUPS) BACKUPS_PATH="${rest}" ;;
            LOCK) LOCK_STATE="${rest}" ;;
            MANIFEST)
                read -r a b <<< "${rest}"
                case "${a}" in
                    absent) ;;
                    toolarge) die "the published manifest.json is larger than 64 KiB; the site refuses it. Fix it by hand." ;;
                    *)
                        STATE_REPORTED_SHA="${a}"
                        printf '%s' "${b}" | base64 -d > "$2" 2>/dev/null || return 1
                        # Hashed here. What the other side says its sha256 is does not count.
                        STATE_MANIFEST_SHA="$(sha_of "$2")"
                        ;;
                esac
                ;;
            FILE)
                read -r a b c <<< "${rest}"
                FILE_SIZE["${a}"]="${b}"
                FILE_SHA["${a}"]="${c}"
                ;;
            EXTRA) EXTRAS+=("${rest}") ;;
            LEFTOVER) LEFTOVERS+=("${rest}") ;;
            JOURNAL)
                read -r a b <<< "${rest}"
                JOURNAL_STATE="${a}"
                JOURNAL_RUN="${b:-}"
                ;;
            HALF)
                read -r a b c d e <<< "${rest}"
                HALF_NAMES+=("${a}")
                HALF_SIZE["${a}"]="${b}"
                HALF_SHA["${a}"]="${c}"
                HALF_SAVED["${a}"]="${d}"
                HALF_PATH["${a}"]="${e}"
                ;;
            SAVEDMANIFEST) SAVED_MANIFESTS+=("${rest}") ;;
            SAVEDTAR)
                read -r a b c <<< "${rest}"
                SAVED_TAR["${a} ${b}"]="${c}"
                ;;
            END) complete=1 ;;
        esac
    done < "$1"
    [[ "${complete}" == "1" && -n "${DEST_PATH}" && -n "${BACKUPS_PATH}" ]]
}

echo "Reading what is published (${WHERE})..."
if ! run_dest state < "${AGENT}" > "${WORK}/state"; then
    die "could not read the destination. Nothing was changed."
fi
parse_state "${WORK}/state" "${WORK}/old-manifest.json" \
    || die "the destination answer was cut short. Nothing was changed."
if [[ -n "${HOST}" ]]; then
    # These two paths go into the printed rollback line, inside quotes.
    [[ "${DEST_PATH}" =~ ${SAFE_ARG_RE} && "${BACKUPS_PATH}" =~ ${SAFE_ARG_RE} ]] \
        || die "the destination or backups path on ${HOST} has characters this script will not put in an ssh command"
fi
if [[ "${LOCK_STATE}" == "held" ]]; then
    [[ "${DRY_RUN}" == "1" ]] || die "another publish is running on ${DEST_PATH}. Nothing was changed."
    echo "note: another publish is running there right now. What is read here may be about to change."
    # Its journal is work in progress, not something that was left behind.
    JOURNAL_STATE=""
    HALF_NAMES=()
fi
if [[ "${JOURNAL_STATE}" == "unreadable" ]]; then
    die "${DEST_PATH}/.publish-journal is left from an interrupted publish and cannot be read. Compare the packs with manifest.json by hand, then remove that file."
fi

# An interrupted update left a tar with its new bytes under the old list.
# A real run puts the old file back first, and only then plans.
if [[ "${JOURNAL_STATE}" == "open" ]]; then
    echo "An earlier publish (${JOURNAL_RUN}) was interrupted between its renames."
    for name in "${HALF_NAMES[@]}"; do
        if [[ "${HALF_SAVED[${name}]}" == "ok" ]]; then
            echo "  ${name} does not match the published list. Its old file is saved at ${HALF_PATH[${name}]}"
        else
            echo "  ${name} does not match the published list, and its saved copy ${HALF_PATH[${name}]} is missing or damaged"
        fi
    done
    if [[ "${DRY_RUN}" == "1" ]]; then
        echo "  A real run first puts the old file(s) back, by rename. The plan below assumes that is done."
        for name in "${HALF_NAMES[@]}"; do
            [[ "${HALF_SAVED[${name}]}" == "ok" ]] \
                || die "the old ${name} cannot be put back: there is no good saved copy. Publish ${name} again from an outdir."
            FILE_SIZE["${name}"]="${HALF_SIZE[${name}]}"
            FILE_SHA["${name}"]="${HALF_SHA[${name}]}"
        done
    else
        echo "Repairing that first..."
        PHASE=repair
        if ! run_dest repair < "${AGENT}" > "${WORK}/repaired"; then
            die "the repair failed; see the lines above. Nothing else was done."
        fi
        sed -n 's/^RESTORED \(.*\)$/  put the old \1 back/p' "${WORK}/repaired"
        sed -n '/^DEST /,$p' "${WORK}/repaired" > "${WORK}/state"
        parse_state "${WORK}/state" "${WORK}/old-manifest.json" \
            || die "the destination answer was cut short after the repair. Run again."
        [[ -z "${JOURNAL_STATE}" ]] || die "the repair did not finish. Run again."
        echo "Repaired. The published list and its files match again."
        if [[ -n "${GOT_SIGNAL}" ]]; then
            echo "error: stopped by SIG${GOT_SIGNAL} after the repair. Nothing was published." >&2
            exit "${GOT_STATUS}"
        fi
        PHASE=check
    fi
fi

[[ "${STATE_REPORTED_SHA}" == "${STATE_MANIFEST_SHA}" ]] \
    || die "the destination's answer does not agree with itself (manifest sha256). Nothing was changed."
OLD_MANIFEST_SHA="${STATE_MANIFEST_SHA}"

declare -a OLD_NAMES=()
declare -A OLD_VER=() OLD_SHA=() OLD_SIZE=()
if [[ -f "${WORK}/old-manifest.json" ]]; then
    if ! python3 -I -c "${PY_MANIFEST}" "${WORK}/old-manifest.json" server > "${WORK}/old.tsv"; then
        die "the published manifest.json is not usable: $(cat "${WORK}/old.tsv"). Fix it by hand or roll back first."
    fi
    while IFS=$'\t' read -r name version sha size; do
        OLD_NAMES+=("${name}")
        OLD_VER["${name}"]="${version}"
        OLD_SHA["${name}"]="${sha}"
        OLD_SIZE["${name}"]="${size}"
    done < "${WORK}/old.tsv"
else
    : > "${WORK}/old.tsv"
fi
echo "Destination: ${DEST_PATH} (${#OLD_NAMES[@]} pack(s) published)"
if [[ "${JOURNAL_STATE}" == "committed" ]]; then
    echo "note: a publish that did switch (${JOURNAL_RUN}) left its journal behind. The next real publish removes it."
fi
if [[ "${#LEFTOVERS[@]}" -gt 0 ]]; then
    echo "note: ${#LEFTOVERS[@]} temp file(s) of a killed publish are in the folder. The next real publish removes them."
fi

# A folder without manifest.json: a first publish, or the wrong folder.
if [[ "${OLD_MANIFEST_SHA}" == "absent" ]]; then
    strangers=("${EXTRAS[@]}")
    for name in "${!FILE_SHA[@]}"; do
        [[ -n "${OUT_SHA[${name}]:-}" ]] || strangers+=("${name}")
    done
    if [[ "${FIRST_PUBLISH}" != "1" ]]; then
        echo "error: refused: ${DEST_PATH} has no manifest.json." >&2
        echo "  A packs folder that is in use always has one, so this is the wrong folder, a damaged" >&2
        echo "  one, or a new one. It holds $((${#FILE_SHA[@]} + ${#EXTRAS[@]})) other file(s). For a real first publish into" >&2
        echo "  a new, empty folder, add --first-publish." >&2
        exit 1
    fi
    if [[ "${#strangers[@]}" -gt 0 ]]; then
        echo "error: refused: --first-publish needs a folder with nothing else in it. ${DEST_PATH} holds:" >&2
        for name in "${strangers[@]:0:10}"; do
            echo "  ${name}" >&2
        done
        echo "  This script does not remove or overwrite files it did not bring." >&2
        exit 1
    fi
elif [[ "${FIRST_PUBLISH}" == "1" ]]; then
    die "refused: --first-publish, but ${DEST_PATH} already has a manifest.json with ${#OLD_NAMES[@]} pack(s). Leave the option out."
fi

# ------------------------------------------------------------ 3. the plan

# True when the destination file for $1 has the sha256 $2 and the size $3.
dest_file_ok() {
    [[ "${FILE_SHA[$1]:-}" == "$2" && "${FILE_SIZE[$1]:-}" == "$3" ]]
}

declare -A STATUS=() MODE=()
declare -a NEW_ORDER=() UPLOADS=() UPDATES=() NO_OLD_COPY=() SAME_VERSION=() NEEDS_UPDATE=()

for name in "${OLD_NAMES[@]}"; do
    NEW_ORDER+=("${name}")
    if [[ -z "${OUT_SHA[${name}]:-}" ]]; then
        dest_file_ok "${name}" "${OLD_SHA[${name}]}" "${OLD_SIZE[${name}]}" \
            || die "the published manifest lists ${name}, but its file at the destination is missing or damaged. Publish ${name} again from an outdir, or roll back."
        STATUS["${name}"]="kept"
    elif [[ "${OUT_SHA[${name}]}" == "${OLD_SHA[${name}]}" && "${OUT_VER[${name}]}" == "${OLD_VER[${name}]}" ]]; then
        if dest_file_ok "${name}" "${OLD_SHA[${name}]}" "${OLD_SIZE[${name}]}"; then
            STATUS["${name}"]="skipped"
        else
            STATUS["${name}"]="repaired"
            MODE["${name}"]="repair"
            UPLOADS+=("${name}")
        fi
    elif [[ "${OUT_VER[${name}]}" == "${OLD_VER[${name}]}" ]]; then
        SAME_VERSION+=("${name}")
    elif [[ "${UPDATE}" != "1" ]]; then
        NEEDS_UPDATE+=("${name}")
    else
        STATUS["${name}"]="updated (${OLD_VER[${name}]} -> ${OUT_VER[${name}]})"
        UPDATES+=("${name}")
        UPLOADS+=("${name}")
        if dest_file_ok "${name}" "${OLD_SHA[${name}]}" "${OLD_SIZE[${name}]}"; then
            MODE["${name}"]="update"
        else
            # The listed file was already missing or damaged. There is no old
            # copy worth saving, so this is a repair with new bytes.
            MODE["${name}"]="repair"
            NO_OLD_COPY+=("${name}")
        fi
    fi
done
for name in "${OUT_NAMES[@]}"; do
    [[ -z "${OLD_SHA[${name}]:-}" ]] || continue
    NEW_ORDER+=("${name}")
    STATUS["${name}"]="new"
    MODE["${name}"]="new"
    UPLOADS+=("${name}")
done

if [[ "${#SAME_VERSION[@]}" -gt 0 || "${#NEEDS_UPDATE[@]}" -gt 0 ]]; then
    for name in "${SAME_VERSION[@]}"; do
        echo "error: refused: ${name} is published as version ${OLD_VER[${name}]} (${OLD_SHA[${name}]:0:12}...)," \
            "and the outdir has other bytes (${OUT_SHA[${name}]:0:12}...) under the same version." >&2
    done
    if [[ "${#SAME_VERSION[@]}" -gt 0 ]]; then
        echo "  A tracker downloads a pack again only when its version changed, so these bytes would" >&2
        echo "  never reach a tracker that has the pack. Build it with a new version, then use --update." >&2
    fi
    for name in "${NEEDS_UPDATE[@]}"; do
        echo "error: refused: ${name} is published as version ${OLD_VER[${name}]} (${OLD_SHA[${name}]:0:12}...);" \
            "the outdir has version ${OUT_VER[${name}]} (${OUT_SHA[${name}]:0:12}...)." >&2
    done
    if [[ "${#NEEDS_UPDATE[@]}" -gt 0 ]]; then
        echo "  That is an update in place: every tracker that uses the pack downloads it again on its" >&2
        echo "  next sync. The old tar is saved first and the rollback line puts it back." >&2
        echo "  If that is what you want, run again with --update." >&2
    fi
    exit 1
fi

[[ "${#NEW_ORDER[@]}" -le "${MAX_PACKS}" ]] \
    || die "refused: the merged list would have ${#NEW_ORDER[@]} packs; the site allows ${MAX_PACKS}."

# The new manifest: one entry per line. lib/packs-dest.sh reads this layout.
entry_field() {
    if [[ -n "${OUT_SHA[$1]:-}" ]]; then
        case "$2" in
            version) echo "${OUT_VER[$1]}" ;;
            sha) echo "${OUT_SHA[$1]}" ;;
            size) echo "${OUT_SIZE[$1]}" ;;
        esac
    else
        case "$2" in
            version) echo "${OLD_VER[$1]}" ;;
            sha) echo "${OLD_SHA[$1]}" ;;
            size) echo "${OLD_SIZE[$1]}" ;;
        esac
    fi
}

NEW_MANIFEST="${WORK}/manifest.json"
{
    echo "["
    last=$((${#NEW_ORDER[@]} - 1))
    for i in "${!NEW_ORDER[@]}"; do
        name="${NEW_ORDER[${i}]}"
        comma=","
        [[ "${i}" == "${last}" ]] && comma=""
        printf '  {"name":"%s","version":"%s","sha256":"%s","size":%s}%s\n' \
            "${name}" "$(entry_field "${name}" version)" "$(entry_field "${name}" sha)" \
            "$(entry_field "${name}" size)" "${comma}"
    done
    echo "]"
} > "${NEW_MANIFEST}"
python3 -I -c "${PY_MANIFEST}" "${NEW_MANIFEST}" server > "${WORK}/new.tsv" \
    || die "internal: the new manifest does not parse: $(cat "${WORK}/new.tsv")"
[[ "$(size_of "${NEW_MANIFEST}")" -le "${MAX_MANIFEST_BYTES}" ]] \
    || die "refused: the new manifest would be larger than 64 KiB"
NEW_MANIFEST_SHA="$(sha_of "${NEW_MANIFEST}")"

print_table() {
    local name
    printf '%-22s %-10s %10s  %-12s  %s\n' "PACK" "VERSION" "SIZE" "SHA256" "STATUS"
    for name in "${NEW_ORDER[@]}"; do
        printf '%-22s %-10s %10s  %-12s  %s\n' "${name}" "$(entry_field "${name}" version)" \
            "$(entry_field "${name}" size)" "$(entry_field "${name}" sha | cut -c1-12)" "${STATUS[${name}]}"
    done
}

# A list without a name that released trackers ask for breaks those releases.
# This script cannot take a name off the list, so this is a first publish or
# a list that was edited by hand.
warn_missing() {
    local need name found
    for need in "${RELEASED_TRACKERS_NEED[@]}"; do
        found=0
        for name in "${NEW_ORDER[@]}"; do
            [[ "${name}" == "${need}" ]] && found=1
        done
        if [[ "${found}" == "0" ]]; then
            echo "warning: the list has no ${need}. Every released tracker asks for it by name and" >&2
            echo "  reports \"The reader pack did not match the site\" without it. Publish it too." >&2
        fi
    done
}

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"

# Optional check through the site, with the tracker token from the environment.
site_check() {
    local name status etag length sha
    if [[ "${HAVE_TOKEN}" != "1" ]]; then
        echo "Authenticated check: SKIPPED. PACKS_TRACKER_TOKEN is not set, so GET /api/tracker/packs" \
            "and the pack downloads were not tried through the site."
        return 0
    fi

    # site_get <path> <file or ->: one request. curl reads the token from the
    # mode-600 config file, so it is in no command line.
    site_get() {
        curl -sS --max-time 120 -K "${SITE_AUTH}" -D "${WORK}/site-headers" -o "$2" "${SITE_BASE}$1"
    }
    site_status() {
        awk 'toupper($1) ~ /^HTTP\// { code = $2 } END { print code }' "${WORK}/site-headers"
    }
    # The pack routes allow 8 requests in a burst per client IP, then one
    # every 10 seconds, and answer 429 with the seconds to wait. site_wait
    # sleeps that long and says so; it fails after 6 waits on one request or
    # when the server asks for more than two minutes.
    waits=0
    site_wait() {
        local seconds
        seconds="$(header_value "${WORK}/site-headers" retry-after)"
        [[ "${seconds}" =~ ^[0-9]{1,4}$ ]] || seconds=11
        waits=$((waits + 1))
        [[ "${waits}" -le 6 && "${seconds}" -le 120 ]] || return 1
        echo "  rate limited (429): waiting ${seconds} s, as the site asks"
        sleep "${seconds}"
    }

    echo "Authenticated check through ${SITE_BASE} (token from PACKS_TRACKER_TOKEN, not shown)..."
    waits=0
    while :; do
        site_get /api/tracker/packs "${WORK}/site-list.json" || die "site check: GET /api/tracker/packs failed"
        status="$(site_status)"
        [[ "${status}" == "429" ]] || break
        site_wait || die "site check: GET /api/tracker/packs is still rate limited (429). Try again in a few minutes."
    done
    [[ "${status}" == "200" ]] \
        || die "site check: GET /api/tracker/packs answered ${status}: $(head -c 200 "${WORK}/site-list.json")"
    python3 -I -c "${PY_MANIFEST}" "${WORK}/site-list.json" server > "${WORK}/site.tsv" \
        || die "site check: the list from the site is not a pack list"
    cmp -s "${WORK}/site.tsv" "${WORK}/new.tsv" \
        || die "site check: the site lists other packs than the manifest at the destination. Is PACKS_SITE_URL this server?"
    echo "  list: ${#NEW_ORDER[@]} pack(s), same as manifest.json"
    for name in "${NEW_ORDER[@]}"; do
        waits=0
        while :; do
            # The body is hashed as it arrives. It is not written to disk.
            sha="$(site_get "/api/tracker/packs/${name}" - | sha256sum | cut -d' ' -f1)" \
                || die "site check: GET /api/tracker/packs/${name} failed"
            status="$(site_status)"
            [[ "${status}" == "429" ]] || break
            site_wait || die "site check: ${name} is still rate limited (429). Try again in a few minutes."
        done
        [[ "${status}" == "200" ]] || die "site check: ${name} answered ${status}"
        etag="$(header_value "${WORK}/site-headers" etag)"
        length="$(header_value "${WORK}/site-headers" content-length)"
        [[ "${etag}" == "\"$(entry_field "${name}" sha)\"" ]] || die "site check: ${name} has ETag ${etag}"
        [[ "${length}" == "$(entry_field "${name}" size)" ]] || die "site check: ${name} has Content-Length ${length}"
        [[ "${sha}" == "$(entry_field "${name}" sha)" ]] || die "site check: ${name} downloads with another sha256"
        echo "  ${name}: 200, ETag, Content-Length and downloaded sha256 match"
    done
    echo "Authenticated check: OK"
}

if [[ "${#UPLOADS[@]}" -eq 0 ]] && cmp -s "${WORK}/old.tsv" "${WORK}/new.tsv"; then
    echo
    print_table
    echo
    warn_missing
    echo "Nothing to publish: every pack in ${OUTDIR} is already published with the same sha256."
    if [[ "${DRY_RUN}" == "1" ]]; then
        echo "DRY RUN: nothing was changed."
    elif ! (site_check); then
        echo "error: the check through the site failed. The destination itself was not changed." >&2
        exit 1
    fi
    exit 0
fi

if [[ "${DRY_RUN}" == "1" ]]; then
    echo
    print_table
    echo
    warn_missing
    echo "DRY RUN: nothing was changed. A real run would do this, in order:"
    step=0
    say() {
        step=$((step + 1))
        echo "  ${step}. $*"
    }
    if [[ "${JOURNAL_STATE}" == "open" ]]; then
        for name in "${HALF_NAMES[@]}"; do
            say "put the old ${name} back from ${HALF_PATH[${name}]}, by rename (repair of the interrupted publish)"
        done
    fi
    for name in "${UPLOADS[@]}"; do
        say "upload ${name} (${OUT_SIZE[${name}]} bytes) to ${DEST_PATH}/.${name}.publish-${STAMP}.tmp, check its sha256 and size there, chmod 0644"
    done
    say "write the new manifest (${#NEW_ORDER[@]} entries, $(size_of "${NEW_MANIFEST}") bytes) to ${DEST_PATH}/.manifest.json.publish-${STAMP}.tmp and flush it"
    say "check every pack the new manifest lists at the destination; stop here on any mismatch"
    if [[ "${OLD_MANIFEST_SHA}" != "absent" ]]; then
        say "copy ${DEST_PATH}/manifest.json to ${BACKUPS_PATH}/packs-manifest-pre-${STAMP}.json"
    fi
    for name in "${UPLOADS[@]}"; do
        [[ "${MODE[${name}]}" == "update" ]] || continue
        say "copy the old ${name} to ${BACKUPS_PATH}/packs-tars-pre-${STAMP}/ and check that copy's sha256"
    done
    for name in "${UPLOADS[@]}"; do
        [[ "${MODE[${name}]}" != "update" ]] || continue
        say "rename .${name}.publish-${STAMP}.tmp to ${name}"
    done
    for name in "${UPLOADS[@]}"; do
        [[ "${MODE[${name}]}" == "update" ]] || continue
        say "rename .${name}.publish-${STAMP}.tmp over ${name} (the update window opens: old list, new bytes)"
    done
    say "rename the new manifest over manifest.json (the switch; the window closes)"
    say "read manifest.json back and check every listed pack's sha256 and size"
    for name in "${NO_OLD_COPY[@]}"; do
        echo "note: ${name} at the destination is not the published file, so there is no old copy to save or roll back to."
    done
    exit 0
fi

# ------------------------------------------------------------ 4. publish

upload_stream() {
    local name
    for name in "${UPLOADS[@]}"; do
        if [[ "${MODE[${name}]}" == "update" ]]; then
            printf 'PACK %s %s %s update %s %s\n' "${name}" "${OUT_SIZE[${name}]}" "${OUT_SHA[${name}]}" \
                "${OLD_SIZE[${name}]}" "${OLD_SHA[${name}]}"
        else
            printf 'PACK %s %s %s %s\n' "${name}" "${OUT_SIZE[${name}]}" "${OUT_SHA[${name}]}" "${MODE[${name}]}"
        fi
        cat -- "${OUTDIR}/${name}"
    done
    printf 'MANIFEST %s %s\n' "$(size_of "${NEW_MANIFEST}")" "${NEW_MANIFEST_SHA}"
    cat -- "${NEW_MANIFEST}"
    printf 'END\n'
}

declare -a APPLY_ARGS=(--stamp "${STAMP}" --expect-old "${OLD_MANIFEST_SHA}")
if [[ "${FIRST_PUBLISH}" == "1" ]]; then
    APPLY_ARGS+=(--first-publish)
fi

warn_missing
echo "Publishing ${#UPLOADS[@]} pack(s) to ${DEST_PATH}..."
PHASE=apply
apply_status=0
{ cat -- "${AGENT}"; upload_stream; } | run_dest apply "${APPLY_ARGS[@]}" > "${WORK}/applied" || apply_status=$?
PHASE=report

# ------------------------------------------------------------ 5. what is live
#
# Nothing below trusts that the run went the way it was meant to. The
# destination's own answer is read (again, if the run did not end cleanly)
# and compared with the old list and the new one.

clean=0
known=0
if [[ "${apply_status}" == "0" ]] && grep -qx 'COMMITTED' "${WORK}/applied"; then
    sed -n '/^DEST /,$p' "${WORK}/applied" > "${WORK}/after"
    if parse_state "${WORK}/after" "${WORK}/after-manifest.json"; then
        clean=1
        known=1
    fi
fi
if [[ "${clean}" != "1" ]]; then
    echo "error: the publish did not end cleanly. Reading the destination again to see what is live..." >&2
    if run_dest state --stamp "${STAMP}" --wait 120 < "${AGENT}" > "${WORK}/after" \
        && parse_state "${WORK}/after" "${WORK}/after-manifest.json" \
        && [[ "${LOCK_STATE}" == "free" ]]; then
        known=1
    fi
fi

d="$(shown "${DEST_PATH:-?}")"
if [[ "${known}" != "1" ]]; then
    echo "error: what is live is NOT known: the destination could not be read again, or a publish is still running there." >&2
    echo "  Do not assume either list. Look with the same command and --dry-run." >&2
    echo "  If the new list turns out to be live, the old manifest is in ${BACKUPS_PATH:-the backups folder}" >&2
    echo "  as packs-manifest-pre-${STAMP}.json, and the Rollback line of a later run is not the one for this run." >&2
    exit 1
fi

# Which list is live, and does every pack it lists match it?
LIVE=other
declare -a BROKEN=()
if [[ "${STATE_MANIFEST_SHA}" == "${NEW_MANIFEST_SHA}" ]]; then
    LIVE=new
    for name in "${NEW_ORDER[@]}"; do
        dest_file_ok "${name}" "$(entry_field "${name}" sha)" "$(entry_field "${name}" size)" || BROKEN+=("${name}")
    done
elif [[ "${STATE_MANIFEST_SHA}" == "${OLD_MANIFEST_SHA}" ]]; then
    LIVE=old
    for name in "${OLD_NAMES[@]}"; do
        dest_file_ok "${name}" "${OLD_SHA[${name}]}" "${OLD_SIZE[${name}]}" || BROKEN+=("${name}")
    done
fi

# One command that puts one saved file back under its name, by rename: the
# file gets a new inode, so a download that is running keeps the bytes it
# started with, and the folder's mode is not touched.
put_back() {
    local tmp="${d}/.$2.rollback-${STAMP}.tmp"
    printf 'cp -p %s %s && mv -f %s %s' "$(shown "$1")" "${tmp}" "${tmp}" "${d}/$2"
}

if [[ "${LIVE}" == "old" ]]; then
    if [[ "${#BROKEN[@]}" -eq 0 ]]; then
        echo "error: the publish failed before the switch. The OLD list is still live, and every pack it lists" >&2
        echo "  is in place (read back just now, checked by sha256 and size). Nothing was published." >&2
        exit 1
    fi
    echo "error: the publish stopped half way. The OLD list is live, but these packs do not match it: ${BROKEN[*]}" >&2
    echo "  A tracker that asks for one of them gets a refusal until this is repaired." >&2
    echo "  Repair: run the same command again. It puts the old file(s) back first, then publishes." >&2
    for name in "${HALF_NAMES[@]}"; do
        if [[ "${HALF_SAVED[${name}]}" == "ok" ]]; then
            echo "  Or by hand: $(for_dest "$(put_back "${HALF_PATH[${name}]}" "${name}")")" >&2
        fi
    done
    exit 1
fi
if [[ "${LIVE}" == "other" ]]; then
    echo "error: manifest.json at the destination is neither the list from before this run nor the new one." >&2
    echo "  Something else changed it. Look at ${DEST_PATH}/manifest.json by hand; old manifests are in ${BACKUPS_PATH}." >&2
    exit 1
fi

# The new list is live.
echo
print_table
echo

rollback=""
for name in "${UPDATES[@]}"; do
    key="${name} ${OLD_SHA[${name}]}"
    saved="${SAVED_TAR[${key}]:-}"
    if [[ -n "${saved}" ]]; then
        echo "Old tar: ${saved}"
        rollback+="$(put_back "${saved}" "${name}") && "
    else
        echo "Old tar: none for ${name}. It was not the published file before this run, so there was nothing to save."
    fi
done
if [[ "${OLD_MANIFEST_SHA}" == "absent" ]]; then
    echo "Backup: none. There was no manifest.json before this publish."
    rollback+="rm -f ${d}/manifest.json"
else
    for entry in "${SAVED_MANIFESTS[@]}"; do
        [[ "${entry%% *}" == "${OLD_MANIFEST_SHA}" ]] && SAVED_MANIFEST="${entry#* }"
    done
    if [[ -n "${SAVED_MANIFEST}" ]]; then
        echo "Backup: ${SAVED_MANIFEST}"
        rollback+="$(put_back "${SAVED_MANIFEST}" manifest.json)"
    else
        echo "Backup: NOT FOUND. No packs-manifest-pre-${STAMP}*.json in ${BACKUPS_PATH} is the old manifest."
        rollback=""
    fi
fi
if [[ -n "${rollback}" ]]; then
    echo "Rollback: $(for_dest "${rollback}")"
else
    echo "Rollback: no line can be given without the old manifest. Publish the old build again instead."
fi
if [[ "${OLD_MANIFEST_SHA}" == "absent" ]]; then
    echo "  (that turns the pack routes off again: 503 packs_disabled)"
fi
warn_missing

if [[ "${#BROKEN[@]}" -gt 0 ]]; then
    echo "error: the NEW list is live, but these packs do not match it: ${BROKEN[*]}" >&2
    echo "  Run the rollback line above." >&2
    exit 1
fi
if [[ "${clean}" != "1" ]]; then
    echo "error: the run did not end cleanly, but the NEW list is live and every pack it lists is in place" >&2
    echo "  (read back just now, checked by sha256 and size). Keep the rollback line above." >&2
    exit 1
fi
echo "Verified: manifest.json read back, ${#NEW_ORDER[@]} pack(s) match by sha256 and size at ${DEST_PATH}."
if [[ -n "${GOT_SIGNAL}" ]]; then
    echo "error: SIG${GOT_SIGNAL} arrived during the publish. The publish is complete and verified;" \
        "the check through the site was not run." >&2
    exit "${GOT_STATUS}"
fi
PHASE=site
if ! (site_check); then
    echo "error: the packs are published and verified at the destination, but the check through the site failed." >&2
    echo "  Is PACKS_SITE_URL this server, and is PACKS_DIR this destination? The rollback line is above." >&2
    exit 1
fi
