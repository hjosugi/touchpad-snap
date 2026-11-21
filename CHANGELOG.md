# Changelog

## [Unreleased]

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
