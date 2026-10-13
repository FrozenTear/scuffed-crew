#!/usr/bin/env bash
# Destination side of scripts/publish-packs.sh.
#
# This file runs on the machine that holds PACKS_DIR. publish-packs.sh sends
# it down a pipe and runs it with `bash -s -- <verb> ...`, on this machine or
# through ssh, so nothing has to be installed on the server. The bytes that
# follow this script on stdin are the upload stream (see `apply`). bash reads
# a script from a pipe one byte at a time, so the stream stays intact.
#
# It needs bash and coreutils. No jq, no python. podman only for --volume.
# It never unpacks a tar.
#
# Verbs:
#   state   Print what is published: the folder, the manifest, every file.
#   apply   Read the upload stream, check it, then switch manifest.json.
#
# Where:
#   --dir PATH       a plain directory
#   --volume NAME    a podman volume; `auto` picks the one named *packs-data
#   --backups PATH   where old manifests go (default: ~/scuffed-crew-backups)
#
# shellcheck shell=bash

set -euo pipefail
umask 022

MAX_MANIFEST_BYTES=65536
NAME_RE='^[A-Za-z0-9][A-Za-z0-9._-]*$'
SHA_RE='^[0-9a-f]{64}$'
# One entry per line, exactly as publish-packs.sh writes it.
ENTRY_RE='^  \{"name":"([A-Za-z0-9][A-Za-z0-9._-]*)","version":"([A-Za-z0-9._-]+)","sha256":"([0-9a-f]{64})","size":([0-9]+)\},?$'

DEST=""
BACKUPS=""

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

# fsync one path when this coreutils can, else flush everything.
flush_path() {
    sync -- "$1" 2>/dev/null || sync 2>/dev/null || true
}

# Mountpoint of the packs volume. backup.sh reads other volumes the same way.
# A publish writes, so two candidate volumes are refused instead of guessed.
resolve_volume() {
    local want="$1" name path
    local -a found=()
    command -v podman >/dev/null 2>&1 || die "podman is required to find the packs volume"
    if [[ "${want}" != "auto" ]]; then
        found=("${want}")
    else
        while IFS= read -r name; do
            case "${name}" in
                packs-data | *_packs-data) found+=("${name}") ;;
            esac
        done < <(podman volume ls --format '{{.Name}}' 2>/dev/null || true)
    fi
    if [[ "${#found[@]}" -eq 0 ]]; then
        die "no podman volume named *packs-data here; start the stack once or pass PACKS_VOLUME"
    fi
    if [[ "${#found[@]}" -gt 1 ]]; then
        die "more than one packs volume (${found[*]}); set PACKS_VOLUME to the one compose mounts"
    fi
    path="$(podman volume inspect "${found[0]}" --format '{{.Mountpoint}}' 2>/dev/null || true)"
    [[ -n "${path}" && -d "${path}" ]] || die "podman volume ${found[0]} has no readable mountpoint"
    echo "${path}"
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
    BACKUPS="${BACKUPS:-${HOME}/scuffed-crew-backups}"
    [[ ! -L "${DEST}/manifest.json" ]] || die "manifest.json is a symlink in ${DEST}"
}

manifest_sha() {
    if [[ -f "${DEST}/manifest.json" ]]; then
        sha_of "${DEST}/manifest.json"
    else
        echo absent
    fi
}

emit_state() {
    local f name size
    echo "DEST ${DEST}"
    echo "BACKUPS ${BACKUPS}"
    if [[ -f "${DEST}/manifest.json" ]]; then
        size="$(size_of "${DEST}/manifest.json")"
        if [[ "${size}" -gt "${MAX_MANIFEST_BYTES}" ]]; then
            echo "MANIFEST toolarge"
        else
            echo "MANIFEST $(sha_of "${DEST}/manifest.json") $(base64 -w0 < "${DEST}/manifest.json")"
        fi
    else
        echo "MANIFEST absent"
    fi
    for f in "${DEST}"/*; do
        [[ -f "${f}" && ! -L "${f}" ]] || continue
        name="${f##*/}"
        [[ "${name}" =~ ${NAME_RE} ]] || continue
        [[ "${name}" != "manifest.json" && "${name}" != "manifest.json.new" ]] || continue
        echo "FILE ${name} $(size_of "${f}") $(sha_of "${f}")"
    done
    echo "END"
}

