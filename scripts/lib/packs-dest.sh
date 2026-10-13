#!/usr/bin/env bash
# Destination side of scripts/publish-packs.sh.
#
# This file runs on the machine that holds PACKS_DIR. publish-packs.sh sends
# it down a pipe and runs it with `bash -s -- <verb> ...`, on this machine or
# through ssh, so nothing has to be installed on the server. The bytes that
# follow this script on stdin are the upload stream (see `apply`). bash reads
# a script from a pipe one byte at a time, so the stream stays intact.
#
# It needs bash 4.4+, GNU coreutils 8.24+ (`sync FILE`) and flock from
# util-linux. No jq, no python. podman only for --volume. It never unpacks a
# tar, and it never calls a bare `sync`: only the files it wrote are flushed.
#
# Verbs:
#   state    Print what is there: the folder, the manifest, every file, and
#            what an interrupted publish left behind. Changes nothing.
#   repair   Put back the old file(s) of an interrupted publish.
#   apply    Read the upload stream, check it, then switch manifest.json.
#
# Where:
#   --dir PATH       a plain directory
#   --volume NAME    a podman volume, exact name; `auto` picks the one named
#                    packs-data or <project>_packs-data
#   --backups PATH   where old manifests and old tars go
#                    (default: ~/scuffed-crew-backups)
#
# shellcheck shell=bash

set -euo pipefail
umask 022

if [[ -z "${BASH_VERSINFO[0]:-}" ]] || [[ "${BASH_VERSINFO[0]}" -lt 4 ]] \
    || [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -lt 4 ]]; then
    echo "error: the destination needs bash 4.4 or newer; it has ${BASH_VERSION:-an unknown shell}" >&2
    exit 1
fi

MAX_MANIFEST_BYTES=65536
NAME_RE='^[A-Za-z0-9][A-Za-z0-9._-]*$'
SHA_RE='^[0-9a-f]{64}$'
STAMP_RE='^[0-9]{8}T[0-9]{6}Z$'
RUN_RE='^[0-9]{8}T[0-9]{6}Z(-[0-9]+)?$'
# Temp names this script (and its printed rollback line) uses inside DEST.
TMP_RE='^\.[A-Za-z0-9][A-Za-z0-9._-]*\.(publish|restore|rollback)-[0-9]{8}T[0-9]{6}Z(-[0-9]+)?\.tmp$'
# One entry per line, exactly as publish-packs.sh writes it.
ENTRY_RE='^  \{"name":"([A-Za-z0-9][A-Za-z0-9._-]*)","version":"([A-Za-z0-9._-]+)","sha256":"([0-9a-f]{64})","size":([0-9]+)\},?$'

DEST=""
BACKUPS=""
JOURNAL=""
STAMP=""
EXPECT_OLD=""
FIRST_PUBLISH=0
WAIT=0

die() {
    echo "error: $*" >&2
    exit 1
}

sha_of() {
    sha256sum -- "$1" | cut -d' ' -f1
}

size_of() {
    stat -c %s -- "$1"
}

# fsync one file or one folder. Never the whole machine: a bare `sync` stalls
# a busy host and flushes filesystems this script has nothing to do with.
flush_path() {
    if ! sync -- "$1"; then
        echo "error: could not flush $1 to disk" >&2
        return 1
    fi
}

preflight() {
    local tool help
    for tool in sha256sum stat base64 head cut cp mv mkdir chmod rm sync grep; do
        command -v "${tool}" >/dev/null 2>&1 || die "${tool} is required on the destination"
    done
    command -v flock >/dev/null 2>&1 \
        || die "flock is required on the destination (it is part of util-linux). Without a lock two publishes can overwrite each other, so nothing is done without it."
    help="$(LC_ALL=C sync --help 2>&1 || true)"
    [[ "${help}" == *FILE* ]] \
        || die "the destination's sync cannot flush one file (GNU coreutils 8.24 or newer is required)"
}

