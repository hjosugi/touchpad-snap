# Changelog

## [Unreleased]

### Added

- Default swipes: Alt + three fingers up / down jump to the next / previous
  window of the app, next to Ctrl + three fingers left / right. Plain three
  fingers up still open GNOME's overview. A config file with its own
  `[[swipe]]` tables is not affected. (#30)

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
