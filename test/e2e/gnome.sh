#!/usr/bin/env bash
# End-to-end test against a real (headless) GNOME Shell.
#
# Drives the extension's D-Bus API directly and through the touchpad-snap
# binary: List, Activate, SetAccelerators, `touchpad-snap list/next/prev`, and
# the daemon's registration of hotkeys and swipe claims.
#
# Needs gnome-shell, gnome-text-editor, gnome-calculator, dbus-run-session,
# gdbus and jq on PATH, and a built binary (cargo build).
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
e2e_enter "$@"

e2e_start_shell
ok "extension loaded and owns io.github.hjosugi.TouchpadSnap"

version=$(gdbus call --session --dest io.github.hjosugi.TouchpadSnap --object-path /io/github/hjosugi/TouchpadSnap \
    --method org.freedesktop.DBus.Properties.Get io.github.hjosugi.TouchpadSnap1 ProtocolVersion)
[[ $version == "(<uint32 4>,)" ]] || fail "unexpected protocol version: $version"
[[ $(list_json | jq '.windows | length') == 0 ]] || fail "expected no windows initially"
ok "protocol 4, empty list at startup"

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

tps list | grep -q "TextEditor" || fail "`touchpad-snap list` does not show the editors"
ok "touchpad-snap list"

# Focus the first editor, then cycle with the binary. Asserted on keyboard
# focus, which is what the binary itself goes by.
focus_on "${editors[0]}"

tps next || fail "touchpad-snap next failed"
wait_for "next → second editor" 15 focus_is "${editors[1]}"
ok "next: ${editors[0]} → ${editors[1]}"

tps next || fail "touchpad-snap next failed"
wait_for "next wraps → first editor" 15 focus_is "${editors[0]}"
ok "next wraps round to ${editors[0]} (calculator skipped)"

tps prev || fail "touchpad-snap prev failed"
wait_for "prev → second editor" 15 focus_is "${editors[1]}"
ok "prev: ${editors[0]} → ${editors[1]}"

# Quick repeats inside the focus hint still step through the windows.
tps next && tps next || fail "touchpad-snap next failed"
wait_for "two quick nexts → second editor again" 15 focus_is "${editors[1]}"
front_is "${editors[1]}" || fail "MRU order does not follow the jumps"
ok "two quick nexts land back on ${editors[1]}"

# The calculator is alone in its app: jumping from it is a no-op, not an error.
focus_on "$calc"
tps next || fail "touchpad-snap next from a single-window app must succeed"
sleep 0.5
focus_is "$calc" || fail "jumping from a single-window app moved focus"
ok "single-window app: next is a no-op"

[[ $(call Activate "uint64 999999999") == "(false,)" ]] || fail "Activate on unknown id must return false"
ok "unknown id rejected"

# The overlay after a jump (the CLI jumps above already asked for it).
[[ $(call ShowHud "uint64 ${editors[0]}" "uint32 1" "uint32 2" "uint32 300") == "(true,)" ]] \
    || fail "ShowHud on a live window must return true"
[[ $(call ShowHud "uint64 999999999" "uint32 1" "uint32 1" "uint32 300") == "(false,)" ]] \
    || fail "ShowHud on an unknown id must return false"
ok "ShowHud"

# Hotkeys: a free accelerator is grabbed, one the shell owns is reported back.
grab=$(call SetAccelerators "[('<Super><Alt>n', 'hotkey:0'), ('<Alt>Tab', 'hotkey:1')]")
[[ $grab == "(['<Alt>Tab'],)" ]] || fail "SetAccelerators: expected only <Alt>Tab to fail, got $grab"
call SetSwipeClaims "[(uint32 3, uint32 2)]" >/dev/null || fail "SetSwipeClaims failed"
ok "SetAccelerators grabs free keys and reports taken ones; SetSwipeClaims accepted"

