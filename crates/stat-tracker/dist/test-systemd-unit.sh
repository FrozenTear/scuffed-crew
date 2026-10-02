#!/usr/bin/env bash
# Discriminating tests for the systemd session-env install.
#
# The pre-fix installer copied assets/scuffed-stat-tracker.service unchanged,
# so ExecStart stayed %h/.local/bin/scuffed-stat-tracker for every PREFIX and
# the user manager never received WAYLAND_DISPLAY / DISPLAY. These checks
# fail on that installer.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
INSTALL="$DIST/install.sh"
UNINSTALL="$DIST/uninstall.sh"
HELPER="$DIST/import-session-env.sh"
UNIT_LIB="$DIST/systemd-unit.sh"
TEMPLATE="$ROOT/assets/scuffed-stat-tracker.desktop"
UNIT="$ROOT/assets/scuffed-stat-tracker.service"
SESSION_UNIT="$ROOT/assets/scuffed-stat-tracker-session.service"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*" >&2; }

[[ -f "$INSTALL" && -f "$HELPER" && -f "$UNIT_LIB" ]] || fail "missing installer files"
[[ -f "$UNIT" && -f "$SESSION_UNIT" && -f "$TEMPLATE" ]] || fail "missing unit templates"

# Sandbox directives live in the templates. install.sh copies them through
# (it only rewrites ExecStart), so the installed units must match.
assert_common_sandbox() {
    local unit="$1"
    grep -q '^NoNewPrivileges=yes$' "$unit" \
        || fail "$unit missing NoNewPrivileges=yes"
    grep -q '^ProtectSystem=strict$' "$unit" \
        || fail "$unit missing ProtectSystem=strict"
    grep -q '^ReadWritePaths=' "$unit" \
        || fail "$unit missing ReadWritePaths="
    grep -q '%t' "$unit" \
        || fail "$unit ReadWritePaths does not include the runtime dir (%t)"
    if grep -q '^PrivateDevices=' "$unit"; then
        fail "$unit sets PrivateDevices= (evdev /dev/input must stay reachable)"
    fi
    if grep -q '^DeviceAllow=' "$unit"; then
        fail "$unit sets DeviceAllow= (device policy must stay permissive)"
    fi
}

assert_common_sandbox "$UNIT"
grep -q '%h/.local/share/scuffed-stat-tracker' "$UNIT" \
    || fail "daemon unit ReadWritePaths missing the data dir"
grep -q '%h/.config/scuffed-stat-tracker' "$UNIT" \
    || fail "daemon unit ReadWritePaths missing the config/state dir"
if grep -q '^PrivateTmp=' "$UNIT"; then
    fail "daemon unit sets PrivateTmp= (that hides /tmp/.X11-unix)"
fi
assert_common_sandbox "$SESSION_UNIT"
grep -q '%h/.config/scuffed-stat-tracker' "$SESSION_UNIT" \
    || fail "session unit ReadWritePaths missing session.env's directory"
grep -q '^PrivateTmp=yes$' "$SESSION_UNIT" \
    || fail "session unit missing PrivateTmp=yes"
pass "unit templates carry the sandbox"

# ── pure ExecStart quoting ────────────────────────────────────────────────────

# shellcheck source=systemd-unit.sh
source "$UNIT_LIB"
bare="$(systemd_exec_token "/home/user/.local/bin/scuffed-stat-tracker")"
[[ "$bare" == "/home/user/.local/bin/scuffed-stat-tracker" ]] \
    || fail "safe path was quoted: $bare"
quoted="$(systemd_exec_token "/home/user/my prefix/bin/scuffed-stat-tracker")"
[[ "$quoted" == '"/home/user/my prefix/bin/scuffed-stat-tracker"' ]] \
    || fail "spaced path not quoted: $quoted"
pass "systemd ExecStart quoting"

# ── helper: compositor environ beats a stale process environment ─────────────

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

write_environ() {
    local dest="$1"
    shift
    : > "$dest"
    local kv
    for kv in "$@"; do
        printf '%s\0' "$kv" >> "$dest"
    done
}

PROC="$TMP/proc"
# pid 99 is a same-user shell that happens to have WAYLAND_DISPLAY. It must
# not be treated as the session — only known compositor comm names count.
mkdir -p "$PROC/20" "$PROC/10" "$PROC/99"
printf 'sway\n' > "$PROC/20/comm"
printf 'Xwayland\n' > "$PROC/10/comm"
printf 'bash\n' > "$PROC/99/comm"
write_environ "$PROC/20/environ" \
    "WAYLAND_DISPLAY=wayland-1" \
    "XDG_CURRENT_DESKTOP=sway" \
    "XDG_SESSION_TYPE=wayland" \
    "WAYLAND_DISPLAY_EVIL=wayland-1;rm"
