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
#   publish-packs.sh [--dry-run] [--replace] [--keep N]
#                    [--host user@host | --local-volume] <outdir>
#
#   --host user@host  Destination is on that host, reached with ssh. The user
#                     must own the podman volumes (root on the VPS).
#   --local-volume    Destination is on this machine.
#   --dry-run         Read the destination, print every step, change nothing.
#   --replace         Allow a pack whose name is already published with other
#                     bytes. Without it that is refused: a changed pack gets a
#                     new name (words-v2), and old trackers keep the old one.
#   --keep N          Keep the N newest generations of each family (words-v1,
#                     words-v2, ... are one family). Default: keep all.
#
# What it does, in order:
#   1. Checks <outdir>: every tar is listed, every listed tar is there, sha256
#      and size match, names are [a-z0-9-]+.tar, and no tar holds an absolute
#      path, "..", a link or a device node. Tars are listed, never unpacked.
#   2. Reads what is published and merges: packs that <outdir> does not
#      mention stay listed, in their old order. New packs go last.
#   3. Uploads each new tar under a temp name, checks sha256 and size at the
#      destination, writes manifest.json.new, copies the old manifest to the
#      backups folder, then renames manifest.json.new over manifest.json last.
#      A failure before that rename leaves the old manifest and its tars alone.
#   4. Reads the manifest back and checks every listed pack at the destination.
#
# Environment (destination):
#   PACKS_DEST        A plain directory to publish into, on this machine or on
#                     --host. With it set and no --host, --local-volume is
#                     implied. Unset: the podman volume is used.
#   PACKS_VOLUME      Exact podman volume name. Unset: the one volume named
#                     packs-data or <project>_packs-data.
#   PACKS_BACKUP_DIR  Where old manifests are kept, on the destination machine
#                     (default: ~/scuffed-crew-backups there).
#   PACKS_PIN         Pack names --keep never drops (default: heroes-v1.tar).
#
# Environment (optional check through the site, after the publish):
#   PACKS_SITE_URL       Site origin, for example https://example.org
#   PACKS_TRACKER_TOKEN  A tracker token. Read from the environment only. It
#                        is never printed and is not accepted as an argument.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AGENT="${SCRIPT_DIR}/lib/packs-dest.sh"

MAX_PACKS=32
MAX_MANIFEST_BYTES=65536
SAFE_ARG_RE='^[A-Za-z0-9_./:@=+-]+$'

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    sed -n '2,/^$/{ /^$/d; s/^# \{0,1\}//; p; }' "${BASH_SOURCE[0]}" >&2
    echo >&2
    echo "usage: publish-packs.sh [--dry-run] [--replace] [--keep N] [--host user@host | --local-volume] <outdir>" >&2
    exit 2
}

DRY_RUN=0
REPLACE=0
KEEP=""
HOST=""
LOCAL_VOLUME=0
OUTDIR=""

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --dry-run) DRY_RUN=1; shift ;;
        --replace) REPLACE=1; shift ;;
        --local-volume) LOCAL_VOLUME=1; shift ;;
        --keep)
            [[ "$#" -ge 2 ]] || usage
            KEEP="$2"
            shift 2
            ;;
        --host)
            [[ "$#" -ge 2 ]] || usage
            HOST="$2"
            shift 2
            ;;
        -h | --help) usage ;;
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
if [[ -n "${KEEP}" && ! "${KEEP}" =~ ^[1-9][0-9]{0,3}$ ]]; then
    die "--keep takes a whole number, 1 or more"
fi
if [[ -n "${HOST}" && "${LOCAL_VOLUME}" == "1" ]]; then
    die "--host and --local-volume are two different destinations; give one"
fi
if [[ -n "${HOST}" && ! "${HOST}" =~ ^[A-Za-z0-9_][A-Za-z0-9_.@:-]*$ ]]; then
    die "--host must look like user@host"
