#!/usr/bin/env bash
# End-to-end test against a real (headless) GNOME Shell.
#
# Drives the extension's D-Bus API directly and through the window-jump
# binary: List, Activate, SetAccelerators, `window-jump list/next/prev`, and
# the daemon's registration of hotkeys and swipe claims.
#
# Needs gnome-shell, gnome-text-editor, gnome-calculator, dbus-run-session,
# gdbus and jq on PATH, and a built binary (cargo build).
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
e2e_enter "$@"

e2e_start_shell
ok "extension loaded and owns org.hjosugi.WindowJump"

version=$(gdbus call --session --dest org.hjosugi.WindowJump --object-path /org/hjosugi/WindowJump \
    --method org.freedesktop.DBus.Properties.Get org.hjosugi.WindowJump1 ProtocolVersion)
[[ $version == "(<uint32 1>,)" ]] || fail "unexpected protocol version: $version"
[[ $(list_json | jq '.windows | length') == 0 ]] || fail "expected no windows initially"
ok "protocol 1, empty list at startup"

e2e_open_clients
# A window gets its size a moment after it is mapped.
placed() { list_json | jq -e 'all(.windows[]; .width > 0 and .height > 0)'; }
wait_for "window geometry" 15 placed
json=$(list_json)
echo "$json" | jq -c '.windows[] | {id, app_id, wm_class, title, mru, x, y, width, height}'