# An upgrade from window-jump, this program's old name: its config file is
# carried over the first time ours is looked for, and only then.
mkdir -p "$XDG_CONFIG_HOME/window-jump"
echo 'mru_session_ms = 1234' > "$XDG_CONFIG_HOME/window-jump/config.toml"
checked=$(tps config check 2>&1) || fail "config check failed: $checked"
grep -q "copied the settings of window-jump" <<<"$checked" || fail "the old config was not carried over: $checked"
grep -q 1234 "$XDG_CONFIG_HOME/touchpad-snap/config.toml" || fail "the carried-over config is not the old one"
echo 'mru_session_ms = 99' > "$XDG_CONFIG_HOME/window-jump/config.toml"
checked=$(tps config check 2>&1) || fail "config check failed: $checked"
grep -q "copied" <<<"$checked" && fail "the old config was carried over twice"
grep -q 1234 "$XDG_CONFIG_HOME/touchpad-snap/config.toml" || fail "the old config overwrote ours"
rm -r "$XDG_CONFIG_HOME/window-jump" "$XDG_CONFIG_HOME/touchpad-snap"
ok "window-jump's config file is carried over once"

# The daemon, with the default config (no file): Super+Alt+N/P, Ctrl +
# three fingers left/right and Alt + three fingers up/down. Release what the
# gdbus calls above grabbed by letting the daemon take over (a new caller
# replaces the previous one).
tps config check >/dev/null || fail "config check failed"
daemon_log="$E2E_ROOT/daemon.log"
e2e_extra_logs+=("$daemon_log")
tps daemon --verbose >"$daemon_log" 2>&1 &
daemon_pid=$!
e2e_pids+=($daemon_pid)
wait_for "daemon registration" 15 grep -q "registered 2 hotkey(s) and 2 swipe claim(s)" "$daemon_log"
ok "daemon registered its hotkeys and swipe claims"

# Hotkeys, typed on a virtual keyboard, go through the shell, the extension
# and the daemon.
focus_on "${editors[0]}"
press_keys Super_L Alt_L n || fail "injecting Super+Alt+N failed"
wait_for "Super+Alt+N → second editor" 15 focus_is "${editors[1]}"
press_keys Super_L Alt_L p || fail "injecting Super+Alt+P failed"
wait_for "Super+Alt+P → first editor" 15 focus_is "${editors[0]}"
grep -q "hotkey Super+Alt+N → Next (new press)" "$daemon_log" || fail "daemon did not log the hotkey"
wait_for "the release of Super+Alt to be reported" 15 grep -q "hotkey modifiers released" "$daemon_log"
ok "hotkeys: Super+Alt+N / Super+Alt+P cycle the editors through the daemon"

# Swipes: what the extension forwards for a claimed Ctrl + three-finger swipe.
# Left is "next"; 130 units passes the 120 threshold.
sleep 0.5
swipe 0 3 2 0.0 0.0
swipe 1 3 2 -70.0 0.0
swipe 1 3 2 -60.0 0.0
swipe 2 3 2 0.0 0.0
wait_for "swipe left → second editor" 15 focus_is "${editors[1]}"
# A swipe with an unclaimed modifier set (Shift) does nothing, nor does a
# claimed one in a direction without a binding (Alt + left).
sleep 0.5
swipe 0 3 1 0.0 0.0
swipe 1 3 1 -300.0 0.0
swipe 2 3 1 0.0 0.0
swipe 0 3 4 0.0 0.0
swipe 1 3 4 -300.0 0.0
swipe 2 3 4 0.0 0.0
sleep 0.5
focus_is "${editors[1]}" || fail "a swipe without a binding moved focus"
# Alt + three fingers up (negative dy) is "next" too: on to the first editor.
swipe 0 3 4 0.0 0.0
swipe 1 3 4 0.0 -130.0
swipe 2 3 4 0.0 0.0
wait_for "Alt + swipe up → first editor" 15 focus_is "${editors[0]}"
grep -q "swipe Up with 3 fingers" "$daemon_log" || fail "daemon did not log the swipe up"
ok "swipes: Ctrl + left and Alt + up jump, unbound swipes do not"