fi
if [[ -z "${HOST}" && "${LOCAL_VOLUME}" != "1" && -z "${PACKS_DEST:-}" ]]; then
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

# Refuse a tar with an absolute path, "..", a link, or a device node.
# The archive is listed. It is never unpacked.
check_tar_members() {
    local tar="$1" name="$2" names verbose line
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

echo "Reading what is published (${WHERE})..."
if ! run_dest state < "${AGENT}" > "${WORK}/state"; then
    die "could not read the destination. Nothing was changed."
fi

# Fills DEST_PATH, BACKUPS_PATH, STATE_MANIFEST_SHA, and FILE_SIZE / FILE_SHA.
# Writes the manifest it carries to the file named in $2 (or nothing).
declare -A FILE_SIZE=() FILE_SHA=()
parse_state() {
    local kind rest a b c complete=0
    DEST_PATH=""
    BACKUPS_PATH=""
    STATE_MANIFEST_SHA="absent"
    FILE_SIZE=()
    FILE_SHA=()
    rm -f -- "$2"
    while IFS=' ' read -r kind rest; do
        case "${kind}" in
            DEST) DEST_PATH="${rest}" ;;
            BACKUPS) BACKUPS_PATH="${rest}" ;;
            MANIFEST)
                read -r a b <<< "${rest}"
                case "${a}" in
                    absent) ;;
                    toolarge) die "the published manifest.json is larger than 64 KiB; the site refuses it. Fix it by hand." ;;
                    *)
                        STATE_MANIFEST_SHA="${a}"
                        printf '%s' "${b}" | base64 -d > "$2"
                        ;;
                esac
                ;;
            FILE)
                read -r a b c <<< "${rest}"
                FILE_SIZE["${a}"]="${b}"
                FILE_SHA["${a}"]="${c}"
                ;;
            END) complete=1 ;;
        esac
    done < "$1"
    [[ "${complete}" == "1" && -n "${DEST_PATH}" ]] || die "the destination answer was cut short"
}

parse_state "${WORK}/state" "${WORK}/old-manifest.json"
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

# ------------------------------------------------------------ 3. the plan

# True when the destination file for $1 has the sha256 $2 and the size $3.
dest_file_ok() {
    [[ "${FILE_SHA[$1]:-}" == "$2" && "${FILE_SIZE[$1]:-}" == "$3" ]]
}

declare -A STATUS=()
declare -a NEW_ORDER=() UPLOADS=() DROPS=() CONFLICTS=()

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
            UPLOADS+=("${name}")
        fi
    elif [[ "${REPLACE}" == "1" ]]; then
        STATUS["${name}"]="replaced"
        UPLOADS+=("${name}")
    else
        CONFLICTS+=("${name}")
    fi
done
for name in "${OUT_NAMES[@]}"; do
    [[ -z "${OLD_SHA[${name}]:-}" ]] || continue
    NEW_ORDER+=("${name}")
    STATUS["${name}"]="new"
    UPLOADS+=("${name}")
done

if [[ "${#CONFLICTS[@]}" -gt 0 ]]; then
    for name in "${CONFLICTS[@]}"; do
        echo "error: refused: ${name} is already published with other bytes" \
            "(published ${OLD_SHA[${name}]:0:12}... version ${OLD_VER[${name}]}," \
            "outdir ${OUT_SHA[${name}]:0:12}... version ${OUT_VER[${name}]})" >&2
    done
    echo "  Packs do not change once published. Build the changed pack under a new name" >&2
    echo "  (words-v1.tar -> words-v2.tar) and publish that. --replace overrides this." >&2
    exit 1
fi