# A path with its symlinks resolved, also when its last folders do not exist yet.
physical_path() {
    local path="$1" tail=""
    [[ "${path}" == /* ]] || path="${PWD}/${path}"
    path="${path%/}"
    case "${path}/" in
        */../* | */./*) die "the backups path must not hold . or .. parts: $1" ;;
    esac
    while [[ -n "${path}" && ! -d "${path}" ]]; do
        tail="/${path##*/}${tail}"
        path="${path%/*}"
    done
    path="$(cd "${path:-/}" && pwd -P)"
    echo "${path%/}${tail}"
}

# True when the folder $1 (or, while it does not exist yet, the nearest folder
# above it that does) is inside a git work tree.
in_git_tree() {
    local dir="$1"
    while [[ -n "${dir}" && ! -d "${dir}" ]]; do
        dir="${dir%/*}"
    done
    while :; do
        [[ -e "${dir:-/}/.git" ]] && return 0
        [[ -n "${dir}" && "${dir}" != "/" ]] || return 1
        dir="${dir%/*}"
    done
}

# Mountpoint of the packs volume. backup.sh reads other volumes the same way.
# A publish writes, so two candidate volumes are refused instead of guessed,
# and a name must match exactly: `podman volume inspect` also takes a prefix.
resolve_volume() {
    local want="$1" name info
    local -a found=()
    command -v podman >/dev/null 2>&1 || die "podman is required to find the packs volume"
    while IFS= read -r name; do
        if [[ "${want}" == "auto" ]]; then
            case "${name}" in
                packs-data | *_packs-data) found+=("${name}") ;;
            esac
        elif [[ "${name}" == "${want}" ]]; then
            found+=("${name}")
        fi
    done < <(podman volume ls --format '{{.Name}}' 2>/dev/null < /dev/null || true)
    if [[ "${#found[@]}" -eq 0 ]]; then
        if [[ "${want}" == "auto" ]]; then
            die "no podman volume named packs-data or <project>_packs-data here; start the stack once or pass PACKS_VOLUME"
        fi
        die "no podman volume is named exactly ${want} here (PACKS_VOLUME takes the full name)"
    fi
    if [[ "${#found[@]}" -gt 1 ]]; then
        die "more than one packs volume (${found[*]}); set PACKS_VOLUME to the one compose mounts"
    fi
    info="$(podman volume inspect "${found[0]}" --format '{{.Name}}|{{.Mountpoint}}' 2>/dev/null < /dev/null || true)"
    [[ "${info%%|*}" == "${found[0]}" ]] \
        || die "podman answered for another volume than ${found[0]}"
    info="${info#*|}"
    [[ -n "${info}" && -d "${info}" ]] || die "podman volume ${found[0]} has no readable mountpoint"
    echo "${info}"
}

parse_where() {
    local dir="" volume=""
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --dir) dir="$2"; shift 2 ;;
            --volume) volume="$2"; shift 2 ;;
            --backups) BACKUPS="$2"; shift 2 ;;
            --stamp) STAMP="$2"; shift 2 ;;
            --expect-old) EXPECT_OLD="$2"; shift 2 ;;
            --wait) WAIT="$2"; shift 2 ;;
            --first-publish) FIRST_PUBLISH=1; shift ;;
            *) die "unknown destination argument: $1" ;;
        esac
    done
    if [[ -n "${dir}" ]]; then
        DEST="${dir}"
    elif [[ -n "${volume}" ]]; then
        DEST="$(resolve_volume "${volume}")"
    else
        die "no destination given (--dir or --volume)"
    fi
    [[ -d "${DEST}" ]] || die "destination is not a directory: ${DEST}"
    DEST="$(cd "${DEST}" && pwd -P)"
    BACKUPS="$(physical_path "${BACKUPS:-${HOME}/scuffed-crew-backups}")"
    [[ "${DEST}" != *$'\n'* && "${BACKUPS}" != *$'\n'* ]] || die "a path with a line break in it is refused"
    case "${BACKUPS}/" in
        "${DEST}/"*) die "the backups folder (${BACKUPS}) must be outside the destination (${DEST})" ;;
    esac
    if in_git_tree "${BACKUPS}"; then
        die "the backups folder (${BACKUPS}) is inside a git work tree. Old packs are saved there, and packs must never sit in a repo."
    fi
    [[ -z "${STAMP}" || "${STAMP}" =~ ${STAMP_RE} ]] || die "bad --stamp"
    [[ "${WAIT}" =~ ^[0-9]{1,4}$ ]] || die "bad --wait"
    [[ ! -L "${DEST}/manifest.json" ]] || die "manifest.json is a symlink in ${DEST}"
    JOURNAL="${DEST}/.publish-journal"
}

# One publish at a time. $1 is how many seconds to wait for the lock; 0 fails
# at once. The lock is on the folder itself and goes away with the process.
take_lock() {
    exec 9< "${DEST}"
    if [[ "$1" == "0" ]]; then
        flock -n 9
    else
        flock -w "$1" 9
    fi
}

manifest_sha() {
    if [[ -f "${DEST}/manifest.json" ]]; then
        sha_of "${DEST}/manifest.json"
    else
        echo absent
    fi
}

