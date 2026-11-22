# Changelog

## [Unreleased]

### Changed

- The configuration is now `config.json`: a list of bindings, each a trigger
  (swipe or hotkey), an optional `name`/`enabled`/`repeat`/`when`, and an
  ordered action chain. This release supports `window.next`, `window.prev` and
  `delay`; `keys`, `exec`, window/workspace actions and `when` conditions follow
  in the next issues. A malformed or unsupported binding is reported and
  skipped instead of failing the whole file, and a committed JSON Schema
  (`schema/config.schema.json`, `touchpad-snap config schema`) documents the
  format. The old `config.toml` is still read in place; `touchpad-snap config
  import` writes the JSON version. (#32)

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