write_environ "$PROC/10/environ" \
    "DISPLAY=:0" \
    "XDG_SESSION_TYPE=x11" \
    "XDG_CURRENT_DESKTOP=not a desktop"
write_environ "$PROC/99/environ" "WAYLAND_DISPLAY=wayland-7"

FAKE_CTL="$TMP/systemctl"
LOG="$TMP/systemctl.log"
cat > "$FAKE_CTL" << EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$LOG"
printf 'ENV WAYLAND_DISPLAY=%s\n' "\${WAYLAND_DISPLAY-}" >> "$LOG"
printf 'ENV DISPLAY=%s\n' "\${DISPLAY-}" >> "$LOG"
printf 'ENV XDG_CURRENT_DESKTOP=%s\n' "\${XDG_CURRENT_DESKTOP-}" >> "$LOG"
printf 'ENV XDG_SESSION_TYPE=%s\n' "\${XDG_SESSION_TYPE-}" >> "$LOG"
exit 0
EOF
chmod +x "$FAKE_CTL"

ENV_FILE="$TMP/session.env"
# Stale manager/process values must lose to the compositor.
env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
    WAYLAND_DISPLAY=wayland-9 \
    DISPLAY=:99 \
    XDG_SESSION_TYPE=tty \
    HOME="$TMP/home" \
    SCUFFED_PROC_ROOT="$PROC" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    SCUFFED_SESSION_ENV_FILE="$ENV_FILE" \
    bash "$HELPER"

[[ -f "$ENV_FILE" ]] || fail "session.env was not written"
grep -qx 'WAYLAND_DISPLAY=wayland-1' "$ENV_FILE" \
    || fail "compositor WAYLAND_DISPLAY lost. file: $(cat "$ENV_FILE")"
grep -qx 'DISPLAY=:0' "$ENV_FILE" \
    || fail "Xwayland DISPLAY was not kept. file: $(cat "$ENV_FILE")"
grep -qx 'XDG_CURRENT_DESKTOP=sway' "$ENV_FILE" \
    || fail "desktop name not taken from sway. file: $(cat "$ENV_FILE")"
grep -qx 'XDG_SESSION_TYPE=wayland' "$ENV_FILE" \
    || fail "session type did not prefer sway over Xwayland. file: $(cat "$ENV_FILE")"
grep -q 'wayland-9\|wayland-7\|:99\|XDG_SESSION_TYPE=tty\|not a desktop\|;rm' "$ENV_FILE" \
    && fail "stale or invalid value landed in session.env: $(cat "$ENV_FILE")"
grep -q 'import-environment' "$LOG" || fail "systemctl import-environment was not called: $(cat "$LOG")"
grep -qx 'ENV WAYLAND_DISPLAY=wayland-1' "$LOG" \
    || fail "import did not export compositor WAYLAND_DISPLAY: $(cat "$LOG")"
grep -qx 'ENV DISPLAY=:0' "$LOG" || fail "import did not export DISPLAY: $(cat "$LOG")"
# Mode: not world-readable (the file can name a display socket).
mode="$(stat -c %a "$ENV_FILE")"
[[ "$mode" == "600" ]] || fail "session.env mode is $mode, want 600"
pass "compositor environ overrides stale process env"

# Invalid process value is dropped when no compositor has the key.
mkdir -p "$TMP/empty-proc"
rm -f "$LOG" "$ENV_FILE"
env -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
    WAYLAND_DISPLAY='wayland-1;touch /tmp/pwned' \
    HOME="$TMP/home" \
    SCUFFED_PROC_ROOT="$TMP/empty-proc" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    SCUFFED_SESSION_ENV_FILE="$ENV_FILE" \
    bash "$HELPER"
grep -q 'WAYLAND_DISPLAY=' "$ENV_FILE" \
    && fail "invalid WAYLAND_DISPLAY was written: $(cat "$ENV_FILE")"
if [[ -f "$LOG" ]] && grep -q 'import-environment' "$LOG"; then
    fail "import-environment ran with no valid variables: $(cat "$LOG")"
fi
pass "invalid session values are not imported"

# systemctl failure still leaves session.env and exits 0.
rm -f "$ENV_FILE"
cat > "$FAKE_CTL" << 'EOF'
#!/bin/sh
exit 1
EOF
chmod +x "$FAKE_CTL"
env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
    HOME="$TMP/home" \
    SCUFFED_PROC_ROOT="$PROC" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    SCUFFED_SESSION_ENV_FILE="$ENV_FILE" \
    bash "$HELPER"
grep -qx 'WAYLAND_DISPLAY=wayland-1' "$ENV_FILE" \
    || fail "session.env missing after systemctl failure"