# ---------------------------------------------------------------- journal
#
# An update keeps the pack's name, so its tar and the manifest are two
# renames. Before the first of them, the journal says which tars are about to
# be swapped, what their old bytes were and where the old copy is. Whatever
# stops the run after that, the journal plus the live manifest are enough to
# put the folder right. The live manifest decides, pack by pack: a file is
# only ever replaced with bytes that the live manifest names.

declare -a J_NAMES=() J_SIZE=() J_SHA=() J_SAVED=()
J_RUN=""
VERDICT=""

load_journal() {
    local kind a b c d ended=0
    J_RUN=""
    J_NAMES=()
    J_SIZE=()
    J_SHA=()
    J_SAVED=()
    while IFS=' ' read -r kind a b c d; do
        case "${kind}" in
            JOURNAL) J_RUN="${a}" ;;
            TAR)
                [[ "${a}" =~ ${NAME_RE} && "${b}" =~ ^[0-9]+$ && "${c}" =~ ${SHA_RE} && -n "${d}" ]] || return 1
                J_NAMES+=("${a}")
                J_SIZE+=("${b}")
                J_SHA+=("${c}")
                J_SAVED+=("${d}")
                ;;
            END) ended=1 ;;
            *) return 1 ;;
        esac
    done < "${JOURNAL}"
    [[ "${ended}" == "1" && "${J_RUN}" =~ ${RUN_RE} ]]
}

# True when DEST/$1 is a plain file with the sha256 $2. A hash that could not
# be computed is not a match.
file_is() {
    local actual
    [[ -f "${DEST}/$1" && ! -L "${DEST}/$1" ]] || return 1
    actual="$(sha_of "${DEST}/$1")" || return 1
    [[ "${actual}" == "$2" ]]
}

# 0: the live manifest.json holds the string $1. 1: it does not. 2: it could
# not be read. The manifest may have any JSON layout, so it is searched, not
# parsed; a file name or a sha256 is in it as that exact text or not at all.
live_mentions() {
    local status=0
    [[ -f "${DEST}/manifest.json" && ! -L "${DEST}/manifest.json" ]] || return 2
    grep -qF -- "$1" "${DEST}/manifest.json" || status=$?
    [[ "${status}" -le 1 ]] || status=2
    return "${status}"
}

# What the live manifest says about one pack of the journal ($1, old sha256
# $2). Sets VERDICT:
#   fine      the file holds bytes the live list names, or the live list does
#             not name the pack at all: nothing to do
#   restore   the live list names the old bytes, and the file is not them
#   unknown   something could not be read, or the file is neither; hands off
# "Could not hash it" is never taken for "it differs".
tar_verdict() {
    local name="$1" old="$2" actual status
    VERDICT=unknown
    status=0
    live_mentions "\"${name}\"" || status=$?
    if [[ "${status}" == "1" ]]; then
        VERDICT=fine
        return 0
    fi
    [[ "${status}" == "0" ]] || return 0
    if [[ -f "${DEST}/${name}" && ! -L "${DEST}/${name}" ]]; then
        actual="$(sha_of "${DEST}/${name}")" || return 0
        [[ "${actual}" =~ ${SHA_RE} ]] || return 0
        status=0
        live_mentions "${actual}" || status=$?
        if [[ "${status}" == "0" ]]; then
            VERDICT=fine
            return 0
        fi
        [[ "${status}" == "1" ]] || return 0
    elif [[ -e "${DEST}/${name}" || -L "${DEST}/${name}" ]]; then
        return 0
    fi
    status=0
    live_mentions "${old}" || status=$?
    if [[ "${status}" == "0" ]]; then
        VERDICT=restore
    fi
    return 0
}

# Finish what an interrupted publish left: every pack of the journal whose
# old bytes the live manifest still names gets them back, by rename. A pack
# whose file already matches the live manifest is not touched, whichever
# manifest that is. Called through `||` and `if`, so every step is checked by
# hand.
recover_journal() {
    local i name tmp
    [[ -e "${JOURNAL}" || -L "${JOURNAL}" ]] || return 0
    if [[ ! -f "${JOURNAL}" || -L "${JOURNAL}" ]] || ! load_journal; then
        echo "error: ${JOURNAL} cannot be read. Compare the packs with manifest.json by hand, then remove that file." >&2
        return 1
    fi
    for i in "${!J_NAMES[@]}"; do
        name="${J_NAMES[${i}]}"
        tar_verdict "${name}" "${J_SHA[${i}]}"
        case "${VERDICT}" in
            fine) continue ;;
            restore) ;;
            *)
                echo "error: cannot tell what ${name} should be: manifest.json or the file could not be read, or the file is neither the listed pack nor the old one. Nothing was done to it." >&2
                echo "  Compare ${DEST}/${name} with manifest.json by hand, then remove ${JOURNAL}" >&2
                return 1
                ;;
        esac
        tmp="${DEST}/.${name}.restore-${J_RUN}.tmp"
        if [[ -f "${J_SAVED[${i}]}" && ! -L "${J_SAVED[${i}]}" ]] \
            && cp -p -- "${J_SAVED[${i}]}" "${tmp}" \
            && file_is ".${name}.restore-${J_RUN}.tmp" "${J_SHA[${i}]}" \
            && flush_path "${tmp}" \
            && mv -fT -- "${tmp}" "${DEST}/${name}"; then
            echo "RESTORED ${name}"
        else
            rm -f -- "${tmp}"
            echo "error: could not put the old ${name} back. Its saved copy should be ${J_SAVED[${i}]}" >&2
            echo "  If that copy is gone for good: remove ${JOURNAL} by hand, then publish ${name} again from an outdir." >&2
            return 1
        fi
    done
    flush_path "${DEST}" || true
    rm -f -- "${JOURNAL}"
}

