# shellcheck shell=bash
#
# Shared plumbing for the end-to-end tests.
#
# Each test runs inside a throw-away session: its own HOME and XDG dirs, a
# dconf keyfile, a private D-Bus session bus and a headless gnome-shell on its
# own Wayland socket. The real desktop and its settings are never touched.
#
# Usage, at the top of a test script:
#
#     source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
#     e2e_enter "$@"
#     ... runs inside the private session from here on ...
#
# Variables named E2E_* are passed through into the session. E2E_BIN is the
# touchpad-snap binary to test (default: the debug build in cargo's target dir),
# E2E_INJECT the key-injection helper (examples/inject_keys.rs).

set -euo pipefail

e2e_repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
e2e_uuid=touchpad-snap@hjosugi.github.io

# Removes the session directory. xdg-document-portal mounts a FUSE filesystem
# under $XDG_RUNTIME_DIR/doc and lets go of it a moment after its bus is gone,
# so unmount and retry instead of leaving the directory behind.
e2e_remove_root() {
    local root=$1 _
    for _ in $(seq 50); do
        if mountpoint -q "$root/run/doc" 2>/dev/null; then
            fusermount3 -uz "$root/run/doc" 2>/dev/null || fusermount -uz "$root/run/doc" 2>/dev/null || true
        fi
        rm -rf "$root" 2>/dev/null && return 0
        sleep 0.1
    done
    echo "warning: could not remove $root" >&2
}

e2e_enter() {
    [[ -n ${E2E_INNER:-} ]] && return 0

    : "${E2E_BIN:=$(cd "$e2e_repo" && cargo metadata --format-version 1 --no-deps | jq -r .target_directory)/debug/touchpad-snap}"
    [[ -x $E2E_BIN ]] || { echo "build first: $E2E_BIN is missing (cargo build)" >&2; exit 1; }
    : "${E2E_INJECT:=$(dirname "$E2E_BIN")/examples/inject_keys}"
    [[ -x $E2E_INJECT ]] || { echo "build first: $E2E_INJECT is missing (cargo build --examples)" >&2; exit 1; }
    : "${E2E_OWN_NAME:=$(dirname "$E2E_BIN")/examples/own_name}"
    [[ -x $E2E_OWN_NAME ]] || { echo "build first: $E2E_OWN_NAME is missing (cargo build --examples)" >&2; exit 1; }
    export E2E_BIN E2E_INJECT E2E_OWN_NAME

    local caller=${BASH_SOURCE[1]}
    local root
    root=$(mktemp -d "${TMPDIR:-/tmp}/touchpad-snap-e2e.XXXXXX")
    chmod 700 "$root"
    # shellcheck disable=SC2064
    trap "e2e_remove_root '$root'" EXIT

    mkdir -p "$root"/{home,run,tmp,cache,data/gnome-shell/extensions,config/glib-2.0/settings}
    cp -r "$e2e_repo/gnome-extension/$e2e_uuid" "$root/data/gnome-shell/extensions/"
    cat > "$root/config/glib-2.0/settings/keyfile" <<EOF2
[org/gnome/shell]
enabled-extensions=['$e2e_uuid']
disable-user-extensions=false
welcome-dialog-last-shown-version='999'
EOF2

    # Containers have no system bus, and GNOME Shell aborts without one; start
    # a private one there (and only there, unless E2E_PRIVATE_SYSTEM_BUS=1
    # asks for it, which mimics a container on a normal machine).
    local system_bus=${DBUS_SYSTEM_BUS_ADDRESS:-}
    if [[ -n ${E2E_PRIVATE_SYSTEM_BUS:-} ]] || [[ -z $system_bus && ! -S /run/dbus/system_bus_socket ]]; then
        local bus_pid
        bus_pid=$(dbus-daemon --config-file="$e2e_repo/test/e2e/system-bus.conf" \
            --address="unix:path=$root/system_bus_socket" --fork --print-pid)
        # shellcheck disable=SC2064
        trap "kill $bus_pid 2>/dev/null; e2e_remove_root '$root'" EXIT
        system_bus="unix:path=$root/system_bus_socket"
    fi

    local passthrough=() name
    while IFS= read -r name; do
        passthrough+=("$name=${!name}")
    done < <(compgen -e | grep '^E2E_' || true)

    # System data dirs stay so schemas, icons and apps are found; only the user
    # dirs are redirected.
    env -i \
        PATH="$PATH" \
        HOME="$root/home" \
        TMPDIR="$root/tmp" \
        XDG_RUNTIME_DIR="$root/run" \
        XDG_DATA_HOME="$root/data" \
        XDG_CONFIG_HOME="$root/config" \
        XDG_CACHE_HOME="$root/cache" \
        XDG_DATA_DIRS="${XDG_DATA_DIRS:-/usr/local/share:/usr/share}" \
        GSETTINGS_BACKEND=keyfile \
        GSETTINGS_SCHEMA_DIR="${GSETTINGS_SCHEMA_DIR:-}" \
        NIX_GSETTINGS_OVERRIDES_DIR="${NIX_GSETTINGS_OVERRIDES_DIR:-}" \
        GIO_EXTRA_MODULES="${GIO_EXTRA_MODULES:-}" \
        ${system_bus:+DBUS_SYSTEM_BUS_ADDRESS="$system_bus"} \
        E2E_INNER=1 \
        E2E_ROOT="$root" \
        "${passthrough[@]}" \
        dbus-run-session -- bash "$caller" "$@"
    exit
}

