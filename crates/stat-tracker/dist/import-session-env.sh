#!/usr/bin/env bash
# Push the graphical session's display variables into two places the
# stat-tracker user unit can actually see:
#
#   1. ~/.config/scuffed-stat-tracker/session.env
#      Loaded by scuffed-stat-tracker.service (EnvironmentFile=) every start.
#   2. The systemd user manager (`systemctl --user import-environment`),
#      so this login's other user units see the same values.
#
# Why both, and why not only an install-time import:
# `graphical-session.target` does not import WAYLAND_DISPLAY or DISPLAY.
# GNOME and KDE do that themselves; Sway and Hyprland do not. An import
# done at install time dies when the user manager restarts (logout, reboot,
# daemon-reexec), and the socket name (wayland-1 vs wayland-2) changes
# when the compositor does. This script runs as a oneshot before every
# daemon start and re-reads the compositor.
#
# Resolution order for each of WAYLAND_DISPLAY, DISPLAY, XDG_CURRENT_DESKTOP,
# XDG_SESSION_TYPE:
#   1. Environ of a same-user compositor process (/proc/<pid>/environ).
#      Sway and Hyprland are preferred over Xorg/Xwayland so a nested X
#      server cannot hide the Wayland socket. A stale value inherited from
#      the user manager does not win when a compositor is visible.
#   2. This process's own environment, for a variable no compositor had.
#      That is the install-from-a-graphical-terminal path, and the path
#      used when /proc cannot be read.
#
# Values with whitespace, newlines, or shell metacharacters are dropped.
# Nothing is written into the unit file.
#
# Exit 0 when there is nothing safe to import or when systemctl cannot
# reach the user bus — the daemon must still start. The EnvironmentFile
# is written first; import-environment is best-effort and timed out so a
# stuck user bus cannot wedge the oneshot the daemon is ordered After=.
#
# Overrides (tests): SCUFFED_PROC_ROOT, SCUFFED_SYSTEMCTL, SCUFFED_SESSION_ENV_FILE.
set -euo pipefail

PROC_ROOT="${SCUFFED_PROC_ROOT:-/proc}"
SYSTEMCTL_BIN="${SCUFFED_SYSTEMCTL:-systemctl}"
SESSION_ENV_FILE="${SCUFFED_SESSION_ENV_FILE:-${HOME:-}/.config/scuffed-stat-tracker/session.env}"

SESSION_VARS=(WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE)

# Highest priority first. Sway and Hyprland lead: they are the desktops
# that do not run `systemctl --user import-environment` on login.
# Xwayland is last so it only fills DISPLAY when the compositor has none.
COMPOSITOR_COMMS=(
    sway
    Hyprland
    hyprland
    labwc
    river
    wayfire
    niri
    cage
    gamescope
    gnome-shell
    kwin_wayland
    cosmic-comp
    weston
    mutter
    Xorg
    X
    Xwayland
)

log() { echo "[session-env] $*" >&2; }

valid_session_value() {
    local key="$1" val="$2"
    [[ -n "$val" ]] || return 1
    [[ ${#val} -le 256 ]] || return 1
    # One token. Reject newlines, quotes, spaces, and anything systemctl
    # or a unit file could split or expand. Key-specific patterns below
    # narrow this further (DISPLAY must look like :0, and so on).
    case "$val" in
        *[!A-Za-z0-9._@/+:-]*) return 1 ;;
    esac
    case "$key" in
        WAYLAND_DISPLAY)
            [[ "$val" =~ ^wayland-[0-9]+$ || "$val" =~ ^/[-._A-Za-z0-9/]+$ ]]
            ;;
        DISPLAY)
            [[ "$val" =~ ^(:[0-9]+(\.[0-9]+)?|[A-Za-z0-9._-]+:[0-9]+(\.[0-9]+)?|unix:[0-9]+(\.[0-9]+)?)$ ]]
            ;;
        XDG_CURRENT_DESKTOP)
            [[ "$val" =~ ^[A-Za-z0-9][A-Za-z0-9_.:-]*$ ]]
            ;;
        XDG_SESSION_TYPE)
            [[ "$val" == "wayland" || "$val" == "x11" || "$val" == "tty" || "$val" == "mir" || "$val" == "unspecified" ]]
            ;;
        *)
            return 1
            ;;
    esac
}

# Print valid KEY=VAL lines from one NUL-separated environ file.
read_environ_file() {
    local file="$1" kv key val
    [[ -r "$file" ]] || return 0
    while IFS= read -r -d '' kv || [[ -n "${kv:-}" ]]; do
        [[ -n "$kv" ]] || continue
        key="${kv%%=*}"
        [[ "$key" == "$kv" ]] && continue
        val="${kv#*=}"
        case "$key" in
            WAYLAND_DISPLAY|DISPLAY|XDG_CURRENT_DESKTOP|XDG_SESSION_TYPE)
                if valid_session_value "$key" "$val"; then
                    printf '%s=%s\n' "$key" "$val"
                else
                    log "ignoring invalid $key from $file"
                fi
                ;;
        esac
    done < "$file"
}