# For `state`: what a repair would do, without doing it.
emit_journal() {
    local i saved stuck=0
    local -a half=()
    [[ -e "${JOURNAL}" || -L "${JOURNAL}" ]] || return 0
    if [[ ! -f "${JOURNAL}" || -L "${JOURNAL}" ]] || ! load_journal; then
        echo "JOURNAL unreadable"
        return 0
    fi
    for i in "${!J_NAMES[@]}"; do
        tar_verdict "${J_NAMES[${i}]}" "${J_SHA[${i}]}"
        if [[ "${VERDICT}" == "restore" ]]; then
            saved=lost
            if [[ -f "${J_SAVED[${i}]}" && ! -L "${J_SAVED[${i}]}" ]] \
                && [[ "$(sha_of "${J_SAVED[${i}]}" || true)" == "${J_SHA[${i}]}" ]]; then
                saved=ok
            fi
            half+=("HALF ${J_NAMES[${i}]} ${J_SIZE[${i}]} ${J_SHA[${i}]} ${saved} ${J_SAVED[${i}]}")
        elif [[ "${VERDICT}" != "fine" ]]; then
            stuck=1
            half+=("STUCK ${J_NAMES[${i}]}")
        fi
    done
    if [[ "${stuck}" == "1" ]]; then
        echo "JOURNAL stuck ${J_RUN}"
    elif [[ "${#half[@]}" -gt 0 ]]; then
        echo "JOURNAL open ${J_RUN}"
    else
        # Every pack of that publish matches the live list. Only its note is left.
        echo "JOURNAL stale ${J_RUN}"
    fi
    for i in "${!half[@]}"; do
        echo "${half[${i}]}"
    done
}

# ---------------------------------------------------------------- state

emit_state() {
    local f name size sha encoded
    echo "DEST ${DEST}"
    echo "BACKUPS ${BACKUPS}"
    # Every hash is taken on a line of its own: a hash that fails must end
    # this script, not turn into an empty field that reads as "differs".
    if [[ -f "${DEST}/manifest.json" ]]; then
        size="$(size_of "${DEST}/manifest.json")"
        if [[ "${size}" -gt "${MAX_MANIFEST_BYTES}" ]]; then
            echo "MANIFEST toolarge"
        else
            sha="$(sha_of "${DEST}/manifest.json")"
            encoded="$(base64 -w0 < "${DEST}/manifest.json")"
            echo "MANIFEST ${sha} ${encoded}"
        fi
    else
        echo "MANIFEST absent"
    fi
    for f in "${DEST}"/* "${DEST}"/.[!.]* "${DEST}"/..?*; do
        [[ -e "${f}" || -L "${f}" ]] || continue
        name="${f##*/}"
        [[ "${name}" != "manifest.json" && "${f}" != "${JOURNAL}" ]] || continue
        if [[ -f "${f}" && ! -L "${f}" && "${name}" =~ ${TMP_RE} ]]; then
            echo "LEFTOVER ${name}"
        elif [[ -f "${f}" && ! -L "${f}" && "${name}" =~ ${NAME_RE} ]]; then
            size="$(size_of "${f}")"
            sha="$(sha_of "${f}")"
            echo "FILE ${name} ${size} ${sha}"
        else
            printf 'EXTRA %q\n' "${name}"
        fi
    done
    emit_journal
    # What this run saved, with hashes, so the rollback line is built from
    # files that were checked and not from a path that was only printed.
    if [[ -n "${STAMP}" ]]; then
        for f in "${BACKUPS}/packs-manifest-pre-${STAMP}"*.json; do
            [[ -f "${f}" && ! -L "${f}" ]] || continue
            sha="$(sha_of "${f}")"
            echo "SAVEDMANIFEST ${sha} ${f}"
        done
        for f in "${BACKUPS}/packs-tars-pre-${STAMP}"*/*; do
            [[ -f "${f}" && ! -L "${f}" && "${f##*/}" =~ ${NAME_RE} ]] || continue
            sha="$(sha_of "${f}")"
            echo "SAVEDTAR ${f##*/} ${sha} ${f}"
        done
        for f in "${BACKUPS}/packs-set-aside-${STAMP}"*/*; do
            [[ -f "${f}" && ! -L "${f}" && "${f##*/}" =~ ${NAME_RE} ]] || continue
            echo "ASIDE ${f##*/} ${f}"
        done
    fi
    echo "END"
}

