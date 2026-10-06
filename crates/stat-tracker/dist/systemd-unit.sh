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
    # Sets DATA_DIR_DROPIN to the drop-in path when a custom data_dir needs one.
    install_data_dir_dropin "$systemd_dir" "${HOME:-}"
}

# Absolute data_dir from config.toml, or empty. Relative paths are skipped:
# the unit cannot name them.
data_dir_from_config() {
    local cfg="$1"
    [[ -f "$cfg" ]] || return 1
    python3 - "$cfg" <<'PY'
import sys
try:
    import tomllib
except ImportError:
    sys.exit(2)
with open(sys.argv[1], "rb") as fh:
    cfg = tomllib.load(fh)
raw = cfg.get("data_dir")
if isinstance(raw, str) and raw.startswith("/") and "\n" not in raw and "\0" not in raw:
    while len(raw) > 1 and raw.endswith("/"):
        raw = raw[:-1]
    print(raw)
    sys.exit(0)
sys.exit(1)
PY
}

# True when the unit's ReadWritePaths already covers this absolute path.
data_dir_covered_by_unit() {
    local path="$1" home="$2" runtime="${XDG_RUNTIME_DIR:-}" root
    for root in \
        "$home/.local/share/scuffed-stat-tracker" \
        "$home/.config/scuffed-stat-tracker" \
        ${runtime:+"$runtime"}
    do
        [[ -n "$root" ]] || continue
        if [[ "$path" == "$root" || "$path" == "$root"/* ]]; then
            return 0
        fi
    done
    return 1
}

# systemd ReadWritePaths token. Keep in step with sandbox::read_write_paths_token.
systemd_read_write_token() {
    local p="$1"
    p="${p//%/%%}"
    if [[ "$p" == *[[:space:]\\'"']* ]]; then
        p="${p//\\/\\\\}"
        p="${p//\"/\\\"}"
        printf '"-%s"' "$p"
    else
        printf -- '-%s' "$p"
    fi
}

# Write or remove ~/.config/systemd/user/scuffed-stat-tracker.service.d/data-dir.conf
# from the current config. Only a file this installer wrote (marker line) is
# replaced or removed. DATA_DIR_DROPIN is the path when a drop-in is left in place.
install_data_dir_dropin() {
    local systemd_dir="$1" home="$2"
    local cfg="${SCUFFED_CONFIG_FILE:-$home/.config/scuffed-stat-tracker/config.toml}"
    local dropdir="$systemd_dir/scuffed-stat-tracker.service.d"
    local drop="$dropdir/data-dir.conf"
    local data_dir token
    DATA_DIR_DROPIN=""
    if [[ -z "$home" ]]; then
        return 0
    fi
    if [[ -f "$drop" ]] && ! grep -q 'scuffed-stat-tracker data_dir drop-in' "$drop"; then
        echo "leaving $drop alone (not written by this installer)" >&2
        return 0
    fi
    if ! data_dir="$(data_dir_from_config "$cfg")" || data_dir_covered_by_unit "$data_dir" "$home"; then
        rm -f "$drop"
        rmdir "$dropdir" 2>/dev/null || true
        return 0
    fi
    token="$(systemd_read_write_token "$data_dir")"
    mkdir -p "$dropdir"
    cat > "$drop" <<EOF
# scuffed-stat-tracker data_dir drop-in
# ProtectSystem=strict only writes the default data dir, the config dir,
# and the session runtime dir. This path is outside those.
[Service]
ReadWritePaths=$token
EOF
    chmod 644 "$drop"
    DATA_DIR_DROPIN="$drop"
}