# Editing the config while the daemon runs: the new hotkey takes over within
# a couple of seconds, without a restart.
mkdir -p "$XDG_CONFIG_HOME/touchpad-snap"
cat > "$XDG_CONFIG_HOME/touchpad-snap/config.json" <<'JSON'
{
  "bindings": [
    {
      "trigger": { "type": "hotkey", "keys": "Ctrl+Alt+J" },
      "actions": [ { "type": "window.next" } ]
    }
  ]
}
JSON
wait_for "daemon to reload the config" 15 grep -q "registered 1 hotkey(s) and 0 swipe claim(s)" "$daemon_log"
grep -q "reloaded .*config.json" "$daemon_log" || fail "daemon did not log the reload"
focus_on "${editors[0]}"
press_keys Control_L Alt_L j || fail "injecting Ctrl+Alt+J failed"
wait_for "Ctrl+Alt+J → second editor" 15 focus_is "${editors[1]}"
# A broken edit is ignored; the working hotkey stays.
echo '{"cycle":{"order":"sideways"}}' > "$XDG_CONFIG_HOME/touchpad-snap/config.json"
wait_for "daemon to reject the broken config" 15 grep -q "keeping the previous config" "$daemon_log"
press_keys Control_L Alt_L j || fail "injecting Ctrl+Alt+J failed"
wait_for "Ctrl+Alt+J still works → first editor" 15 focus_is "${editors[0]}"
# Removing the file brings back the defaults (Super+Alt+N/P).
rm "$XDG_CONFIG_HOME/touchpad-snap/config.json"
wait_for "daemon back on the defaults" 15 bash -c "grep -c 'registered 2 hotkey(s) and 2 swipe claim(s)' '$daemon_log' | grep -qx 2"
ok "config reload: new hotkey active without a restart, broken edits ignored, removal restores defaults"