state() {
    if [[ "${WAIT}" != "0" ]]; then
        # After a failed publish: wait for a destination half that is still
        # running, so the answer is the final one.
        if take_lock "${WAIT}"; then
            echo "LOCK free"
        else
            echo "LOCK held"
        fi
    elif take_lock 0; then
        flock -u 9
        echo "LOCK free"
    else
        echo "LOCK held"
    fi
    emit_state
}

# Temp files of a publish that was killed. Only called with the lock held, so
# no running publish owns them, and nothing lists them. Only this script's own
# temp names: any other file in the folder is left alone.
sweep_leftovers() {
    local f
    for f in "${DEST}"/.[!.]*; do
        if [[ -f "${f}" && ! -L "${f}" && "${f##*/}" =~ ${TMP_RE} ]]; then
            rm -f -- "${f}"
            echo "CLEANED ${f##*/}"
        fi
    done
    # The same for half-written copies in this script's own backup folders.
    for f in "${BACKUPS}"/.packs-manifest-pre-*.json.tmp "${BACKUPS}"/packs-tars-pre-*/.*.tmp \
        "${BACKUPS}"/packs-set-aside-*/.*.tmp; do
        if [[ -f "${f}" && ! -L "${f}" ]]; then
            rm -f -- "${f}"
            echo "CLEANED ${f##*/}"
        fi
    done
}

# True when no pack of the journal needs anything: the journal is only a note
# that an earlier publish left behind.
journal_is_stale() {
    local i
    for i in "${!J_NAMES[@]}"; do
        tar_verdict "${J_NAMES[${i}]}" "${J_SHA[${i}]}"
        [[ "${VERDICT}" == "fine" ]] || return 1
    done
    return 0
}

repair() {
    take_lock 0 || die "another publish is running on ${DEST}"
    recover_journal || die "the interrupted publish could not be repaired"
    sweep_leftovers
    emit_state
}

# ---------------------------------------------------------------- apply

RUN_ID=""
TAR_BACKUPS=""
SET_ASIDE=""
MANIFEST_TMP=""
NEW_MANIFEST_SHA=""
declare -a TMP_FILES=()
declare -A UP_SIZE=()
declare -A UP_SHA=()
declare -A UP_MODE=()
declare -A OLD_SIZE=()
declare -A OLD_SHA=()
declare -a UP_ORDER=()

# shellcheck disable=SC2317  # reached through the EXIT trap in apply
cleanup() {
    local f
    # Nothing may stop this half way. If kill -9 does, the journal is still
    # there and the next run finishes the job.
    trap '' TERM INT HUP PIPE
    if [[ -e "${JOURNAL}" || -L "${JOURNAL}" ]]; then
        recover_journal >&2 \
            || echo "error: the old file(s) are not back in place. Run the publish again: it repairs this first." >&2
    fi
    for f in "${TMP_FILES[@]}"; do
        rm -f -- "${f}"
    done
}

# shellcheck disable=SC2317  # reached through the signal traps in apply
stopped() {
    echo "error: the destination half was stopped by SIG$1" >&2
    exit "$2"
}

