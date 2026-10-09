#!/usr/bin/env bash
# Canonical paths a bootstrap.sh install writes, and the only paths an
# uninstall may remove. install.sh, uninstall.sh, bootstrap.sh --uninstall,
# and the desktop app all read this file. Do not keep a second copy.
#
# Source this file. Do not execute it.
#
# Each spec line is: category  when  template
#   when = always | purge
#   purge paths are local data. They are removed only with --purge,
#   or when the desktop checkbox is on.
# Placeholders: {prefix}  {home}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    echo "source this file; do not execute it" >&2
    exit 1
fi

install_path_specs() {
    cat <<'EOF'
bin always {prefix}/bin/scuffed-stat-tracker
bin always {prefix}/bin/stat-tracker-gui
bin always {prefix}/bin/scuffed-stat-tracker-uninstall
libdir always {prefix}/lib/scuffed-stat-tracker
desktop always {home}/.local/share/applications/scuffed-stat-tracker.desktop
unit always {home}/.config/systemd/user/scuffed-stat-tracker.service
unit always {home}/.config/systemd/user/scuffed-stat-tracker-session.service
dropin always {home}/.config/systemd/user/scuffed-stat-tracker.service.d/data-dir.conf
autostart always {home}/.config/systemd/user/graphical-session.target.wants/scuffed-stat-tracker.service
helper always {prefix}/lib/scuffed-stat-tracker/import-session-env.sh
list always {prefix}/lib/scuffed-stat-tracker/install-paths.sh
manifest always {prefix}/share/scuffed-stat-tracker/install-manifest.txt
data purge {home}/.local/share/scuffed-stat-tracker
config purge {home}/.config/scuffed-stat-tracker
EOF
}

expand_install_template() {
    local template="$1" home="$2" prefix="$3"
    template="${template//\{home\}/$home}"
    template="${template//\{prefix\}/$prefix}"
    printf '%s\n' "$template"
}

# stdout: category when absolute-path
# Optional third arg filters on `when` (always or purge).
expanded_install_paths() {
    local home="$1" prefix="$2" when_filter="${3:-}"
    local cat when template path
    while read -r cat when template; do
        [[ -z "${cat:-}" || "$cat" == \#* ]] && continue
        if [[ -n "$when_filter" && "$when" != "$when_filter" ]]; then
            continue
        fi
        path="$(expand_install_template "$template" "$home" "$prefix")"
        printf '%s %s %s\n' "$cat" "$when" "$path"
    done < <(install_path_specs)
}

# Absolute path for one spec. category + basename must match one line.
install_path_named() {
    local home="$1" prefix="$2" want_cat="$3" base="$4"
    local cat when path
    while read -r cat when path; do
        if [[ "$cat" == "$want_cat" && "$(basename "$path")" == "$base" ]]; then
            printf '%s\n' "$path"
            return 0
        fi
    done < <(expanded_install_paths "$home" "$prefix")
    return 1
}

# /usr, /bin, /lib, /opt, /etc, and anything under them. Bootstrap's
# default prefix is ~/.local. These system roots are never a user install.
is_system_install_path() {
    local p="$1"
    case "$p" in
        /usr|/usr/*|/bin|/bin/*|/sbin|/sbin/*|/lib|/lib/*|/lib64|/lib64/*|/opt|/opt/*|/etc|/etc/*)
            return 0
            ;;
    esac
    return 1
}

valid_package_name() {
    [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9.+_-]*$ ]]
}

# Print "pacman NAME" or "apt NAME" when a package owns $1.
package_owner_of() {
    local target="$1" line pkg
    [[ -n "$target" && -e "$target" ]] || return 1
    if command -v pacman >/dev/null 2>&1; then
        if line="$(pacman -Qo "$target" 2>/dev/null)"; then
            pkg="$(printf '%s\n' "$line" | sed -n 's/.* is owned by \([^ ][^ ]*\).*/\1/p')"
            if valid_package_name "$pkg"; then
                printf 'pacman %s\n' "$pkg"
                return 0
            fi
        fi
    fi
    if command -v dpkg >/dev/null 2>&1; then
        if line="$(dpkg -S "$target" 2>/dev/null)"; then
            pkg="${line%%:*}"
            pkg="${pkg#"${pkg%%[![:space:]]*}"}"
            if valid_package_name "$pkg"; then
                printf 'apt %s\n' "$pkg"
                return 0
            fi
        fi
    fi
    return 1
}

package_remove_command() {
    local kind="$1" pkg="$2"
    case "$kind" in
        pacman) printf 'sudo pacman -R %s\n' "$pkg" ;;
        apt) printf 'sudo apt remove %s\n' "$pkg" ;;
        *) return 1 ;;
    esac
}