# Alt+Tab-style sessions with order = "mru": presses while Super+Alt stays
# down dig further back, however slowly they come; releasing commits, and the
# windows passed on the way do not count as used. Needs a third editor.
# What the daemon logged since `mark_log`.
mark_log() { log_mark=$(wc -l <"$daemon_log"); }
logged() { tail -n +$((log_mark + 1)) "$daemon_log" | grep -q "$1"; }
mark_log
cat > "$XDG_CONFIG_HOME/touchpad-snap/config.json" <<'JSON'
{ "cycle": { "order": "mru" } }
JSON
wait_for "daemon to load order = mru" 15 logged "reloaded .*config.json"
GDK_BACKEND=wayland gnome-text-editor --standalone >"$E2E_ROOT/editor3.log" 2>&1 &
e2e_pids+=($!)
wait_for "third text editor window" 30 window_count_is 4
editors3=($(list_json | jq '[.windows[] | select(.app_id | test("TextEditor"))] | sort_by(.id) | .[].id'))
[[ ${#editors3[@]} == 3 ]] || fail "expected three editors"
# MRU order: e0, e1, e2.
focus_on "${editors3[2]}"
focus_on "${editors3[1]}"
focus_on "${editors3[0]}"
sleep 0.5
mark_log
# Hold Super+Alt, press N, wait longer than mru_session_ms (1 s), press N again.
press_keys +Super_L +Alt_L n @1500 n @300 -Alt_L -Super_L || fail "injecting a held Super+Alt+N+N failed"
wait_for "held Super+Alt, N, N → third editor" 15 focus_is "${editors3[2]}"
logged "hotkey Super+Alt+N → Next (new press)" || fail "the first press did not start a session"
logged "hotkey Super+Alt+N → Next (keys still held)" || fail "the second press did not continue the session"
wait_for "the release to be reported" 15 logged "hotkey modifiers released"
# The shell's MRU order is now e2, e1, e0, but e1 was only passed through:
# a new press goes back to e0, where the session started.
press_keys Super_L Alt_L n || fail "injecting Super+Alt+N failed"
wait_for "next press → back to the first editor" 15 focus_is "${editors3[0]}"
ok "hotkey sessions: held modifiers keep digging past the timeout, release commits"
rm "$XDG_CONFIG_HOME/touchpad-snap/config.json"

# The `keys` action: Super+Alt+K types Ctrl+Shift+N, which opens a new text
# editor window. It only does if Super and Alt, held for the hotkey, are up
# by then: Super+Alt+Ctrl+Shift+N opens nothing. (A virtual keyboard cannot
# release them, so the extension waits for the release.)
mark_log
cat > "$XDG_CONFIG_HOME/touchpad-snap/config.json" <<'JSON'
{ "bindings": [ {
    "name": "New editor window",
    "trigger": { "type": "hotkey", "keys": "Super+Alt+K" },
    "actions": [ { "type": "keys", "keys": "Ctrl+Shift+N" } ]
} ] }
JSON
wait_for "daemon to load the keys binding" 15 logged "reloaded .*config.json"
focus_on "${editors3[0]}"
sleep 0.5
press_keys Super_L Alt_L k || fail "injecting Super+Alt+K failed"
wait_for "Ctrl+Shift+N typed by the keys action → a fifth window" 30 window_count_is 5
logged "keys Ctrl+Shift+N" || fail "daemon did not log the keys action"
list_json | jq -e '[.windows[] | select(.app_id | test("TextEditor"))] | length == 4' >/dev/null \
    || fail "the new window is not a text editor"
ok "keys action: Super+Alt+K types Ctrl+Shift+N once Super and Alt are up"
# Nobody but the registered daemon may type.
denied=$(call SendKeys "uint32 97" "uint32 0" 2>&1) && fail "SendKeys from another connection was accepted"
grep -q "AccessDenied" <<<"$denied" || fail "SendKeys from another connection: expected AccessDenied, got: $denied"
ok "SendKeys refuses callers other than the daemon"
rm "$XDG_CONFIG_HOME/touchpad-snap/config.json"

# A second daemon refuses to start.
if tps daemon >"$E2E_ROOT/daemon2.log" 2>&1; then fail "second daemon started"; fi
grep -q "already running" "$E2E_ROOT/daemon2.log" || fail "second daemon gave no reason"
ok "second daemon refused"

doctor=$(tps doctor 2>&1) || fail "doctor reported problems: $doctor"
echo "$doctor"
grep -q "protocol 4" <<<"$doctor" || fail "doctor does not see the extension"
ok "doctor"

# What window-jump, this program's old name, can leave behind: its GNOME
# Shell extension still enabled (played here by a process that owns its bus
# name) and its autostart entry. doctor reports both; `autostart enable`
# replaces the entry.
"$E2E_OWN_NAME" org.hjosugi.WindowJump >"$E2E_ROOT/old-extension.log" 2>&1 &
old_extension_pid=$!
e2e_pids+=($old_extension_pid)
wait_for "the stand-in for the old extension" 15 grep -q "owning" "$E2E_ROOT/old-extension.log"
mkdir -p "$XDG_CONFIG_HOME/autostart"
touch "$XDG_CONFIG_HOME/autostart/window-jump.desktop"
if doctor=$(tps doctor 2>&1); then fail "doctor missed what window-jump left behind: $doctor"; fi
grep -q "gnome-extensions disable window-jump@hjosugi.github.io" <<<"$doctor" || fail "doctor does not name the old extension: $doctor"
grep -q "window-jump still starts at login" <<<"$doctor" || fail "doctor does not name the old autostart entry: $doctor"
kill "$old_extension_pid"
tps autostart enable >/dev/null || fail "autostart enable failed"
[[ -e "$XDG_CONFIG_HOME/autostart/touchpad-snap.desktop" && ! -e "$XDG_CONFIG_HOME/autostart/window-jump.desktop" ]] \
    || fail "autostart enable did not replace the old entry"
tps autostart disable >/dev/null || fail "autostart disable failed"
doctor=$(tps doctor 2>&1) || fail "doctor still reports problems: $doctor"
ok "doctor reports what window-jump left behind; autostart enable replaces its entry"

# When the daemon goes away, the extension lets go of its keys at once.
kill "$daemon_pid"
regrab() { [[ $(call SetAccelerators "[('<Super><Alt>n', 'probe')]") == "(@as [],)" ]]; }
wait_for "Super+Alt+N released after the daemon exited" 15 regrab
ok "daemon exit releases its hotkeys"

# Nothing the extension did along the way may have thrown.
if grep -E "JS ERROR|JS WARNING" "$e2e_shell_log" | grep -q "touchpad-snap@"; then
    grep -E -A5 "JS ERROR|JS WARNING" "$e2e_shell_log" | grep -B2 -A5 "touchpad-snap@" >&2
    fail "the extension logged JavaScript errors"
fi
ok "no JavaScript errors from the extension"

echo "ALL PASSED"