read_stream() {
    local kind a b c d e f tmp ended=0
    while IFS=' ' read -r kind a b c d e f; do
        case "${kind}" in
            PACK)
                [[ "${a}" =~ ${NAME_RE} && "${a}" != manifest.json* ]] || die "bad pack name in the stream"
                [[ "${b}" =~ ^[0-9]+$ && "${c}" =~ ${SHA_RE} ]] || die "bad pack record for ${a}"
                [[ "${d}" == new || "${d}" == repair || "${d}" == update ]] || die "bad pack mode for ${a}"
                [[ -z "${UP_SHA[${a}]:-}" ]] || die "${a} is in the stream twice"
                # What the published list said this pack was when the plan
                # was made. An update needs it; a repair may carry it.
                if [[ "${d}" == update || ( "${d}" == repair && -n "${e}" ) ]]; then
                    [[ "${e}" =~ ^[0-9]+$ && "${f}" =~ ${SHA_RE} ]] || die "bad ${d} record for ${a}"
                    OLD_SIZE["${a}"]="${e}"
                    OLD_SHA["${a}"]="${f}"
                fi
                [[ ! -L "${DEST}/${a}" ]] || die "${a} is a symlink in ${DEST}"
                tmp="${DEST}/.${a}.publish-${STAMP}.tmp"
                TMP_FILES+=("${tmp}")
                head -c "${b}" > "${tmp}"
                [[ "$(size_of "${tmp}")" == "${b}" ]] || die "${a}: size at the destination is not ${b}"
                [[ "$(sha_of "${tmp}")" == "${c}" ]] || die "${a}: sha256 at the destination does not match"
                chmod 0644 -- "${tmp}"
                flush_path "${tmp}"
                UP_SIZE["${a}"]="${b}"
                UP_SHA["${a}"]="${c}"
                UP_MODE["${a}"]="${d}"
                UP_ORDER+=("${a}")
                echo "UPLOADED ${a} ${b} ${c}"
                ;;
            MANIFEST)
                [[ "${a}" =~ ^[0-9]+$ && "${b}" =~ ${SHA_RE} ]] || die "bad manifest record"
                [[ "${a}" -le "${MAX_MANIFEST_BYTES}" ]] || die "new manifest is larger than 64 KiB"
                [[ -z "${NEW_MANIFEST_SHA}" ]] || die "two manifests in the stream"
                TMP_FILES+=("${MANIFEST_TMP}")
                head -c "${a}" > "${MANIFEST_TMP}"
                [[ "$(size_of "${MANIFEST_TMP}")" == "${a}" ]] || die "the new manifest arrived with the wrong size"
                [[ "$(sha_of "${MANIFEST_TMP}")" == "${b}" ]] || die "the new manifest arrived with the wrong sha256"
                chmod 0644 -- "${MANIFEST_TMP}"
                flush_path "${MANIFEST_TMP}"
                NEW_MANIFEST_SHA="${b}"
                ;;
            END)
                ended=1
                break
                ;;
            *)
                die "unknown record in the upload stream"
                ;;
        esac
    done
    [[ "${ended}" == "1" ]] || die "the upload stream ended early; nothing was switched"
    [[ -n "${NEW_MANIFEST_SHA}" ]] || die "the upload stream had no manifest; nothing was switched"
}

# Every pack the new manifest lists must be here with that sha256 and size,
# counting the uploads that are still under their temp names.
check_new_manifest() {
    local line name sha size count=0
    while IFS= read -r line; do
        [[ "${line}" == "[" || "${line}" == "]" ]] && continue
        [[ "${line}" =~ ${ENTRY_RE} ]] || die "the new manifest has a line this script did not write"
        name="${BASH_REMATCH[1]}"
        sha="${BASH_REMATCH[3]}"
        size="${BASH_REMATCH[4]}"
        count=$((count + 1))
        if [[ -n "${UP_SHA[${name}]:-}" ]]; then
            [[ "${UP_SHA[${name}]}" == "${sha}" && "${UP_SIZE[${name}]}" == "${size}" ]] \
                || die "${name}: the upload does not match the new manifest"
            continue
        fi
        [[ -f "${DEST}/${name}" && ! -L "${DEST}/${name}" ]] \
            || die "${name} is listed in the new manifest but is not at the destination"
        [[ "$(size_of "${DEST}/${name}")" == "${size}" ]] \
            || die "${name}: size at the destination does not match the new manifest"
        [[ "$(sha_of "${DEST}/${name}")" == "${sha}" ]] \
            || die "${name}: sha256 at the destination does not match the new manifest"
    done < "${MANIFEST_TMP}"
    for name in "${UP_ORDER[@]}"; do
        grep -qF "\"name\":\"${name}\"" "${MANIFEST_TMP}" \
            || die "${name} was uploaded but the new manifest does not list it"
    done
    echo "CHECKED ${count}"
}