# ---------------------------------------------------------------- apply

STAMP=""
EXPECT_OLD=""
COMMITTED=0
TAR_BACKUPS=""
declare -a TMP_FILES=()
declare -a REPLACED_DONE=()
declare -A UP_SIZE=()
declare -A UP_SHA=()
declare -A UP_MODE=()
declare -a UP_ORDER=()
declare -a DROPS=()

# shellcheck disable=SC2317  # reached through the EXIT trap in apply
cleanup() {
    local name f
    if [[ "${COMMITTED}" != "1" ]]; then
        # Put back any tar that was swapped for a replacement.
        for name in "${REPLACED_DONE[@]}"; do
            if cp -p -- "${TAR_BACKUPS}/${name}" "${DEST}/.${name}.restore-${STAMP}.tmp" \
                && mv -fT -- "${DEST}/.${name}.restore-${STAMP}.tmp" "${DEST}/${name}"; then
                echo "restored ${name} after the failed publish" >&2
            else
                echo "error: could not restore ${name}; the old copy is ${TAR_BACKUPS}/${name}" >&2
            fi
        done
        rm -f -- "${DEST}/manifest.json.new"
    fi
    for f in "${TMP_FILES[@]}"; do
        rm -f -- "${f}"
    done
}

read_stream() {
    local kind a b c d tmp ended=0 have_manifest=0
    while IFS=' ' read -r kind a b c d; do
        case "${kind}" in
            PACK)
                [[ "${a}" =~ ${NAME_RE} && "${a}" != manifest.json* ]] || die "bad pack name in the stream"
                [[ "${b}" =~ ^[0-9]+$ && "${c}" =~ ${SHA_RE} ]] || die "bad pack record for ${a}"
                [[ "${d}" == new || "${d}" == repair || "${d}" == replace ]] || die "bad pack mode for ${a}"
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
                head -c "${a}" > "${DEST}/manifest.json.new"
                [[ "$(size_of "${DEST}/manifest.json.new")" == "${a}" ]] || die "manifest.json.new: wrong size"
                [[ "$(sha_of "${DEST}/manifest.json.new")" == "${b}" ]] || die "manifest.json.new: wrong sha256"
                chmod 0644 -- "${DEST}/manifest.json.new"
                flush_path "${DEST}/manifest.json.new"
                have_manifest=1
                ;;
            DROP)
                [[ "${a}" =~ ${NAME_RE} && "${a}" != manifest.json* ]] || die "bad drop record"
                DROPS+=("${a}")
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
    [[ "${have_manifest}" == "1" ]] || die "the upload stream had no manifest; nothing was switched"
}

# Every pack the new manifest lists must be here with that sha256 and size,
# counting the uploads that are still under their temp names.
check_new_manifest() {
    local line name sha size count=0
    while IFS= read -r line; do
        [[ "${line}" == "[" || "${line}" == "]" ]] && continue
        [[ "${line}" =~ ${ENTRY_RE} ]] || die "manifest.json.new has a line this script did not write"
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
    done < "${DEST}/manifest.json.new"
    for name in "${UP_ORDER[@]}"; do
        grep -qF "\"name\":\"${name}\"" "${DEST}/manifest.json.new" \
            || die "${name} was uploaded but the new manifest does not list it"
    done
    for name in "${DROPS[@]}"; do
        if grep -qF "\"name\":\"${name}\"" "${DEST}/manifest.json.new"; then
            die "refusing to drop ${name}: the new manifest still lists it"
        fi
    done
    echo "CHECKED ${count}"
}

