# Sourced by the stat-tracker installers. Rewrites systemd user units so
# ExecStart is the absolute installed path (PREFIX), not the template's
# %h/.local/bin placeholder.
#
# Do not execute this file.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    echo "source this file; do not execute it" >&2
    exit 1
fi

# Absolute path for a file that may not exist yet. Relative paths are
# resolved against the current working directory (same rule as the
# desktop-entry writer).
absolute_install_path() {
    local p="$1" dir base
    if [[ "$p" == /* ]]; then
        printf '%s' "$p"
        return 0
    fi
    base="$(basename "$p")"
    dir="$(dirname "$p")"
    mkdir -p "$dir"
    dir="$(cd "$dir" && pwd)"
    printf '%s/%s' "$dir" "$base"
}

# systemd ExecStart token. Safe paths stay bare; anything else is quoted
# so a PREFIX with spaces is one argument, not a split command.
systemd_exec_token() {
    local p="$1" escaped
    if [[ "$p" =~ ^[A-Za-z0-9._@/+:-]+$ ]]; then
        printf '%s' "$p"
        return 0
    fi
    escaped="${p//\\/\\\\}"
    escaped="${escaped//\"/\\\"}"
    printf '"%s"' "$escaped"
}

# Copy a unit template, replacing every ExecStart= line with `exec_path`.
install_systemd_unit() {
    local src="$1" dest="$2" exec_path="$3"
    local token tmp
    token="$(systemd_exec_token "$exec_path")"
    mkdir -p "$(dirname "$dest")"
    tmp="$(mktemp "$(dirname "$dest")/.unit.XXXXXX")"
    {
        while IFS= read -r line || [[ -n "$line" ]]; do
            case "$line" in
                ExecStart=*) printf 'ExecStart=%s\n' "$token" ;;
                *) printf '%s\n' "$line" ;;
            esac
        done < "$src"
    } > "$tmp"
    chmod 644 "$tmp"
    mv -f "$tmp" "$dest"
}

# Install the daemon unit, the session-env oneshot, and the helper script.
# Paths must already be absolute. Returns non-zero if a template is missing.
install_user_units() {
    local assets_dir="$1"
    local systemd_dir="$2"
    local daemon_bin="$3"
    local helper_src="$4"
    local helper_dest="$5"
    local unit="scuffed-stat-tracker.service"
    local session_unit="scuffed-stat-tracker-session.service"

    if [[ ! -f "$helper_src" ]]; then
        echo "missing session helper: $helper_src" >&2
        return 1
    fi
    if [[ ! -f "$assets_dir/$unit" || ! -f "$assets_dir/$session_unit" ]]; then
        echo "missing systemd unit templates in $assets_dir" >&2
        return 1
    fi

    mkdir -p "$(dirname "$helper_dest")" "$systemd_dir"
    install -m755 "$helper_src" "$helper_dest"
    install_systemd_unit "$assets_dir/$unit" "$systemd_dir/$unit" "$daemon_bin"
    install_systemd_unit \
        "$assets_dir/$session_unit" "$systemd_dir/$session_unit" "$helper_dest"
}