pass "systemctl failure still writes session.env"

# Restore a recording systemctl for the installer tests.
cat > "$FAKE_CTL" << EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$LOG"
printf 'ENV WAYLAND_DISPLAY=%s\n' "\${WAYLAND_DISPLAY-}" >> "$LOG"
printf 'ENV DISPLAY=%s\n' "\${DISPLAY-}" >> "$LOG"
printf 'ENV XDG_CURRENT_DESKTOP=%s\n' "\${XDG_CURRENT_DESKTOP-}" >> "$LOG"
printf 'ENV XDG_SESSION_TYPE=%s\n' "\${XDG_SESSION_TYPE-}" >> "$LOG"
exit 0
EOF
chmod +x "$FAKE_CTL"

# ── installer: custom PREFIX, compositor fallback, uninstall ─────────────────

stage_pkg() {
    local pkg="$1"
    mkdir -p "$pkg/bin" "$pkg/assets"
    printf '%s\n' '#!/bin/sh' 'echo scuffed-stat-tracker 0.4.4' > "$pkg/bin/scuffed-stat-tracker"
    printf '%s\n' '#!/bin/sh' 'echo stat-tracker-gui' > "$pkg/bin/stat-tracker-gui"
    chmod +x "$pkg/bin/scuffed-stat-tracker" "$pkg/bin/stat-tracker-gui"
    cp "$TEMPLATE" "$UNIT" "$SESSION_UNIT" "$pkg/assets/"
    cp "$INSTALL" "$pkg/install.sh"
    cp "$UNINSTALL" "$pkg/uninstall.sh"
    cp "$HELPER" "$UNIT_LIB" "$pkg/"
    chmod +x "$pkg/install.sh" "$pkg/uninstall.sh" "$pkg/import-session-env.sh"
}

run_install() {
    local home="$1" prefix="$2"
    rm -f "$LOG"
    env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
        HOME="$home" \
        PREFIX="$prefix" \
        SCUFFED_PROC_ROOT="$PROC" \
        SCUFFED_SYSTEMCTL="$FAKE_CTL" \
        bash "$PKG/install.sh"
}

PKG="$TMP/pkg"
HOME_DIR="$TMP/home-install"
PREFIX="$TMP/opt/scuffed"
stage_pkg "$PKG"
mkdir -p "$HOME_DIR"
run_install "$HOME_DIR" "$PREFIX"

DAEMON_UNIT="$HOME_DIR/.config/systemd/user/scuffed-stat-tracker.service"
SESSION_INSTALLED="$HOME_DIR/.config/systemd/user/scuffed-stat-tracker-session.service"
DAEMON_BIN="$PREFIX/bin/scuffed-stat-tracker"
HELPER_INSTALLED="$PREFIX/lib/scuffed-stat-tracker/import-session-env.sh"

[[ -x "$DAEMON_BIN" ]] || fail "daemon not installed at $DAEMON_BIN"
[[ -x "$HELPER_INSTALLED" ]] || fail "helper not installed at $HELPER_INSTALLED"
[[ -f "$DAEMON_UNIT" ]] || fail "daemon unit missing"
[[ -f "$SESSION_INSTALLED" ]] || fail "session unit missing"

exec_line="$(grep -E '^ExecStart=' "$DAEMON_UNIT" || true)"
[[ "$exec_line" == "ExecStart=$DAEMON_BIN" ]] \
    || fail "ExecStart is not the PREFIX binary. got: ${exec_line:-<missing>} want: ExecStart=$DAEMON_BIN"
grep -q '%h/.local/bin' "$DAEMON_UNIT" \
    && fail "daemon unit still points at %h/.local/bin"
grep -q '^EnvironmentFile=-%h/.config/scuffed-stat-tracker/session.env$' "$DAEMON_UNIT" \
    || fail "EnvironmentFile line missing from daemon unit"
grep -q '^Wants=scuffed-stat-tracker-session.service$' "$DAEMON_UNIT" \
    || fail "daemon unit does not Wants= the session oneshot"

session_exec="$(grep -E '^ExecStart=' "$SESSION_INSTALLED" || true)"
[[ "$session_exec" == "ExecStart=$HELPER_INSTALLED" ]] \
    || fail "session ExecStart is not the installed helper. got: ${session_exec:-<missing>}"
grep -q '%h/.local/lib' "$SESSION_INSTALLED" \
    && fail "session unit still points at %h/.local/lib"

installed_env="$HOME_DIR/.config/scuffed-stat-tracker/session.env"
grep -qx 'WAYLAND_DISPLAY=wayland-1' "$installed_env" \
    || fail "install did not record compositor WAYLAND_DISPLAY. file: $(cat "$installed_env")"