editors=($(echo "$json" | jq '[.windows[] | select(.app_id | test("TextEditor"))] | sort_by(.id) | .[].id'))
calc=$(echo "$json" | jq '.windows[] | select(.app_id | test("TextEditor") | not) | .id')
[[ ${#editors[@]} == 2 ]] || fail "expected two text editor windows grouped under one app id"
[[ -n $calc ]] || fail "calculator window missing"
echo "$json" | jq -e 'all(.windows[]; .id > 0 and .pid > 0 and .width > 0 and .height > 0)' >/dev/null \
    || fail "every window needs id, pid and geometry"
ok "three windows, the two editors share an app (${editors[*]}), calculator $calc"

wj list | grep -q "TextEditor" || fail "`window-jump list` does not show the editors"
ok "window-jump list"

# Focus the first editor, then cycle with the binary. Asserted on keyboard
# focus, which is what the binary itself goes by.
focus_on "${editors[0]}"

wj next || fail "window-jump next failed"
wait_for "next → second editor" 15 focus_is "${editors[1]}"
ok "next: ${editors[0]} → ${editors[1]}"

wj next || fail "window-jump next failed"
wait_for "next wraps → first editor" 15 focus_is "${editors[0]}"
ok "next wraps round to ${editors[0]} (calculator skipped)"

wj prev || fail "window-jump prev failed"
wait_for "prev → second editor" 15 focus_is "${editors[1]}"
ok "prev: ${editors[0]} → ${editors[1]}"

# Quick repeats inside the focus hint still step through the windows.
wj next && wj next || fail "window-jump next failed"
wait_for "two quick nexts → second editor again" 15 focus_is "${editors[1]}"
front_is "${editors[1]}" || fail "MRU order does not follow the jumps"
ok "two quick nexts land back on ${editors[1]}"

# The calculator is alone in its app: jumping from it is a no-op, not an error.
focus_on "$calc"
wj next || fail "window-jump next from a single-window app must succeed"
sleep 0.5
focus_is "$calc" || fail "jumping from a single-window app moved focus"
ok "single-window app: next is a no-op"

[[ $(call Activate "uint64 999999999") == "(false,)" ]] || fail "Activate on unknown id must return false"
ok "unknown id rejected"

# Hotkeys: a free accelerator is grabbed, one the shell owns is reported back.
grab=$(call SetAccelerators "[('<Super><Alt>n', 'hotkey:0'), ('<Alt>Tab', 'hotkey:1')]")
[[ $grab == "(['<Alt>Tab'],)" ]] || fail "SetAccelerators: expected only <Alt>Tab to fail, got $grab"
call SetSwipeClaims "[(uint32 3, uint32 2)]" >/dev/null || fail "SetSwipeClaims failed"
ok "SetAccelerators grabs free keys and reports taken ones; SetSwipeClaims accepted"

# The daemon, with the default config (no file): Super+Alt+N/P and
# Ctrl + three-finger swipes. Release what the gdbus calls above grabbed by
# letting the daemon take over (a new caller replaces the previous one).
wj config check >/dev/null || fail "config check failed"
daemon_log="$E2E_ROOT/daemon.log"
e2e_extra_logs+=("$daemon_log")
wj daemon --verbose >"$daemon_log" 2>&1 &
daemon_pid=$!
e2e_pids+=($daemon_pid)
wait_for "daemon registration" 15 grep -q "registered 2 hotkey(s) and 1 swipe claim(s)" "$daemon_log"
ok "daemon registered its hotkeys and swipe claim"

# Hotkeys, typed on a virtual keyboard, go through the shell, the extension
# and the daemon.
focus_on "${editors[0]}"
press_keys Super_L Alt_L n || fail "injecting Super+Alt+N failed"
wait_for "Super+Alt+N → second editor" 15 focus_is "${editors[1]}"
press_keys Super_L Alt_L p || fail "injecting Super+Alt+P failed"
wait_for "Super+Alt+P → first editor" 15 focus_is "${editors[0]}"
grep -q "hotkey Super+Alt+N → Next" "$daemon_log" || fail "daemon did not log the hotkey"
ok "hotkeys: Super+Alt+N / Super+Alt+P cycle the editors through the daemon"

# Swipes: what the extension forwards for a claimed Ctrl + three-finger swipe.
# Left is "next"; 130 units passes the 120 threshold.
sleep 0.5
swipe 0 3 2 0.0 0.0
swipe 1 3 2 -70.0 0.0
swipe 1 3 2 -60.0 0.0
swipe 2 3 2 0.0 0.0
wait_for "swipe left → second editor" 15 focus_is "${editors[1]}"
# A swipe with an unclaimed modifier set does nothing.
sleep 0.5
swipe 0 3 4 0.0 0.0
swipe 1 3 4 -300.0 0.0
swipe 2 3 4 0.0 0.0
sleep 0.5
focus_is "${editors[1]}" || fail "a swipe without a binding moved focus"
ok "swipes: Ctrl + three fingers left jumps, unbound swipes do not"

# Editing the config while the daemon runs: the new hotkey takes over within
# a couple of seconds, without a restart.
mkdir -p "$XDG_CONFIG_HOME/window-jump"
cat > "$XDG_CONFIG_HOME/window-jump/config.toml" <<'TOML'
[[hotkey]]
keys = "Ctrl+Alt+J"
action = "next"
TOML
wait_for "daemon to reload the config" 15 grep -q "registered 1 hotkey(s) and 1 swipe claim(s)" "$daemon_log"
grep -q "reloaded .*config.toml" "$daemon_log" || fail "daemon did not log the reload"
focus_on "${editors[0]}"
press_keys Control_L Alt_L j || fail "injecting Ctrl+Alt+J failed"
wait_for "Ctrl+Alt+J → second editor" 15 focus_is "${editors[1]}"
# A broken edit is ignored; the working hotkey stays.
echo 'order = "sideways"' > "$XDG_CONFIG_HOME/window-jump/config.toml"
wait_for "daemon to reject the broken config" 15 grep -q "keeping the previous config" "$daemon_log"
press_keys Control_L Alt_L j || fail "injecting Ctrl+Alt+J failed"
wait_for "Ctrl+Alt+J still works → first editor" 15 focus_is "${editors[0]}"
# Removing the file brings back the defaults (Super+Alt+N/P).
rm "$XDG_CONFIG_HOME/window-jump/config.toml"
wait_for "daemon back on the defaults" 15 bash -c "grep -c 'registered 2 hotkey(s) and 1 swipe claim(s)' '$daemon_log' | grep -qx 2"
ok "config reload: new hotkey active without a restart, broken edits ignored, removal restores defaults"

# A second daemon refuses to start.
if wj daemon >"$E2E_ROOT/daemon2.log" 2>&1; then fail "second daemon started"; fi
grep -q "already running" "$E2E_ROOT/daemon2.log" || fail "second daemon gave no reason"
ok "second daemon refused"

doctor=$(wj doctor 2>&1) || fail "doctor reported problems: $doctor"
echo "$doctor"
grep -q "protocol 1" <<<"$doctor" || fail "doctor does not see the extension"
ok "doctor"

# When the daemon goes away, the extension lets go of its keys at once.
kill "$daemon_pid"
regrab() { [[ $(call SetAccelerators "[('<Super><Alt>n', 'probe')]") == "(@as [],)" ]]; }
wait_for "Super+Alt+N released after the daemon exited" 15 regrab
ok "daemon exit releases its hotkeys"

echo "ALL PASSED"
