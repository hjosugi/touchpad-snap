# Changelog

## [Unreleased]

### Added

- Window, workspace and overview actions: `window.minimize`, `maximize`,
  `restore`, `toggle_maximize`, `fullscreen`, `close`, `tile` (halves and
  corners), `move_to_monitor`, `move_to_workspace`, `workspace.next/prev` and
  `overview.toggle`, on GNOME and Windows where the platform has an API.
  `window.close` never repeats within one held gesture. (#35)
- A local control channel in the daemon (Unix socket / Windows named pipe,
  newline-delimited JSON): `status`, `reload`, `run` and `subscribe`, plus the
  `touchpad-snap status`, `run` and `events` commands. `next`/`prev` go
  through the daemon when it runs, so one place owns the MRU session; `doctor`
  asks it too and the socket is the second-instance check. (#41)
- Esc cancels a held session and returns to the window it started from
  (`touchpad-snap` restores the origin and forgets the windows passed). (#43)
- Pinch and hold triggers on GNOME, with the extension claiming
  `(fingers, modifiers)` pairs: `{ "type": "pinch", "direction": "in" }` and
  `{ "type": "hold", "fingers": 3 }`. (#17)
- Mouse button triggers on Windows, including a held button + wheel
  (`{ "type": "mouse", "button": "back", "scroll": "up" }`), swallowed only
  while a binding matches. (#44)
- Modifier double-tap and one-key sequence triggers:
  `{ "type": "double_tap", "modifier": "ctrl" }` and
  `{ "type": "sequence", "keys": "Super+Alt+W", "then": "M" }`, on GNOME and
  Windows. (#45)
- Packaging for AUR, winget and Scoop, generated from the release checksums;
  optional Windows code signing in the release workflow, and a guide for
  submitting the extension to extensions.gnome.org. (#12, #14, #15)

### Changed

- The crate is a Cargo workspace: the platform-neutral config, triggers,
  conditions, window model and cycling logic live in `touchpad-snap-core`;
  the daemon, CLI and backends re-export them, so nothing embeds a GUI
  toolkit in the daemon. (#40)
- GNOME extension protocol 8 adds pinch/hold claims, sequence suffixes,
  modifier double-taps, `Cancel` and the generic window/desktop actions.
  Older extensions keep working without the newer triggers. (#17, #35, #43,
  #45)

## [0.6.0] - 2026-11-23

### Added

- The `keys` action types a keyboard shortcut into the focused window,
  `{ "type": "keys", "keys": "Ctrl+Shift+Tab" }`, from any swipe or hotkey.
  The trigger's own modifiers are not part of it: `Alt` + swipe sending
  `Ctrl+Tab` arrives as `Ctrl+Tab`. Windows releases the held modifiers for it
  and presses them again; GNOME's virtual keyboard cannot release a key you
  hold (mutter ignores it), so the extension types the shortcut when the
  modifier comes up. `Backspace` and `Delete` join the key names. (#33)
- The `exec` action runs a command, as an argument list or (with
  `"shell": true`) a shell string, detached from the daemon with the focused
  window and trigger in its environment; `config check` warns about programs
  missing from `PATH`. (#34)
- `when` conditions restrict a binding to apps (`app` / `not_app`, glob
  patterns), the OS and the desktop; a matching conditional binding beats an
  unconditional one, ties go to file order. On GNOME the app filter travels
  with the swipe claim (protocol 4), so a swipe bound for one app stays the
  shell's everywhere else. (#37)
- The `app.next` / `app.prev` actions, and the matching `next-app` /
  `prev-app` commands, move to the most recently used window of the next or
  previous app, the companion of `window.next` / `window.prev`. Apps are
  ordered by their most recent window, the usual session rules apply (a held
  trigger freezes the order and steps one app further), and the HUD counts
  apps. (#36)

### Changed

- The configuration is now `config.json`: a list of bindings, each a trigger
  (swipe or hotkey), an optional `name`/`enabled`/`repeat`/`when`, and an
  ordered action chain (`window.next`, `window.prev`, `app.next`, `app.prev`,
  `delay`, `keys`, `exec`). Window-management and workspace actions follow in
  the next issues. A malformed or
  unsupported binding is reported and skipped instead of failing the whole
  file, and a committed JSON Schema (`schema/config.schema.json`,
  `touchpad-snap config schema`) documents the format. The old `config.toml`
  is still read in place; `touchpad-snap config import` writes the JSON
  version. (#32)
- GNOME extension protocol 5 combines per-app swipe claims (4) and `SendKeys`
  (5); older extensions still work without the newer features.

### Fixed

- Releasing a lone `Alt` or `Super` after a claimed swipe no longer opens the
  focused window's menu bar (Alt), the GNOME overview or the Windows Start
  menu (Super): the extension and the Windows backend tap an unbound key when
  a claimed swipe ends with the modifier still down. (#38)

## [0.5.1] - 2026-11-22

### Added

- `touchpad-snap autostart enable --systemd` writes and enables a
  `~/.config/systemd/user/touchpad-snap.service` user unit
  (`PartOf=graphical-session.target`, `Restart=on-failure`) and
  `autostart disable --systemd` removes it. The Linux release archive ships the
  unit as `touchpad-snap.service`. (#19)
- Nix flake modules `nixosModules.touchpad-snap` and
  `homeManagerModules.touchpad-snap`: `services.touchpad-snap` installs the
  package, writes `config.toml` from `settings`, runs the daemon as a systemd
  user service, and can enable the GNOME extension. (#13)

## [0.5.0] - 2026-11-22

### Changed

- **Renamed to touchpad-snap.** The program is growing from a window switcher
  into a binder of touchpad gestures and shortcuts to actions, and the name
  now says so. Everything follows: the repository, the binaries
  (`touchpad-snap`, `touchpad-snapw.exe`), the GNOME Shell extension
  (`touchpad-snap@hjosugi.github.io`), its D-Bus name
  (`io.github.hjosugi.TouchpadSnap`), the config directory and the autostart
  entry. Upgrading: the old config file is copied over on first use,
  `autostart enable` replaces the old entry, the GNOME extension has to be
  switched by hand, and `doctor` reports what is left of window-jump. The
  portal app id is now `io.github.hjosugi.TouchpadSnap` and the environment
  variable `TOUCHPAD_SNAP_HOTKEYS` (`WINDOW_JUMP_HOTKEYS` is still read). See
  "Upgrading from window-jump" in the README. (#31)

## [0.4.0] - 2026-11-22

### Added

- Sway, Hyprland and niri backends: window lists, `next`/`prev` and `activate`
  work through each compositor's IPC, selected by `SWAYSOCK`,
  `HYPRLAND_INSTANCE_SIGNATURE` or `NIRI_SOCKET`. Bind the commands in the
  compositor config; the daemon is not needed there. (#9)
- KDE Plasma backend: a KWin script bridges the window list and activation, so
  `window-jump list` / `next` / `prev` work on Plasma Wayland. (#8)
- Hotkeys through `org.freedesktop.portal.GlobalShortcuts`: set
  `WINDOW_JUMP_HOTKEYS=portal` to let KDE or GNOME 48+ register the bindings,
  with the desktop's confirmation dialog. GNOME still uses the extension by
  default; other desktops try the portal automatically. (#11)
- Optional `libinput` cargo feature: on desktops without the GNOME extension
  (KDE, wlroots, X11 window managers) the daemon can read touchpad swipes
  directly from libinput; needs the user in the `input` group. The compositor
  still sees the same gesture, so use a modifier the desktop does not use for
  its own gesture. (#10)
- Default swipes: Alt + three fingers up / down jump to the next / previous
  window of the app, next to Ctrl + three fingers left / right. Plain three
  fingers up still open GNOME's overview. A config file with its own
  `[[swipe]]` tables is not affected. (#30)

### Changed

- Linux backend selection: GNOME (the default), KDE Plasma, then
  Sway/Hyprland/niri. `window-jump doctor` names the detected backend and
  reports portal and libinput availability.

### Tests

- The NixOS VM touchpad test also covers Alt + three fingers up/down, and CI
  builds and tests the optional `libinput` feature. (#30, #10)

## [0.3.0] - 2026-11-22

### Added

- Windows: windows on other virtual desktops are listed and can be jumped to;
  `workspace = "current"` still excludes them. Windows has no documented API to
  switch desktops, so activation sends `Win+Ctrl+Left/Right` as a best effort
  and reports an error when the window cannot be reached. (#1)
- Windows: native Precision Touchpad swipes through raw HID input, so the same
  `[[swipe]]` bindings as on GNOME work. Windows' own three/four-finger
  gestures must be turned off first. (#2)
- Windows: a notification-area icon for `window-jumpw.exe` with the hotkey
  status, *Reload config*, *Open config file*, *Start at login* and *Quit*. (#3)

### Changed

- `order = "mru"` works like Alt+Tab: presses while the hotkey's modifiers
  stay down form one session (each goes one window further back, however
  slowly they come), and releasing the modifiers ends it. The windows passed
  on the way do not count as used: the next press goes back to where the
  session started. A swipe is one session while the fingers stay down.
  `mru_session_ms` now only applies where key releases cannot be seen
  (`window-jump next` from another shortcut tool, hotkeys without
  modifiers). Extension protocol 3 adds the `AcceleratorReleased` signal; with
  an older extension sessions go by time as before. (#6)

### Tests

- A NixOS VM test boots a full GNOME session and swipes on a virtual
  (uinput) touchpad: Ctrl + three fingers must jump to the app's other window
  without switching workspace, plain three fingers must still switch it.
  Runs in CI. (#26)

## [0.2.0] - 2026-11-21

### Added

- GNOME Shell 51 support. CI now runs the end-to-end test on GNOME 46, 48,
  49, 50 and 51. (#7)
- The daemon reloads `config.toml` when it changes (checked every two
  seconds) and registers the new hotkeys and swipes without a restart. An
  invalid edit is reported once and the previous config stays in effect; a
  removed file means the defaults. (#4)
- GNOME: an overlay after each jump (app icon, window title, "2 / 5"),
  `[hud] enabled` / `duration_ms` in the config. Extension protocol 2 adds
  `ShowHud`; the binary still works with a protocol 1 extension, without the
  overlay. (#5)
- Windows: the same overlay from the daemon, as a layered, click-through,
  topmost window that never takes focus. (#5)

### Changed

- The Nix package no longer installs `window-jumpw` on Linux (it is the
  Windows console-less daemon).

### Tests

- The Windows end-to-end test now starts the daemon, types its hotkey with
  `SendInput` and checks the jump and the overlay (#20). The GNOME one types
  hotkeys on a virtual keyboard, rewrites the config under a running daemon,
  and runs on GNOME 46, 48, 49, 50 and 51.

## [0.1.0] - 2026-11-21

First release.

### Added

- `window-jump next` / `prev`: focus the next / previous window of the focused
  app, in `stable`, `position` or `mru` order, on all workspaces or the
  current one, with or without minimized windows.
- `window-jump daemon`: global hotkeys (default `Super+Alt+N` / `Super+Alt+P`)
  and, on GNOME, modifier + touchpad swipes (default `Ctrl` + three fingers
  left / right) that keep stepping while the fingers stay down.
- GNOME Shell extension `window-jump@hjosugi.github.io` (GNOME 46–50): window
  list and activation, accelerator grabs and swipe claims on the session bus,
  released as soon as the daemon exits.
- Windows backend: Alt+Tab-style window list grouped by executable (UWP apps
  resolved behind ApplicationFrameHost), `RegisterHotKey`, foreground
  activation; `window-jumpw.exe` runs the daemon without a console window.
- `list`, `activate`, `doctor`, `config init|path|check`,
  `autostart enable|disable`.
- Release binaries: static Linux (x86_64, aarch64), Windows (x86_64, aarch64);
  Nix flake with the binary and the extension.