grep -qx 'ENV WAYLAND_DISPLAY=wayland-1' "$LOG" \
    || fail "install did not import WAYLAND_DISPLAY. log: $(cat "$LOG")"

MANIFEST="$PREFIX/share/scuffed-stat-tracker/install-manifest.txt"
grep -qx "$HELPER_INSTALLED" "$MANIFEST" || fail "helper missing from manifest"
grep -qx "$DAEMON_UNIT" "$MANIFEST" || fail "daemon unit missing from manifest"
grep -qx "$SESSION_INSTALLED" "$MANIFEST" || fail "session unit missing from manifest"
assert_common_sandbox "$DAEMON_UNIT"
assert_common_sandbox "$SESSION_INSTALLED"
grep -q '%h/.local/share/scuffed-stat-tracker' "$DAEMON_UNIT" \
    || fail "installed daemon unit lost the data-dir ReadWritePaths"
grep -q '^PrivateTmp=yes$' "$SESSION_INSTALLED" \
    || fail "installed session unit lost PrivateTmp=yes"
if grep -q '^PrivateTmp=' "$DAEMON_UNIT"; then
    fail "installed daemon unit gained PrivateTmp="
fi
pass "custom PREFIX ExecStart=$DAEMON_BIN"

# Spaced PREFIX is quoted in the unit.
PREFIX_SPACE="$TMP/my prefix"
HOME_SPACE="$TMP/home-space"
mkdir -p "$HOME_SPACE"
run_install "$HOME_SPACE" "$PREFIX_SPACE"
space_unit="$HOME_SPACE/.config/systemd/user/scuffed-stat-tracker.service"
space_exec="$(grep -E '^ExecStart=' "$space_unit" || true)"
want_space="ExecStart=\"$PREFIX_SPACE/bin/scuffed-stat-tracker\""
[[ "$space_exec" == "$want_space" ]] \
    || fail "spaced PREFIX ExecStart not quoted. got: ${space_exec:-<missing>} want: $want_space"
pass "spaced PREFIX ExecStart quoted"

# Relative PREFIX becomes an absolute ExecStart.
REL_HOME="$TMP/home-rel"
mkdir -p "$REL_HOME"
(
    cd "$TMP"
    rm -f "$LOG"
    env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
        HOME="$REL_HOME" \
        PREFIX="rel-prefix" \
        SCUFFED_PROC_ROOT="$PROC" \
        SCUFFED_SYSTEMCTL="$FAKE_CTL" \
        bash "$PKG/install.sh"
)
rel_unit="$REL_HOME/.config/systemd/user/scuffed-stat-tracker.service"
rel_abs="$TMP/rel-prefix/bin/scuffed-stat-tracker"
rel_exec="$(grep -E '^ExecStart=' "$rel_unit" || true)"
[[ "$rel_exec" == "ExecStart=$rel_abs" ]] \
    || fail "relative PREFIX ExecStart not absolute. got: ${rel_exec:-<missing>}"
pass "relative PREFIX ExecStart=$rel_abs"

# Uninstall removes the unit, the session unit, and the helper. It must not
# call the real systemctl (SCUFFED_SYSTEMCTL).
env -u WAYLAND_DISPLAY \
    HOME="$HOME_DIR" \
    PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$PKG/uninstall.sh" --yes
[[ ! -e "$DAEMON_UNIT" ]] || fail "daemon unit survived uninstall"
[[ ! -e "$SESSION_INSTALLED" ]] || fail "session unit survived uninstall"
[[ ! -e "$HELPER_INSTALLED" ]] || fail "helper survived uninstall"
pass "uninstall removes unit, session unit, and helper"

# SKIP_INTEGRATION must not install units or invoke systemctl.
rm -f "$LOG"
SKIP_HOME="$TMP/home-skip"
SKIP_PREFIX="$TMP/prefix-skip"
mkdir -p "$SKIP_HOME"
env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
    HOME="$SKIP_HOME" \
    PREFIX="$SKIP_PREFIX" \
    SKIP_INTEGRATION=1 \
    SCUFFED_PROC_ROOT="$PROC" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$PKG/install.sh"
[[ ! -e "$SKIP_HOME/.config/systemd/user/scuffed-stat-tracker.service" ]] \
    || fail "SKIP_INTEGRATION installed the daemon unit"
[[ ! -e "$SKIP_PREFIX/lib/scuffed-stat-tracker/import-session-env.sh" ]] \
    || fail "SKIP_INTEGRATION installed the helper"
[[ ! -e "$LOG" ]] || fail "SKIP_INTEGRATION invoked systemctl: $(cat "$LOG")"
pass "SKIP_INTEGRATION leaves systemd alone"

echo "all systemd session-env checks passed"