# A folder without manifest.json is a first publish only when the operator
# said so, and only when nothing else is in it. A folder that holds other
# files is the wrong folder or a damaged one, and this script does not guess.
check_first_publish() {
    local f name
    [[ "${FIRST_PUBLISH}" == "1" ]] \
        || die "there is no manifest.json in ${DEST}. That is right only for a first publish (--first-publish); nothing was switched"
    for f in "${DEST}"/* "${DEST}"/.[!.]* "${DEST}"/..?*; do
        [[ -e "${f}" || -L "${f}" ]] || continue
        name="${f##*/}"
        if [[ -f "${f}" && ! -L "${f}" ]]; then
            [[ "${name}" =~ ${TMP_RE} || -n "${UP_SHA[${name}]:-}" ]] && continue
        fi
        die "$(printf '%q' "${name}") is in ${DEST} and is not a pack of this publish. A first publish needs a folder with nothing else in it; nothing was switched"
    done
}

write_journal() {
    local name tmp="${DEST}/.publish-journal.publish-${STAMP}.tmp"
    TMP_FILES+=("${tmp}")
    {
        echo "JOURNAL ${RUN_ID}"
        for name in "${UP_ORDER[@]}"; do
            [[ "${UP_MODE[${name}]}" == update ]] || continue
            echo "TAR ${name} ${OLD_SIZE[${name}]} ${OLD_SHA[${name}]} ${TAR_BACKUPS}/${name}"
        done
        echo "END"
    } > "${tmp}"
    flush_path "${tmp}"
    mv -fT -- "${tmp}" "${JOURNAL}"
    flush_path "${DEST}"
}