apply() {
    local name backup f run_id n
    [[ "${STAMP}" =~ ^[0-9]{8}T[0-9]{6}Z$ ]] || die "bad --stamp"
    [[ -n "${EXPECT_OLD}" ]] || die "missing --expect-old"
    [[ -w "${DEST}" ]] || die "destination is not writable: ${DEST}"
    case "${BACKUPS}/" in
        "${DEST}/"*) die "the backups folder must be outside ${DEST}" ;;
    esac
    if command -v flock >/dev/null 2>&1; then
        exec 9< "${DEST}"
        flock -n 9 || die "another publish is running on ${DEST}"
    fi
    [[ ! -L "${DEST}/manifest.json.new" ]] || die "manifest.json.new is a symlink in ${DEST}"
    # Leftovers from a publish that was killed. Nothing lists them.
    for f in "${DEST}"/.*.publish-*.tmp "${DEST}"/.*.restore-*.tmp "${DEST}/manifest.json.new"; do
        if [[ -f "${f}" || -L "${f}" ]]; then
            rm -f -- "${f}"
            echo "CLEANED ${f##*/}"
        fi
    done
    # Two publishes in one second must not share a backup name.
    run_id="${STAMP}"
    n=1
    while [[ -e "${BACKUPS}/packs-manifest-pre-${run_id}.json" || -e "${BACKUPS}/packs-tars-pre-${run_id}" ]]; do
        n=$((n + 1))
        run_id="${STAMP}-${n}"
    done
    TAR_BACKUPS="${BACKUPS}/packs-tars-pre-${run_id}"
    trap cleanup EXIT

    read_stream
    [[ "$(manifest_sha)" == "${EXPECT_OLD}" ]] \
        || die "manifest.json changed at the destination since it was read; run again"
    check_new_manifest

    # New names first. The live manifest does not list them yet.
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" != replace ]] || continue
        mv -fT -- "${DEST}/.${name}.publish-${STAMP}.tmp" "${DEST}/${name}"
    done

    mkdir -p -- "${BACKUPS}"
    chmod 0700 -- "${BACKUPS}"
    if [[ -f "${DEST}/manifest.json" ]]; then
        backup="${BACKUPS}/packs-manifest-pre-${run_id}.json"
        [[ ! -e "${backup}" ]] || die "backup already exists: ${backup}"
        cp -p -- "${DEST}/manifest.json" "${backup}"
        flush_path "${backup}"
        echo "BACKUP ${backup}"
    else
        echo "BACKUP none"
    fi

    # A replaced pack keeps its name, so its file and the manifest cannot
    # switch in one step. Save every old file first (the slow part), then do
    # the renames back to back. cleanup puts the old files back on a failure.
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" == replace && -f "${DEST}/${name}" ]] || continue
        mkdir -p -- "${TAR_BACKUPS}"
        cp -p -- "${DEST}/${name}" "${TAR_BACKUPS}/${name}"
        flush_path "${TAR_BACKUPS}/${name}"
        echo "TARS ${TAR_BACKUPS}"
    done
    for name in "${UP_ORDER[@]}"; do
        [[ "${UP_MODE[${name}]}" == replace ]] || continue
        [[ ! -f "${TAR_BACKUPS}/${name}" ]] || REPLACED_DONE+=("${name}")
        mv -fT -- "${DEST}/.${name}.publish-${STAMP}.tmp" "${DEST}/${name}"
    done

    mv -fT -- "${DEST}/manifest.json.new" "${DEST}/manifest.json"
    COMMITTED=1
    flush_path "${DEST}"
    echo "COMMITTED"

    # Only now, when the live manifest no longer lists them.
    for name in "${DROPS[@]}"; do
        [[ -f "${DEST}/${name}" && ! -L "${DEST}/${name}" ]] || continue
        mkdir -p -- "${TAR_BACKUPS}"
        if mv -fT -- "${DEST}/${name}" "${TAR_BACKUPS}/${name}"; then
            echo "TARS ${TAR_BACKUPS}"
        else
            echo "warning: could not move the dropped ${name}; it is no longer listed" >&2
        fi
    done

    emit_state
}

main() {
    local verb="${1:-}"
    [[ "$#" -gt 0 ]] && shift
    parse_where "$@"
    case "${verb}" in
        state) emit_state ;;
        apply) apply ;;
        *) die "unknown verb: ${verb}" ;;
    esac
}

# One line: bash must not look for more script after main has read the stream.
main "$@"; exit $?