same_user_pid() {
    local pid="$1" owner self
    self="$(id -u)"
    owner="$(stat -c %u "$PROC_ROOT/$pid" 2>/dev/null)" || return 1
    [[ "$owner" == "$self" ]]
}

declare -A FOUND=()
declare -A FOUND_NOTE=()

scan_compositors() {
    local i comm pid_dir pid got line k v
    # Low priority first so a later (higher) match overwrites a key it has
    # and leaves keys it does not (Xwayland's DISPLAY under Sway).
    for ((i = ${#COMPOSITOR_COMMS[@]} - 1; i >= 0; i--)); do
        comm="${COMPOSITOR_COMMS[$i]}"
        for pid_dir in "$PROC_ROOT"/[0-9]*; do
            [[ -d "$pid_dir" ]] || continue
            pid="${pid_dir##*/}"
            [[ "$pid" =~ ^[0-9]+$ ]] || continue
            same_user_pid "$pid" || continue
            got="$(tr -d '\n\0' < "$pid_dir/comm" 2>/dev/null || true)"
            [[ "$got" == "$comm" ]] || continue
            while IFS= read -r line; do
                [[ -n "$line" ]] || continue
                k="${line%%=*}"
                v="${line#*=}"
                FOUND["$k"]="$v"
                FOUND_NOTE["$k"]="compositor pid $pid ($comm)"
            done < <(read_environ_file "$pid_dir/environ")
        done
    done
}

fill_from_process_env() {
    local key val
    for key in "${SESSION_VARS[@]}"; do
        [[ -n "${FOUND[$key]:-}" ]] && continue
        val="${!key:-}"
        if [[ -z "$val" ]]; then
            continue
        fi
        if valid_session_value "$key" "$val"; then
            FOUND["$key"]="$val"
            FOUND_NOTE["$key"]="process environment"
        else
            log "ignoring invalid $key from process environment"
        fi
    done
}

write_session_env_file() {
    local file="$1" dir tmp key
    if [[ -z "$file" || "$file" == "/.config/scuffed-stat-tracker/session.env" ]]; then
        log "HOME is unset; not writing session.env"
        return 0
    fi
    dir="$(dirname "$file")"
    mkdir -p "$dir"
    tmp="$(mktemp "$dir/.session.env.XXXXXX")"
    {
        echo "# Generated by import-session-env.sh for scuffed-stat-tracker.service."
        echo "# Rewritten on each daemon start. Do not edit."
        for key in "${SESSION_VARS[@]}"; do
            if [[ -n "${FOUND[$key]:-}" ]]; then
                printf '%s=%s\n' "$key" "${FOUND[$key]}"
            fi
        done
    } > "$tmp"
    chmod 600 "$tmp"
    mv -f "$tmp" "$file"
}

import_into_user_manager() {
    local -a args=()
    local key
    for key in "${SESSION_VARS[@]}"; do
        if [[ -n "${FOUND[$key]:-}" ]]; then
            export "$key=${FOUND[$key]}"
            args+=("$key")
        fi
    done
    if [[ ${#args[@]} -eq 0 ]]; then
        log "no session variables to import"
        log "From a terminal in the graphical session, run:"
        log "  systemctl --user import-environment ${SESSION_VARS[*]}"
        log "Or start the daemon after Sway/Hyprland is up; this oneshot retries."
        return 0
    fi
    if [[ ! -x "$SYSTEMCTL_BIN" ]] && ! command -v "$SYSTEMCTL_BIN" >/dev/null 2>&1; then
        log "systemctl not found ($SYSTEMCTL_BIN); session.env is still written"
        return 0
    fi
    # timeout so a user-manager D-Bus stall cannot block the daemon (After=).
    if command -v timeout >/dev/null 2>&1; then
        if ! timeout 5 "$SYSTEMCTL_BIN" --user import-environment "${args[@]}"; then
            log "systemctl --user import-environment failed; daemon still reads session.env"
            return 0
        fi
    elif ! "$SYSTEMCTL_BIN" --user import-environment "${args[@]}"; then
        log "systemctl --user import-environment failed; daemon still reads session.env"
        return 0
    fi
    log "imported into user manager: ${args[*]}"
}

main() {
    local key missing=()
    scan_compositors
    fill_from_process_env
    for key in "${SESSION_VARS[@]}"; do
        if [[ -n "${FOUND[$key]:-}" ]]; then
            log "$key=${FOUND[$key]} (${FOUND_NOTE[$key]})"
        else
            missing+=("$key")
        fi
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        log "not found: ${missing[*]}"
    fi
    write_session_env_file "$SESSION_ENV_FILE"
    log "wrote $SESSION_ENV_FILE"
    import_into_user_manager
}

main "$@"