# --- helpers for the inside of the session ---------------------------------------

e2e_pids=()
e2e_shell_log=${E2E_ROOT:-/dev/null}/shell.log
e2e_extra_logs=()   # more log files to show on failure

e2e_cleanup() {
    local pid
    for pid in "${e2e_pids[@]}"; do kill "$pid" 2>/dev/null || true; done
    wait 2>/dev/null || true
}
[[ -n ${E2E_INNER:-} ]] && trap e2e_cleanup EXIT

fail() {
    echo "FAIL: $*" >&2
    echo "--- window list ---" >&2
    list_json >&2 2>/dev/null || true
    local log
    for log in "$e2e_shell_log" "${e2e_extra_logs[@]}"; do
        echo "--- $(basename "$log") (tail) ---" >&2
        tail -n 25 "$log" >&2 2>/dev/null || true
    done
    exit 1
}

ok() { echo "ok:   $*"; }

tps() { "$E2E_BIN" "$@"; }

# Types a key combination through Mutter's RemoteDesktop API: press_keys Super_L Alt_L n
press_keys() { "$E2E_INJECT" "$@"; }

# Sends the daemon a Swipe signal as the extension would after claiming a
# touchpad swipe: swipe <phase> <fingers> <mods> <app_id> <dx> <dy>
swipe() {
    gdbus emit --session --object-path /io/github/hjosugi/TouchpadSnap \
        --signal io.github.hjosugi.TouchpadSnap1.Swipe -- \
        "uint32 $1" "uint32 $2" "uint32 $3" "$4" "$5" "$6" >/dev/null
}

# Call a method of the extension.
call() {
    gdbus call --session --dest io.github.hjosugi.TouchpadSnap \
        --object-path /io/github/hjosugi/TouchpadSnap \
        --method "io.github.hjosugi.TouchpadSnap1.$1" "${@:2}"
}

list_json() { tps list --json; }

window_count_is() { list_json | jq -e --argjson n "$1" '.windows | length == $n'; }

# The id of the most recently used window, and of the focused one.
front() { list_json | jq '.windows | min_by(.mru) | .id'; }
front_is() { [[ $(front) == "$1" ]]; }
focused() { list_json | jq '[.windows[] | select(.focused) | .id] | first // 0'; }
focus_is() { [[ $(focused) == "$1" ]]; }

# Activates a window through the extension until it holds keyboard focus.
#
# Right after start-up a headless session can hand focus to a freshly mapped
# window after an activation already went through (there is no real seat to
# settle it), so a single Activate is not always enough there. Later
# activations move focus at once, which is what the jump tests rely on.
focus_on() {
    local id=$1 _
    for _ in 1 2 3 4 5; do
        [[ $(call Activate "uint64 $id") == "(true,)" ]] || fail "Activate($id) returned false"
        for _ in $(seq 20); do
            focus_is "$id" && return 0
            sleep 0.1
        done
    done
    fail "window $id never got focus"
}

# wait_for <description> <timeout-seconds> <command...>
wait_for() {
    local what=$1 timeout=$2
    shift 2
    local _
    for _ in $(seq $((timeout * 10))); do
        if "$@" >/dev/null 2>&1; then return 0; fi
        sleep 0.1
    done
    fail "timed out waiting for: $what"
}

# Starts the headless gnome-shell and waits for the extension to come up.
e2e_start_shell() {
    export WAYLAND_DISPLAY=wayland-e2e
    gnome-shell --headless --wayland --no-x11 --wayland-display "$WAYLAND_DISPLAY" \
        --virtual-monitor 1280x720 >"$e2e_shell_log" 2>&1 &
    e2e_pids+=($!)

    wait_for "wayland socket" 30 test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
    wait_for "extension bus name" 30 gdbus call --session --dest io.github.hjosugi.TouchpadSnap \
        --object-path /io/github/hjosugi/TouchpadSnap --method org.freedesktop.DBus.Peer.Ping
}

# Opens two text editors (separate processes, same app) and a calculator, in
# that order, and waits for all three windows.
e2e_open_clients() {
    GDK_BACKEND=wayland gnome-text-editor --standalone >"$E2E_ROOT/editor1.log" 2>&1 &
    e2e_pids+=($!)
    wait_for "first text editor window" 30 window_count_is 1

    GDK_BACKEND=wayland gnome-text-editor --standalone >"$E2E_ROOT/editor2.log" 2>&1 &
    e2e_pids+=($!)
    wait_for "second text editor window" 30 window_count_is 2

    GDK_BACKEND=wayland gnome-calculator >"$E2E_ROOT/calc.log" 2>&1 &
    e2e_pids+=($!)
    wait_for "calculator window" 30 window_count_is 3
}
