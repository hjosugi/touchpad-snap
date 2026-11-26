# touchpad-snap

Bind touchpad gestures and keyboard shortcuts to actions. Switch between the
windows **of the app you are in**, jump between apps, tile, close or maximize,
switch workspaces or the overview, send a shortcut or run a command — with a
hotkey, a modifier + touchpad swipe, a pinch, a hold or a mouse button. — like `Cmd+`` ` on macOS or the "cycle windows of
the current app" action in BetterTouchTool, for **GNOME (Wayland and X11)**,
**KDE Plasma**, **Sway, Hyprland and niri**, and **Windows**. Written in Rust;
a small GNOME Shell extension or KWin script bridges to the compositor where
Wayland allows nothing else.

[日本語の README はこちら](README.ja.md)

- **Hotkeys**: `Super+Alt+N` / `Super+Alt+P` by default (configurable).
- **Swipes on GNOME**: hold `Ctrl` and swipe left or right with three
  fingers, or hold `Alt` and swipe up or down (configurable). Only the
  swipes you claim (fingers + modifiers) are taken from GNOME; a plain
  three-finger swipe still switches workspace or opens the overview. Claims
  can be limited to an app (see [`when`](#conditions-when)): in other apps
  the gesture stays GNOME's. Keep the fingers down and keep
  moving to step through several windows; move back to step back.
- **Swipes on Windows**: the same modifier + three-finger swipe works on a
  Windows Precision Touchpad; Windows' own three/four-finger gestures must be
  turned off first (see below).
- **Swipes elsewhere**: an optional `--features libinput` build reads swipes
  straight from libinput on KDE, Sway/Hyprland/niri and X11 window managers
  (see [libinput gestures](#libinput-gestures-kde-swayhyprlandniri-x11)).
  On GNOME the extension path is preferred.
- **Overlay**: after each jump the window title and "App · 2 / 5" (plus the
  app icon on GNOME) show briefly in the middle of the screen, so you can
  follow where you are while scrubbing. On Windows this needs the daemon.
- **Orders**: `stable` (fixed order, walks every window), `position` (left to
  right on screen) or `mru` (like Alt+Tab: one press toggles between the two
  most recent windows; pressing again while holding the modifiers digs
  deeper, and releasing settles there).
- **Any binder**: `touchpad-snap next` / `touchpad-snap prev` are plain commands,
  so they also work from GNOME's own custom shortcuts, AutoHotkey, keyd, a
  mouse driver…

## How it works

```
GNOME                                           Windows
─────                                           ───────
touchpad-snap next / daemon                       touchpad-snap next / daemon
        │  D-Bus (session bus)                          │  Win32
        ▼                                               ▼
touchpad-snap@hjosugi.github.io                   EnumWindows, SetForegroundWindow,
(GNOME Shell extension, inside the compositor)  RegisterHotKey, Raw Input
 · List / Activate windows                       (Precision Touchpad)
 · grabs the daemon's hotkeys
 · claims modifier + touchpad swipes
```

On Wayland a normal program cannot list or focus other programs' windows,
grab global keys or see the touchpad gestures GNOME uses for itself. The
extension runs inside GNOME Shell and offers exactly that on the session bus;
all the logic (which window is next, what a swipe means, the config) is in the
Rust program. Everything the daemon registered is dropped the moment it exits.

On KDE Plasma a small KWin script plays the extension's role (see
[KDE Plasma](#kde-plasma)); hotkeys come from the desktop's GlobalShortcuts
portal, so `touchpad-snap daemon` works there too.

## Install

### GNOME (any distribution)

1. Download from the [latest release](https://github.com/hjosugi/touchpad-snap/releases/latest):
   - `touchpad-snap-<version>-x86_64-unknown-linux-musl.tar.gz` (or `aarch64`) —
     a static binary, no dependencies. Put `touchpad-snap` on your `PATH`, e.g.
     `~/.local/bin`.
   - `touchpad-snap@hjosugi.github.io.shell-extension.zip`
2. Install the extension, then **log out and back in** (GNOME on Wayland only
   discovers new extensions at login):
   ```sh
   gnome-extensions install --force touchpad-snap@hjosugi.github.io.shell-extension.zip
   # after logging back in:
   gnome-extensions enable touchpad-snap@hjosugi.github.io
   ```
3. Check, start the daemon, and have it start at every login:
   ```sh
   touchpad-snap doctor
   touchpad-snap autostart enable     # ~/.config/autostart/touchpad-snap.desktop
   touchpad-snap daemon &             # for this session
   ```

On Linux, `touchpad-snap autostart enable --systemd` writes a
`~/.config/systemd/user/touchpad-snap.service` user unit instead and enables it.
systemd starts it with the graphical session, restarts it if it crashes
(`Restart=on-failure`), and collects its output in the journal:
`systemctl --user status touchpad-snap`, `journalctl --user -u touchpad-snap`.
Remove it with `touchpad-snap autostart disable --systemd`. Use either the XDG
entry or the systemd unit, not both. The release tarball also ships the unit as
`touchpad-snap.service` for package installs.

Supported GNOME Shell versions: 46–51. CI runs the end-to-end test on 46 (Ubuntu 24.04), 48 (Debian 13), 49 (Fedora 43), 50 (Fedora 44) and the latest on Arch Linux (51 at the time of writing); 47 is not tested.

### NixOS / Nix flake

The flake exports a NixOS module and a Home Manager module, each as
`default` and `touchpad-snap`:

```nix
# flake.nix
{
  inputs.touchpad-snap.url = "github:hjosugi/touchpad-snap";

  outputs = { self, nixpkgs, touchpad-snap, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      modules = [
        touchpad-snap.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

```nix
# configuration.nix
services.touchpad-snap = {
  enable = true;
  settings = {
    cycle = { order = "mru"; };
    bindings = [
      {
        trigger = { type = "hotkey"; keys = "Super+Alt+N"; };
        actions = [ { type = "window.next"; } ];
      }
    ];
  };
};
```

This:

- installs the `touchpad-snap` package and runs `touchpad-snap daemon` as a
  systemd **user** service, bound to `graphical-session.target` with
  `Restart=on-failure`;
- renders `settings` (the `config.json` schema) to
  `/etc/touchpad-snap/config.json` and starts the daemon with
  `--config /etc/touchpad-snap/config.json`;
- installs the GNOME Shell extension and adds `touchpad-snap@hjosugi.github.io`
  to the **system default** of `org.gnome.shell.enabled-extensions` through
  `programs.dconf.profiles.user.databases`.

Options: `enable` (default `false`), `package`, `settings` (default `{}`),
`systemd.enable` (default `true`) and `gnome.enable` (default `true`). When
`gnome.enable` is true, `gnome.extensionPackage` chooses the extension
package. Set `systemd.enable = false` to manage the daemon yourself, or
`gnome.enable = false` on a non-GNOME desktop.

The dconf database is a *default* behind `user-db:user`. A user who has
already written `enabled-extensions` (for example by toggling extensions in
the Extensions app) keeps their own value and the system default is ignored.
Log out and back in after the first switch, then either enable the extension
once:

```sh
gnome-extensions enable touchpad-snap@hjosugi.github.io
```

or clear the per-user key so the system default applies again:

```sh
dconf reset /org/gnome/shell/enabled-extensions
```

If you do not want to use the module, the packages are still exported
manually:

```nix
environment.systemPackages = [
  touchpad-snap.packages.${pkgs.stdenv.hostPlatform.system}.touchpad-snap
  touchpad-snap.packages.${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension
];
```

Then enable the extension and run `touchpad-snap autostart enable` as usual.
Outside NixOS: `nix run github:hjosugi/touchpad-snap -- doctor`.

#### Home Manager

```nix
{
  imports = [ inputs.touchpad-snap.homeManagerModules.default ];

  services.touchpad-snap = {
    enable = true;
    settings = {
      cycle = { order = "mru"; };
      bindings = [
        {
          trigger = { type = "hotkey"; keys = "Super+Alt+N"; };
          actions = [ { type = "window.next"; } ];
        }
      ];
    };
  };
}
```

The option names match the NixOS module. Home Manager renders the config to
`$XDG_CONFIG_HOME/touchpad-snap/config.json` (usually
`~/.config/touchpad-snap/config.json`), which is the path the daemon reads by
default, so the user service runs plain `touchpad-snap daemon`.
`gnome.enable` uses Home Manager's `programs.gnome-shell.extensions`, which
also writes `org/gnome/shell/enabled-extensions`. That list is managed as a
whole: add any other extensions you use to `programs.gnome-shell.extensions`
too, rather than expecting the module to append to the existing list.

Historically the program was called **window-jump**, and the binary copies
`~/.config/window-jump/config.toml` to
`~/.config/touchpad-snap/config.toml` on first run. That TOML file is still
read in memory, but run `touchpad-snap config import` once to write the new
`config.json`. Both modules write the new file themselves (and NixOS passes an
explicit `--config`), so that automatic copy does not happen; move any old
settings into `settings`.

### Windows 10 / 11

1. Download `touchpad-snap-<version>-x86_64-pc-windows-msvc.zip` (or
   `aarch64`) and unpack it somewhere permanent, e.g.
   `%LOCALAPPDATA%\Programs\touchpad-snap`.
2. Start at login, and now:
   ```powershell
   .\touchpad-snap.exe autostart enable   # runs touchpad-snapw.exe (no console window) at login
   .\touchpad-snapw.exe                    # start it now
   ```
3. Touchpad (optional): swipes need a **Windows Precision Touchpad** (most
   modern laptops have one). Turn off the built-in **Three-finger gestures** and
   **Four-finger gestures** under **Settings → Bluetooth & devices → Touchpad**
   (set them to *Nothing*), then use the same `swipe` bindings as on GNOME:
   with the default config, hold `Ctrl` and swipe three fingers left or right,
   or hold `Alt` and swipe them up or down.
   Windows' own gestures still fire if they stay enabled; raw input cannot
   intercept them, so both actions would happen.

`touchpad-snapw.exe` shows a notification-area icon. Its menu reports how many
hotkeys were registered (and names any another app already took), and offers
**Reload config**, **Open config file**, **Start at login** and **Quit** — no
Task Manager needed to stop the daemon.

Windows are grouped by executable (`notepad.exe`, `Code.exe`…); UWP apps are
looked up behind their ApplicationFrameHost frame.

Windows on other virtual desktops are included when `cycle.workspace` is
`"all"` (the default) and skipped when it is `"current"`. The current desktop is `ws 0`;
other desktops get positive numbers. Jumping to such a window is best effort:
Windows has no documented API to switch desktops, so touchpad-snap sends
`Win+Ctrl+Left/Right` (at most eight steps each way) and reports an error if
the window cannot be reached. Suspended UWP windows stay hidden.

Manual test: create a second desktop with `Win+Ctrl+D`, open a second Notepad
there (or move one with Task View), return with `Win+Ctrl+Left`, then run
`touchpad-snap list` (the second Notepad shows a positive `ws`) and
`touchpad-snap next` from the first Notepad.

### From source

```sh
cargo install --git https://github.com/hjosugi/touchpad-snap touchpad-snap
```

The GNOME extension is the `gnome-extension/touchpad-snap@hjosugi.github.io`
directory: copy it to `~/.local/share/gnome-shell/extensions/`.

### Upgrading from window-jump

touchpad-snap was called **window-jump** before version 0.5. The program is the
same; every name changed with it. After installing touchpad-snap as above:

- **Settings**: `~/.config/window-jump/config.toml` (`%APPDATA%\window-jump` on
  Windows) is copied to the new `touchpad-snap` directory the first time
  touchpad-snap runs. The old file is left alone and no longer read.
- **GNOME**: switch extensions, then log out and back in if the new one is not
  found:
  ```sh
  gnome-extensions disable window-jump@hjosugi.github.io
  gnome-extensions enable touchpad-snap@hjosugi.github.io
  ```
- **Start at login**: `touchpad-snap autostart enable` replaces window-jump's
  entry. On Windows, quit window-jump from its tray icon first. With
  `--systemd`, an old `window-jump.service` user unit is removed as well.
- **Portal hotkeys**: the variable is now `TOUCHPAD_SNAP_HOTKEYS`
  (`WINDOW_JUMP_HOTKEYS` is still read), and the desktop asks once more to
  confirm the bindings, as the app id changed with the name.
- `touchpad-snap doctor` lists whatever of window-jump is still active.
- Remove the old binary (`cargo uninstall window-jump`, or delete
  `window-jump.exe` and `window-jumpw.exe`) and
  `~/.local/share/gnome-shell/extensions/window-jump@hjosugi.github.io`.

## Usage

| Command | |
|---|---|
| `touchpad-snap next` / `prev` | Focus the next / previous window of the focused app. `--order`, `--workspace` override the config. |
| `touchpad-snap next-app` / `prev-app` | Focus the most recently used window of the next / previous app. |
| `touchpad-snap daemon [-v]` | Serve the configured hotkeys and swipes. |
| `touchpad-snap list [--json]` | Show windows grouped by app. |
| `touchpad-snap activate <id>` | Focus one window by id. |
| `touchpad-snap doctor` | Check config, backend, daemon and window list. |
| `touchpad-snap status [--json]` | Ask the running daemon for its version, backend, bindings and hotkeys it could not grab. |
| `touchpad-snap run <action>` / `--binding <name>` | Run one action (or a binding) through the running daemon. |
| `touchpad-snap events` | Stream the hotkeys, swipes, pinches, holds and mouse triggers the daemon sees, as JSON lines. |
| `touchpad-snap record [--timeout N]` | Claim the next gesture for a moment and print it as a trigger (used by the settings window). |
| `touchpad-snap settings` | Open the settings window (needs `touchpad-snap-settings`, installed by `task install` or `cargo install --path crates/gui`). |
| `touchpad-snap config init\|import\|schema\|path\|check` | Write / import / print the schema of / locate / validate the config file. |
| `touchpad-snap autostart enable\|disable [--systemd]` | Start the daemon at login (XDG entry by default, systemd user unit with `--systemd`). |

### Settings window

`touchpad-snap settings` (or the **touchpad-snap Settings** desktop entry, or
**Settings…** in the Windows tray menu) lists the bindings, edits one
(trigger, app conditions, action chain, name, repeat, enable), and changes the
general cycle/gesture/HUD settings. It writes the same `config.json` the daemon
watches, so a save applies within about two seconds; **Reload** re-reads a file
edited elsewhere. Problems from the same loader as `config check` are shown in
the window, and **Check daemon** asks the running daemon for its status. The
window is available in English and Japanese. **Try** runs the selected binding
through the daemon, **Live** streams what the daemon sees (hotkeys, swipes,
pinches, holds, mouse triggers), and bindings can be exported to or imported
from a JSON file; importing lists every `exec` command before anything is
added. **Record gesture** asks the daemon to claim every swipe, pinch and hold
for a moment and fills the trigger from what you performed (GNOME);
**Capture shortcut** fills a hotkey trigger from the keys pressed;
**Open apps** offers the running apps as `when.app` patterns; **Undo** steps
back over the last save.

The GUI is a separate binary: on Linux it is built from source (`cargo install
--locked --path crates/gui`, or `nix build .#touchpad-snap-gui`) because the
static musl daemon build cannot link the GUI's `dlopen` dependencies; the
Windows release zip ships `touchpad-snap-settings.exe`.

### Without the daemon

Bind the commands to any shortcut tool instead. GNOME: **Settings → Keyboard →
View and Customise Shortcuts → Custom Shortcuts**, command `touchpad-snap next`.
(The extension is still needed on GNOME, for listing and focusing windows.)

## Sway, Hyprland, niri

Sway, Hyprland and niri expose their window lists and focus over their own
IPC, so touchpad-snap works there without an extension and without the daemon.
It detects the compositor from `SWAYSOCK`, `HYPRLAND_INSTANCE_SIGNATURE` or
`NIRI_SOCKET`, falling back to `XDG_CURRENT_DESKTOP` to explain a missing
socket. `touchpad-snap list`, `next`, `prev`, `activate` and `doctor` work as
usual.

Global hotkeys have no portable API on these compositors, so bind the command
in the compositor's own config instead of running `touchpad-snap daemon`.

Sway (`~/.config/sway/config`):

```
bindsym $mod+n exec touchpad-snap next
bindsym $mod+p exec touchpad-snap prev
```

Hyprland (`~/.config/hypr/hyprland.conf`):

```
bind = SUPER, N, exec, touchpad-snap next
bind = SUPER, P, exec, touchpad-snap prev
```

niri (`~/.config/niri/config.kdl`):

```
binds {
    MOD+N { spawn "touchpad-snap" "next"; }
    MOD+P { spawn "touchpad-snap" "prev"; }
}
```

`touchpad-snap daemon` is not needed for hotkeys on these compositors; it
prints a reminder to bind the commands in the compositor config. Built with the
optional `libinput` feature it stays running for touchpad swipes (next
section); without it, it only prints the reminder. The overlay is a
GNOME/Windows feature for now. `touchpad-snap doctor` names the compositor it
detected.

## KDE Plasma

Plasma does not support GNOME Shell extensions. Instead, touchpad-snap writes a
small KWin script to `$XDG_RUNTIME_DIR/touchpad-snap/` and loads it on demand
through `org.kde.KWin.Scripting` under a per-process name. The script sends
`workspace.windowList()` over a private session-bus service and applies the
focus command that comes back. Nothing is installed in System Settings and
nothing is enabled there: the script exists only while the `touchpad-snap`
command (or the daemon) is running, and is unloaded again when it exits.
`touchpad-snap list`, `next`, `prev`, `activate` and `doctor` work as usual, and
`doctor` reports the detected session and whether KWin scripting is reachable.
MRU order is best effort, taken from KWin's stacking order.

`touchpad-snap daemon` takes its hotkeys from the
[GlobalShortcuts portal](#globalshortcuts-portal-kde-gnome-48) (the desktop
shows a confirmation dialog), and uses the KWin bridge for listing and
focusing windows. This needs `xdg-desktop-portal-kde` running. If the portal is
unavailable, the daemon cannot grab global hotkeys on KDE: bind
`touchpad-snap next` / `touchpad-snap prev` as command shortcuts in **System
Settings → Shortcuts** instead, and do not run the daemon. Touchpad swipes and
the overlay are GNOME/Windows features for now; KDE only lists and focuses
windows.

The bridge is normally cleaned up automatically. If touchpad-snap was killed
hard, its script stops polling but stays registered until KWin restarts. Log
out and back in (KWin scripts loaded this way are not persistent), or unload it
with the pid of the process that loaded it:

```sh
qdbus6 org.kde.KWin /Scripting unloadScript touchpad-snap-kde-bridge-<pid>
```

The generated file can be removed with:

```sh
rm -f "$XDG_RUNTIME_DIR"/touchpad-snap/kde-bridge-*.js
```

## GlobalShortcuts portal (KDE, GNOME 48+)

Normally `touchpad-snap daemon` grabs hotkeys through the GNOME extension. To let
the desktop register them instead, set `TOUCHPAD_SNAP_HOTKEYS=portal`:

```sh
TOUCHPAD_SNAP_HOTKEYS=portal touchpad-snap daemon
```

The desktop then shows a confirmation dialog and remembers the bindings. On
KDE this is how the daemon gets its hotkeys; the KWin script described above is
only for window listing and focus, and GNOME 48+ implements the same portal. On
GNOME the extension is still used for window listing, the HUD and swipes — only
the hotkeys move to the portal. When the variable is unset, GNOME keeps using
the extension's grabs and other desktops try the portal automatically; on
Sway, Hyprland and niri the compositor config bindings above are the usual
choice. `touchpad-snap doctor` reports whether the portal is available and whether the
app id (`io.github.hjosugi.TouchpadSnap`) was registered; desktops that require it
accept the id when it matches a `.desktop` file. Changing the hotkey list takes
effect after a daemon restart, since a portal session is bound only once.

## libinput gestures (KDE, Sway/Hyprland/niri, X11)

GNOME's extension and Windows Precision Touchpads provide swipes out of the
box. Elsewhere the daemon can read gestures directly from libinput, like
libinput-gestures, if touchpad-snap is built with the optional `libinput` feature:

```sh
cargo install --locked --path . --features libinput   # from a checkout
# or
cargo build --release --features libinput
```

The `input` crate uses libinput and libudev, so the development files are
needed to build (`libinput-dev libudev-dev pkg-config` on Debian/Ubuntu,
`libinput-devel libudev-devel` on Fedora, or `nix develop` in this repository).

Reading `/dev/input/event*` needs the user in the `input` group:

```sh
sudo usermod -aG input "$USER"
```

Log out and back in for the new group to take effect. `touchpad-snap doctor` has
a `gestures` line: whether the feature is compiled in, whether a touchpad was
found, and whether `/dev/input` is readable. Group membership lets your user
read every input device, keyboards included; use a narrower udev rule instead
if that trade-off is not acceptable.

Then run the daemon as usual. On Sway, Hyprland and niri the hotkeys still come
from the compositor config; on KDE and other portal desktops from
GlobalShortcuts. The daemon serves the configured `swipe` bindings from the
touchpad. On GNOME keep using the extension: it can claim swipes, so the shell
does not also act on them.

libinput cannot stop the compositor from seeing the same gesture. touchpad-snap
does not intercept it, so the compositor may switch workspace, move a window
or do whatever else it has bound to those fingers at the same time. Use a
modifier + finger count the compositor does not use for its own gestures (the
default `Ctrl` + three fingers is usually safe), or turn the compositor
gesture off. This is the same limitation as Windows Precision Touchpad input.

## Configuration

`touchpad-snap config init` writes the default JSON file to
`~/.config/touchpad-snap/config.json`
(`%APPDATA%\touchpad-snap\config.json` on Windows). Everything is optional;
these are the defaults, with the bindings spelled out. Omit a field to keep its
default; an explicit `"bindings": []` turns all bindings off.

```json
{
  "$schema": "https://raw.githubusercontent.com/hjosugi/touchpad-snap/main/schema/config.schema.json",
  "version": 2,
  "cycle": {
    "order": "stable",
    "workspace": "all",
    "include_minimized": true,
    "mru_session_ms": 1000
  },
  "gesture": { "threshold": 120, "repeat_step": 220 },
  "hud": { "enabled": true, "duration_ms": 700 },
  "bindings": [
    {
      "name": "Next window of this app",
      "trigger": { "type": "hotkey", "keys": "Super+Alt+N" },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "name": "Previous window of this app",
      "trigger": { "type": "hotkey", "keys": "Super+Alt+P" },
      "actions": [ { "type": "window.prev" } ]
    },
    {
      "name": "Next window (Ctrl + swipe left)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "left" },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "name": "Previous window (Ctrl + swipe right)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "right" },
      "actions": [ { "type": "window.prev" } ]
    },
    {
      "name": "Next window (Alt + swipe up)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "up" },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "name": "Previous window (Alt + swipe down)",
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["alt"], "direction": "down" },
      "actions": [ { "type": "window.prev" } ]
    }
  ]
}
```

- `trigger` is a hotkey, swipe, pinch, hold, sequence, modifier double-tap or
  mouse button (see [Triggers and actions](#triggers-and-actions)).
- `actions` is a chain run in order: window and app cycling, window management,
  workspace and overview actions, `delay`, `keys` and `exec`.
- `window.next` / `window.prev` stay within the focused app's windows;
  `app.next` / `app.prev` move to the most recently used window of the next /
  previous **app**. Apps are ordered by their most recent window, while
  `cycle.workspace` and `cycle.include_minimized` still apply; the session
  rules are the same, so a held trigger freezes the order and steps one app
  further per repeat, and the HUD counts apps.
- `keys` types a keyboard shortcut into the focused window,
  `{ "type": "keys", "keys": "Ctrl+Shift+Tab" }`: the same syntax as hotkeys
  (modifiers, then a letter, digit, `F1`–`F24`, an arrow, `Tab`, `Space`,
  `Return`, `Escape`, `Home`, `End`, `PageUp`, `PageDown`, `Backspace`,
  `Delete` or punctuation). The trigger's own modifiers are not part of the
  shortcut: `Alt` + swipe sending `Ctrl+Tab` arrives as `Ctrl+Tab`. On Windows
  they are released for the shortcut and pressed again; on GNOME a virtual
  keyboard cannot release a key you hold, so the shortcut is typed when you
  let go of the modifier (and dropped if you hold it for more than 3 s).
  Needs the GNOME extension with protocol 9 (log in again after updating).
  Other desktops report the action as unsupported.
- `name` is optional and is only used in logs and `config check`.
  `enabled: false` keeps a binding in the file without registering it.
- `repeat` says whether a swipe binding fires again every further
  `gesture.repeat_step` while the fingers stay down. When omitted it is true
  for chains containing `window.next` / `window.prev` / `app.next` /
  `app.prev`, false otherwise.
- `when` (optional) restricts a binding to the focused app, the OS and/or the
  desktop. See [Conditions (`when`)](#conditions-when) below.
- A claimed swipe with `Alt` or `Super` held is safe. When the swipe ends,
  touchpad-snap taps a key no application binds (`F19` on GNOME, virtual key
  `0xE8` — the AutoHotkey trick — on Windows) while the modifier is still
  down. A claimed swipe is not a key, so without that tap your eventual
  release would look like a lone modifier press and open the window's menu bar
  (`Alt`) or the overview/Start (`Super`). The tap is skipped if you released
  the modifier first, and is never sent for hotkeys or `keys` actions (those
  already press a real key). A rare app that binds `F19` itself may still see
  the tap.
- `cycle`, `gesture` and `hud` work as before: `cycle.order` is `stable`,
  `position` or `mru`; `cycle.workspace` is `all` or `current`;
  `gesture.threshold` is the finger travel before a swipe counts (GNOME's full
  workspace swipe is 400); `gesture.repeat_step` is the further travel per
  extra jump (0 = one jump per swipe); `hud` controls the overlay after a jump.

### `exec`: run a command

`exec` runs a program when its gesture or hotkey fires. Give the program and
its arguments as a list to run it directly, with no shell to split or expand
anything:

```json
{ "type": "exec", "command": ["ghostty", "--working-directory", "~"] }
```

A single string is run by `sh -c` on Linux and `cmd /C` on Windows, and needs
`"shell": true`:

```json
{ "type": "exec", "command": "notify-send \"$TOUCHPAD_SNAP_WINDOW_TITLE\"", "shell": true }
```

A string without `"shell": true`, or a list with it, is a binding error.

The command is detached: it gets its own process group on Linux and no console
window on Windows, with stdin at `/dev/null` (or `NUL`). The daemon does not
wait for it; a separate thread reaps it, and `touchpad-snap daemon --verbose`
logs its exit status and stderr. The child receives the focused window from
the moment the trigger started, and the trigger itself:

| Variable | Value |
| --- | --- |
| `TOUCHPAD_SNAP_WINDOW_ID` | The focused window's id |
| `TOUCHPAD_SNAP_APP_ID` | Its app id (`org.gnome.TextEditor.desktop`, an executable path…) |
| `TOUCHPAD_SNAP_APP_NAME` | The app name |
| `TOUCHPAD_SNAP_WINDOW_TITLE` | The window title |
| `TOUCHPAD_SNAP_TRIGGER` | The trigger, e.g. `swipe-3-alt-up` or `hotkey-Super+Alt+N` |

Only the variables a window provides are set; the daemon's own environment is
never changed. An `exec` binding fires once per swipe unless `"repeat": true`
(the default stays true for chains containing `window.next` / `window.prev` /
`app.next` / `app.prev`).
`config check` warns when the program of an argument list is not on `PATH`;
a shell string cannot be checked and is only reported as such.

**Only run config files you trust.** A config can run any command as you: an
`exec` action is not sandboxed. Before importing someone else's config, read
its `exec` actions; the GUI must show them before importing too.
### Conditions (`when`)

One trigger can do different things depending on where it happens:

```json
{
  "bindings": [
    {
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "down" },
      "when": { "app": ["firefox", "org.chromium.Chromium"] },
      "actions": [ { "type": "window.next" } ]
    },
    {
      "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "down" },
      "actions": [ { "type": "window.prev" } ]
    }
  ]
}
```

- `app`: app patterns (case-insensitive, `*` is a wildcard) matched against
  the desktop-file id (`list` prints it as `[org.gnome.TextEditor.desktop]`),
  the app name and the WM class. On Windows, ids are executable paths
  (`code.exe`), matched the same way. Matching all three means `firefox`,
  `Firefox` and `firefox.desktop` all work.
- `not_app`: apps to exclude, with the same patterns.
- `os`: `linux` or `windows` (or `*`).
- `desktop`: `gnome`, `kde`, `sway`, `hyprland`, `niri` (or `*`). This makes
  one config file work on every machine.
- An empty or absent `when` applies everywhere. All present conditions must
  match.

Among enabled bindings with the same trigger, a binding whose `when` matches
wins over one without `when`; ties go to the first in the file. If none
matches, nothing fires. The focused app and platform are captured when the
trigger starts and kept for the whole gesture. Duplicate hotkeys are allowed
for this reason: bind one key once per app and leave an unconditional fallback.

`config check` reports bindings whose conditions can never fire: an unknown
`os`/`desktop` value, the same pattern in both `app` and `not_app`, or
`not_app: ["*"]`. It also lists bindings that are merely inactive on this
machine (for example a `windows` binding when run on Linux). An app pattern
that matches no window can only be known at run time and is not reported.

**GNOME claims are per app.** A swipe bound only for Firefox is sent to the
extension with its app filter: the extension checks the focused app at the
start of the swipe and leaves the gesture to GNOME in every other app. Plain
three-finger workspace switching therefore keeps working where your binding
does not apply. This uses protocol 4; an older extension still runs, but
app-filtered swipes are then claimed in every app (and do nothing outside
theirs), and the daemon says so at startup. A `not_app`-only binding cannot be
expressed as a positive GNOME claim: it is claimed everywhere and does nothing
in the excluded apps.

On GNOME a claim is per (fingers, modifiers, app), not per direction: a plain
three-finger binding for an app claims plain three-finger swipes in every
direction in that app, so its unbound directions do nothing instead of reaching
GNOME.

Windows' raw touchpad listener cannot hand a gesture back: once the listener
sees a swipe, Windows' own gesture handling already ran too. Turn Windows'
three/four-finger gestures off, and use a modifier plus fingers for bindings
that should not overlap the shell's gestures. The same applies to the optional
`libinput` backend: it observes swipes without intercepting them, so the
compositor still acts on the same gesture.

The file is strict inside objects it knows: a typo or an unknown field is an
error naming the JSON path. Bindings are decoded one by one, though: one bad
binding is reported and skipped, and the others still load. `config check`
lists every `bindings[i]` problem, warns about `exec` programs that are not on
`PATH`, and exits non-zero only for malformed bindings; an unknown
trigger/action type or an unsupported `when` key (from a newer build) is a
warning, is skipped, and does not fail the check. The daemon does the same and
keeps running with the valid bindings. `config schema` prints the JSON Schema
generated from the program's types; the committed
`schema/config.schema.json` is what the `$schema` line points at, so editors
can validate and complete the file.

Editing the file while the daemon runs: changes are picked up within about two
seconds. A broken edit is reported in the daemon log and the previous config
stays in effect. Removing the file returns to the built-in defaults.

### Upgrading from `config.toml`

A `config.toml` next to `config.json` is still read in memory when
`config.json` does not exist (and an explicit `--config something.toml` is
read as TOML too). `touchpad-snap doctor` and `config check` say when that
happens. Run `touchpad-snap config import` to convert it and write
`config.json` (`--force` overwrites an existing JSON file). `config init`
writes the JSON defaults; `config path` prints the JSON path.

Tips:

- `Alt` and `Super` alone are fine as swipe modifiers for claimed swipes:
  before you let go, touchpad-snap taps a harmless key so the release is not a
  lone modifier press (which would open a menu bar, the overview or Start).
  Apps that bind `F19` themselves are the unusual exception.
- On Windows, turn off the built-in three/four-finger gestures first.
  Precision Touchpads report their contacts as raw HID input, and Windows'
  shell sees the same gesture independently — it cannot be intercepted.
- If a hotkey cannot be grabbed (GNOME or another app already uses it), the
  daemon says so at startup; `touchpad-snap daemon -v` logs every event.

## Triggers and actions

Every trigger has `type`; modifiers are `ctrl`, `alt`, `shift`, `super`, and
any trigger can carry a `when` (`app`/`not_app`, `os`, `desktop`).

| Trigger | Fields | Platforms |
|---|---|---|
| `hotkey` | `keys`: `"Super+Alt+N"` | all |
| `swipe` | `fingers` 3–5, `modifiers`, `direction`: left/right/up/down | GNOME, Windows touchpad, libinput |
| `pinch` | `fingers` 2–5, `modifiers`, `direction`: `in` / `out` | GNOME, Windows, libinput |
| `hold` | `fingers` 2–5, `modifiers` | GNOME, Windows, libinput |
| `sequence` | `keys`: prefix hotkey, `then`: one letter or digit | GNOME, Windows |
| `double_tap` | `modifier`: `ctrl` / `alt` / `shift` / `super` | GNOME, Windows |
| `mouse` | `button`: left/right/middle/back/forward, `modifiers`, `scroll`: up/down/left/right | Windows |

| Action | Fields | Platforms |
|---|---|---|
| `window.next` / `window.prev` | | all |
| `app.next` / `app.prev` | | all |
| `window.minimize` | | GNOME, Windows |
| `window.maximize` / `window.restore` / `window.toggle_maximize` | | GNOME, Windows |
| `window.fullscreen` | | GNOME (bind `keys` with F11 on Windows) |
| `window.close` | never repeats within one gesture | GNOME, Windows |
| `window.tile` | `to`: left/right/top/bottom/top-left/top-right/bottom-left/bottom-right | GNOME, Windows |
| `window.move_to_monitor` | `to`: next/prev | GNOME, Windows |
| `window.move_to_workspace` | `to`: next/prev | GNOME (no public Windows API) |
| `workspace.next` / `workspace.prev` | | GNOME, Windows |
| `overview.toggle` | | GNOME, Windows |
| `keys` | `keys`: `"Ctrl+W"` typed into the focused window | all |
| `exec` | `command` (argument list or, with `"shell": true`, a string) | all |
| `delay` | `ms` | all |

Taps are not exposed by GNOME's touchpad gesture events (only swipes, pinch and
hold), so there is no tap trigger; bind a mouse button or a shortcut instead.
A binding that cannot work on this platform is skipped with a warning;
`touchpad-snap config check` and the GUI list them.

## Recipes

Copy a binding into `bindings`. All of them work as `hotkey` triggers too.

```json
{ "name": "Next window of this app",
  "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "left" },
  "actions": [ { "type": "window.next" } ] }
```

```json
{ "name": "Next app",
  "trigger": { "type": "swipe", "fingers": 3, "modifiers": ["ctrl"], "direction": "up" },
  "actions": [ { "type": "app.next" } ] }
```

```json
{ "name": "Close tab in the browser",
  "trigger": { "type": "hotkey", "keys": "Super+Alt+W" },
  "when": { "app": ["*firefox*", "*chrom*"] },
  "actions": [ { "type": "keys", "keys": "Ctrl+W" } ] }
```

```json
{ "name": "Close window",
  "trigger": { "type": "pinch", "fingers": 3, "direction": "in", "modifiers": ["ctrl"] },
  "actions": [ { "type": "window.close" } ] }
```

```json
{ "name": "Volume up while the mouse wheel turns with Back held",
  "trigger": { "type": "mouse", "button": "back", "scroll": "up" },
  "actions": [ { "type": "keys", "keys": "XF86AudioRaiseVolume" } ] }
```

```json
{ "name": "Tile left / right",
  "trigger": { "type": "hotkey", "keys": "Super+Alt+Left" },
  "actions": [ { "type": "window.tile", "to": "left" } ] }
```

```json
{ "name": "Terminal",
  "trigger": { "type": "hotkey", "keys": "Super+Alt+Return" },
  "actions": [ { "type": "exec", "command": ["xterm"] } ] }
```

```json
{ "name": "Prefix: Super+Alt+W, then M = maximize",
  "trigger": { "type": "sequence", "keys": "Super+Alt+W", "then": "M" },
  "actions": [ { "type": "window.maximize" } ] }
```

## Troubleshooting

`touchpad-snap doctor` says what is missing, and `touchpad-snap status` asks a
running daemon what it actually registered.

- **A gesture does nothing**: the daemon must run (`touchpad-snap daemon`), the
  config must be listed by `touchpad-snap config check`, and the extension
  must be enabled and at the same protocol version (native GNOME, Windows) —
  `doctor` prints it.
- **A hotkey is taken**: GNOME Settings → Keyboard → Shortcuts, or another
  Windows app, owns it; `status` lists the refused ones.
- **Windows' own gestures still fire**: Settings → Bluetooth & devices →
  Touchpad → Advanced gestures, set three/four-finger gestures to *Nothing*.
- **A `when` binding is inactive**: `config check` prints the reason
  (`desktop`/`os`/app pattern).
- **GNOME after an update**: log out and back in once; GNOME discovers
  extensions at login.

## Development

```sh
cargo test                     # Rust unit tests (platform-neutral core)
npm test                       # unit tests of the extension's pure JS
cargo build --all-targets && ./test/e2e/gnome.sh
                               # end to end in a throw-away headless GNOME Shell:
                               # real windows, hotkeys typed on a virtual keyboard
pwsh test/e2e/windows.ps1 -Bin target/debug/touchpad-snap.exe   # on Windows
pwsh test/e2e/windows.ps1 -Bin target/debug/touchpad-snap.exe -VirtualDesktops
                               # also covers a window on a second virtual desktop
nix build .#checks.x86_64-linux.module-eval
                               # evaluates the NixOS module, its generated
                               # config and the user unit (no KVM)
nix build .#checks.x86_64-linux.vm-touchpad -L
                               # a full GNOME session in a VM (needs KVM),
                               # swiped on a virtual touchpad
nix flake check                # all Nix checks: the build (runs cargo test),
                               # module-eval and the VM test
```

The e2e test never touches your running desktop: it starts its own
gnome-shell with its own D-Bus, HOME and dconf. That shell reads no input
devices, so the touchpad path is tested in the VM instead: a uinput touchpad
plays three-finger swipes through libinput, mutter and GNOME Shell, and the
test checks that Ctrl + three fingers left, Alt + three fingers up/down and
Super + three fingers left jump without switching workspace or opening the
overview, while plain three fingers still do both. See
[gnome-extension/README.md](gnome-extension/README.md) for the D-Bus protocol.
CI runs all of the above on GNOME 46, 48, 49, 50 and 51 (Ubuntu, Debian, Fedora and Arch containers), in the NixOS VM, and on Windows Server 2022.

## License

MIT