apply() {
    local name n tmp saved target updates=0 asides=0
    [[ "${STAMP}" =~ ${STAMP_RE} ]] || die "bad --stamp"
    [[ -n "${EXPECT_OLD}" ]] || die "missing --expect-old"
    [[ -w "${DEST}" ]] || die "destination is not writable: ${DEST}"
    take_lock 0 || die "another publish is running on ${DEST}"

    if [[ -e "${JOURNAL}" || -L "${JOURNAL}" ]]; then
        if [[ -f "${JOURNAL}" && ! -L "${JOURNAL}" ]] && load_journal && journal_is_stale; then
            # Every pack of that publish matches the live list. Only its note is left.
            rm -f -- "${JOURNAL}"
            echo "CLEANED ${JOURNAL##*/}"
        else
            die "an interrupted publish has to be repaired first; run the publish again"
        fi
    fi
    sweep_leftovers
    # Two publishes in one second must not share a backup name.
    RUN_ID="${STAMP}"
    n=1
    while [[ -e "${BACKUPS}/packs-manifest-pre-${RUN_ID}.json" || -e "${BACKUPS}/packs-tars-pre-${RUN_ID}" \
        || -e "${BACKUPS}/packs-set-aside-${RUN_ID}" ]]; do
        n=$((n + 1))
        RUN_ID="${STAMP}-${n}"
    done
    TAR_BACKUPS="${BACKUPS}/packs-tars-pre-${RUN_ID}"
    SET_ASIDE="${BACKUPS}/packs-set-aside-${RUN_ID}"
    MANIFEST_TMP="${DEST}/.manifest.json.publish-${STAMP}.tmp"
    trap cleanup EXIT
    trap 'stopped TERM 143' TERM
    trap 'stopped INT 130' INT
    trap 'stopped HUP 129' HUP
    # A closed connection must end in an error this script handles, not in a
    # silent death half way through a line of output.
    trap '' PIPE

    read_stream
    [[ "$(manifest_sha)" == "${EXPECT_OLD}" ]] \
        || die "manifest.json changed at the destination since it was read; run again"
    if [[ "${EXPECT_OLD}" == "absent" ]]; then
        check_first_publish
    elif [[ "${FIRST_PUBLISH}" == "1" ]]; then
        die "--first-publish, but ${DEST} already has a manifest.json; nothing was switched"
    fi
    check_new_manifest

    # ---- save what the switch replaces. The slow part. Copies only: nothing
    # that is live changes here. A run that fails after this leaves its copies
    # in the backups folder.
    if [[ ! -d "${BACKUPS}" ]]; then
        mkdir -p -- "${BACKUPS}"
        chmod 0700 -- "${BACKUPS}"
    fi
    if [[ -f "${DEST}/manifest.json" ]]; then
        saved="${BACKUPS}/packs-manifest-pre-${RUN_ID}.json"
        tmp="${BACKUPS}/.packs-manifest-pre-${RUN_ID}.json.tmp"
        TMP_FILES+=("${tmp}")
        cp -p -- "${DEST}/manifest.json" "${tmp}"
        [[ "$(sha_of "${tmp}")" == "${EXPECT_OLD}" ]] || die "the copy of the old manifest is not the old manifest"
        flush_path "${tmp}"
        mv -T -- "${tmp}" "${saved}"
        echo "BACKUP ${saved}"
    else
        echo "BACKUP none"
    fi
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" == update ]] || continue
        # The file that is saved as "the old tar" must be the one the old
        # manifest names. After a damaged or half-done publish it is not.
        if ! file_is "${name}" "${OLD_SHA[${name}]}" || [[ "$(size_of "${DEST}/${name}")" != "${OLD_SIZE[${name}]}" ]]; then
            die "${name} at the destination is not the file the published list names; it was not saved and nothing was switched"
        fi
        if [[ "${updates}" == "0" ]]; then
            mkdir -m 0700 -- "${TAR_BACKUPS}"
        fi
        updates=$((updates + 1))
        saved="${TAR_BACKUPS}/${name}"
        tmp="${TAR_BACKUPS}/.${name}.tmp"
        TMP_FILES+=("${tmp}")
        cp -p -- "${DEST}/${name}" "${tmp}"
        [[ "$(sha_of "${tmp}")" == "${OLD_SHA[${name}]}" ]] || die "the saved copy of ${name} is not the old file"
        flush_path "${tmp}"
        mv -T -- "${tmp}" "${saved}"
        echo "SAVED ${name} ${saved}"
    done
    if [[ "${updates}" != "0" ]]; then
        flush_path "${TAR_BACKUPS}"
    fi
    # A file that already has the name of a new or repaired pack: an unlisted
    # tar of an earlier publish, or a damaged one. It is about to be replaced,
    # so a copy of it is set aside first. Nothing is overwritten without one.
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" != update ]] || continue
        target="${DEST}/${name}"
        [[ -e "${target}" || -L "${target}" ]] || continue
        [[ -f "${target}" && ! -L "${target}" ]] \
            || die "${name} at the destination is not a plain file; nothing was switched"
        # The plan called this pack damaged. If it is the published file again
        # (another run repaired it in between), the plan is out of date.
        if [[ -n "${OLD_SHA[${name}]:-}" && "${OLD_SHA[${name}]}" != "${UP_SHA[${name}]}" ]] \
            && file_is "${name}" "${OLD_SHA[${name}]}"; then
            die "${name} at the destination is the published file again, so the plan for it is out of date; nothing was switched. Run again."
        fi
        if file_is "${name}" "${UP_SHA[${name}]}"; then
            continue
        fi
        if [[ "${asides}" == "0" ]]; then
            mkdir -m 0700 -- "${SET_ASIDE}"
        fi
        asides=$((asides + 1))
        tmp="${SET_ASIDE}/.${name}.tmp"
        TMP_FILES+=("${tmp}")
        cp -p -- "${target}" "${tmp}"
        flush_path "${tmp}"
        mv -T -- "${tmp}" "${SET_ASIDE}/${name}"
        echo "ASIDE ${name} ${SET_ASIDE}/${name}"
    done

    # ---- new names. The live manifest does not list them, so nobody is
    # served them yet. A repaired pack was already broken; now it is whole.
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" != update ]] || continue
        mv -fT -- "${DEST}/.${name}.publish-${STAMP}.tmp" "${DEST}/${name}"
    done
    flush_path "${DEST}"

    # ---- the switch. For an update the tar goes first and the manifest
    # last: in between, the old list is live with the new bytes under it. The
    # server answers 404 for that pack when the size differs, and a tracker
    # refuses bytes whose sha256 is not the listed one, so nobody installs the
    # wrong pack. A tracker that already has the old version asks for nothing.
    # TERM, INT and HUP are ignored for these renames: one that arrives here
    # is dropped, and the publish goes through. kill -9 and power loss cannot
    # be ignored; the journal covers them.
    if [[ "${updates}" != "0" ]]; then
        write_journal
    fi
    trap '' TERM INT HUP
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" == update ]] || continue
        mv -fT -- "${DEST}/.${name}.publish-${STAMP}.tmp" "${DEST}/${name}"
    done
    mv -fT -- "${MANIFEST_TMP}" "${DEST}/manifest.json"
    trap 'stopped TERM 143' TERM
    trap 'stopped INT 130' INT
    trap 'stopped HUP 129' HUP

    # The new list is live. From here on nothing is undone: cleanup reads the
    # live manifest and sees that for itself.
    flush_path "${DEST}" || echo "warning: could not flush ${DEST} after the switch" >&2
    rm -f -- "${JOURNAL}"
    echo "COMMITTED"
    emit_state
}

main() {
    local verb="${1:-}"
    [[ "$#" -gt 0 ]] && shift
    preflight
    parse_where "$@"
    case "${verb}" in
        state) state ;;
        repair) repair ;;
        apply) apply ;;
        *) die "unknown verb: ${verb}" ;;
    esac
}

# One line: bash must not look for more script after main has read the stream.
main "$@"; exit $?