# Family and generation of a pack name: words-v2.tar is family words,
# generation 2. heroes-v2-1080.tar is family heroes-1080, generation 2.
family_of() {
    local stem="${1%.tar}"
    if [[ "${stem}" =~ ^(.*)-v([0-9]{1,9})(-[a-z0-9-]+)?$ ]]; then
        FAMILY="${BASH_REMATCH[1]}${BASH_REMATCH[3]}"
        GENERATION=$((10#${BASH_REMATCH[2]}))
    else
        FAMILY="${stem}"
        GENERATION=0
    fi
}

if [[ -n "${KEEP}" ]]; then
    read -r -a PINNED <<< "${PACKS_PIN-heroes-v1.tar}"
    declare -a kept_order=()
    for name in "${NEW_ORDER[@]}"; do
        family_of "${name}"
        family="${FAMILY}"
        generation="${GENERATION}"
        newer=0
        for other in "${NEW_ORDER[@]}"; do
            family_of "${other}"
            if [[ "${FAMILY}" == "${family}" && "${GENERATION}" -gt "${generation}" ]]; then
                newer=$((newer + 1))
            fi
        done
        pinned=0
        for other in "${PINNED[@]}"; do
            [[ "${other}" == "${name}" ]] && pinned=1
        done
        if [[ "${newer}" -ge "${KEEP}" && "${pinned}" == "0" && -z "${OUT_SHA[${name}]:-}" ]]; then
            STATUS["${name}"]="dropped"
            DROPS+=("${name}")
        else
            kept_order+=("${name}")
        fi
    done
    NEW_ORDER=("${kept_order[@]}")
fi

[[ "${#NEW_ORDER[@]}" -le "${MAX_PACKS}" ]] \
    || die "refused: the merged list would have ${#NEW_ORDER[@]} packs; the site allows ${MAX_PACKS}. Use --keep N."

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

print_table() {
    local name
    printf '%-26s %10s  %-12s  %s\n' "PACK" "SIZE" "SHA256" "STATUS"
    for name in "${NEW_ORDER[@]}"; do
        printf '%-26s %10s  %-12s  %s\n' "${name}" "$(entry_field "${name}" size)" \
            "$(entry_field "${name}" sha | cut -c1-12)" "${STATUS[${name}]}"
    done
    for name in "${DROPS[@]}"; do
        printf '%-26s %10s  %-12s  %s\n' "${name}" "${OLD_SIZE[${name}]}" \
            "${OLD_SHA[${name}]:0:12}" "dropped"
    done
}

for name in "${UPLOADS[@]}"; do
    if [[ "${STATUS[${name}]}" == "replaced" && "${OUT_VER[${name}]}" == "${OLD_VER[${name}]}" ]]; then
        echo "warning: ${name} is replaced under the same version (${OLD_VER[${name}]})." \
            "A tracker that already has it will not download it again." >&2
    fi
done

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP_FILE="${BACKUPS_PATH}/packs-manifest-pre-${STAMP}.json"
TAR_BACKUPS="${BACKUPS_PATH}/packs-tars-pre-${STAMP}"

# Optional check through the site, with a tracker token from the environment.
site_check() {
    local base="${PACKS_SITE_URL:-}" name status etag length sha requests=0
    if [[ -z "${PACKS_TRACKER_TOKEN:-}" ]]; then
        echo "Authenticated check: SKIPPED. PACKS_TRACKER_TOKEN is not set, so GET /api/tracker/packs" \
            "and the pack downloads were not tried through the site."
        return 0
    fi
    [[ -n "${base}" ]] || die "PACKS_TRACKER_TOKEN is set but PACKS_SITE_URL is not; the site was not asked"
    base="${base%/}"
    [[ "${base}" =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?$ || "${base}" =~ ^http://(127\.0\.0\.1|localhost)(:[0-9]+)?$ ]] \
        || die "PACKS_SITE_URL must be an https:// origin (plain http only for 127.0.0.1); the site was not asked"
    [[ "${PACKS_TRACKER_TOKEN}" =~ ^[A-Za-z0-9._~+/=-]+$ ]] \
        || die "PACKS_TRACKER_TOKEN has characters a token cannot have; the site was not asked"
    command -v curl >/dev/null 2>&1 || die "curl is required for the check through the site"

    # The routes allow a burst of 8, then one request every 10 seconds.
    site_pace() {
        requests=$((requests + 1))
        [[ "${requests}" -le 8 ]] || sleep 11
    }
    # site_get <path> <file or ->. The token goes to curl on stdin, so it is
    # in no command line.
    site_get() {
        printf 'header = "Authorization: Bearer %s"\n' "${PACKS_TRACKER_TOKEN}" \
            | curl -sS --max-time 120 -K - -D "${WORK}/site-headers" -o "$2" "${base}$1"
    }
    site_status() {
        awk 'toupper($1) ~ /^HTTP\// { code = $2 } END { print code }' "${WORK}/site-headers"
    }

    echo "Authenticated check through ${base} (token from PACKS_TRACKER_TOKEN, not shown)..."
    site_pace
    site_get /api/tracker/packs "${WORK}/site-list.json" || die "site check: GET /api/tracker/packs failed"
    status="$(site_status)"
    [[ "${status}" == "200" ]] \
        || die "site check: GET /api/tracker/packs answered ${status}: $(head -c 200 "${WORK}/site-list.json")"
    python3 -I -c "${PY_MANIFEST}" "${WORK}/site-list.json" server > "${WORK}/site.tsv" \
        || die "site check: the list from the site is not a pack list"
    cmp -s "${WORK}/site.tsv" "${WORK}/new.tsv" \
        || die "site check: the site lists other packs than the manifest that was just written. Is PACKS_SITE_URL this server?"
    echo "  list: ${#NEW_ORDER[@]} pack(s), same as manifest.json"
    for name in "${NEW_ORDER[@]}"; do
        site_pace
        # The body is hashed as it arrives. It is not written to disk.
        sha="$(site_get "/api/tracker/packs/${name}" - | sha256sum | cut -d' ' -f1)" \
            || die "site check: GET /api/tracker/packs/${name} failed"
        status="$(site_status)"
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

if [[ "${#UPLOADS[@]}" -eq 0 && "${#DROPS[@]}" -eq 0 ]] && cmp -s "${WORK}/old.tsv" "${WORK}/new.tsv"; then
    echo
    print_table
    echo
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
    echo "DRY RUN: nothing was changed. A real run would do this, in order:"
    step=0
    say() {
        step=$((step + 1))
        echo "  ${step}. $*"
    }
    for name in "${UPLOADS[@]}"; do
        say "upload ${name} (${OUT_SIZE[${name}]} bytes) to ${DEST_PATH}/.${name}.publish-${STAMP}.tmp, check its sha256 and size there, chmod 0644"
    done
    say "write ${DEST_PATH}/manifest.json.new (${#NEW_ORDER[@]} entries, $(size_of "${NEW_MANIFEST}") bytes) and sync it"
    say "check every pack the new manifest lists at the destination; stop here on any mismatch"
    for name in "${UPLOADS[@]}"; do
        [[ "${STATUS[${name}]}" != "replaced" ]] || continue
        say "rename .${name}.publish-${STAMP}.tmp to ${name}"
    done
    if [[ "${OLD_MANIFEST_SHA}" != "absent" ]]; then
        say "copy ${DEST_PATH}/manifest.json to ${BACKUP_FILE}"
    fi
    for name in "${UPLOADS[@]}"; do
        [[ "${STATUS[${name}]}" == "replaced" ]] || continue
        say "copy the old ${name} to ${TAR_BACKUPS}/, then rename .${name}.publish-${STAMP}.tmp over ${name}"
    done
    say "rename manifest.json.new over manifest.json"
    for name in "${DROPS[@]}"; do
        say "move ${name} to ${TAR_BACKUPS}/ (no longer listed)"
    done
    say "read manifest.json back and check every listed pack's sha256 and size"
    exit 0
fi

# ------------------------------------------------------------ 4. publish

upload_stream() {
    local name mode
    for name in "${UPLOADS[@]}"; do
        case "${STATUS[${name}]}" in
            replaced) mode=replace ;;
            repaired) mode=repair ;;
            *) mode=new ;;
        esac
        printf 'PACK %s %s %s %s\n' "${name}" "${OUT_SIZE[${name}]}" "${OUT_SHA[${name}]}" "${mode}"
        cat -- "${OUTDIR}/${name}"
    done
    for name in "${DROPS[@]}"; do
        printf 'DROP %s\n' "${name}"
    done
    printf 'MANIFEST %s %s\n' "$(size_of "${NEW_MANIFEST}")" "$(sha_of "${NEW_MANIFEST}")"
    cat -- "${NEW_MANIFEST}"
    printf 'END\n'
}

echo "Publishing ${#UPLOADS[@]} pack(s) to ${DEST_PATH}..."
if ! { cat -- "${AGENT}"; upload_stream; } \
    | run_dest apply --stamp "${STAMP}" --expect-old "${OLD_MANIFEST_SHA}" > "${WORK}/applied"; then
    if grep -qx 'COMMITTED' "${WORK}/applied"; then
        echo "error: the new manifest is live, but a step after the switch failed. Check the destination." >&2
    else
        echo "error: the publish failed before the switch. The published manifest and its packs were not changed." >&2
    fi
    exit 1
fi
grep -qx 'COMMITTED' "${WORK}/applied" || die "the destination did not confirm the switch. Check it by hand."

# ------------------------------------------------------------ 5. verify

sed -n '/^DEST /,$p' "${WORK}/applied" > "${WORK}/after"
parse_state "${WORK}/after" "${WORK}/after-manifest.json"
failed=0
if ! cmp -s "${WORK}/after-manifest.json" "${NEW_MANIFEST}"; then
    echo "error: manifest.json at the destination is not the one that was written" >&2
    failed=1
fi
for name in "${NEW_ORDER[@]}"; do
    if ! dest_file_ok "${name}" "$(entry_field "${name}" sha)" "$(entry_field "${name}" size)"; then
        echo "error: ${name} at the destination does not match manifest.json" >&2
        failed=1
    fi
done

echo
print_table
echo

SAVED_TARS="$(sed -n 's/^TARS //p' "${WORK}/applied" | head -n1)"
d="$(shown "${DEST_PATH}")"
if [[ "${OLD_MANIFEST_SHA}" == "absent" ]]; then
    echo "Backup: none. There was no manifest.json before this publish."
    rollback="rm -f ${d}/manifest.json"
else
    BACKUP_FILE="$(sed -n 's/^BACKUP //p' "${WORK}/applied" | head -n1)"
    echo "Backup: ${BACKUP_FILE}"
    rollback="cp -p $(shown "${BACKUP_FILE}") ${d}/manifest.json.new && mv -f ${d}/manifest.json.new ${d}/manifest.json"
fi
if [[ -n "${SAVED_TARS}" ]]; then
    echo "Old tars: ${SAVED_TARS}/"
    rollback="cp -pR $(shown "${SAVED_TARS}")/. ${d}/ && ${rollback}"
fi
if [[ -n "${HOST}" ]]; then
    rollback="ssh ${HOST} '${rollback}'"
fi
echo "Rollback: ${rollback}"
if [[ "${OLD_MANIFEST_SHA}" == "absent" ]]; then
    echo "  (that turns the pack routes off again: 503 packs_disabled)"
fi

if [[ "${failed}" == "1" ]]; then
    echo "error: the publish was switched in but did not verify. Run the rollback line above." >&2
    exit 1
fi
echo "Verified: manifest.json read back, ${#NEW_ORDER[@]} pack(s) match by sha256 and size at ${DEST_PATH}."
if ! (site_check); then
    echo "error: the packs are published and verified at the destination, but the check through the site failed." >&2
    echo "  Is PACKS_SITE_URL this server, and is PACKS_DIR this destination? The rollback line is above." >&2
    exit 1
fi
