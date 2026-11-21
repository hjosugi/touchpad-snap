# touchpad-snap

Switch between the windows **of the app you are in**, with a hotkey or a
modifier + touchpad swipe — like `Cmd+`` ` on macOS or the "cycle windows of
the current app" action in BetterTouchTool, for **GNOME (Wayland and X11)**,
**KDE Plasma**, **Sway, Hyprland and niri**, and **Windows**. Written in Rust;
a small GNOME Shell extension or KWin script bridges to the compositor where
Wayland allows nothing else.

[日本語の README はこちら](README.ja.md)

- **Hotkeys**: `Super+Alt+N` / `Super+Alt+P` by default (configurable).
- **Swipes on GNOME**: hold `Ctrl` and swipe left or right with three
  fingers, or hold `Alt` and swipe up or down (configurable). Only the
  swipes you claim (fingers + modifiers) are taken from GNOME; a plain
  three-finger swipe still switches workspace or opens the overview. Keep the fingers down and keep
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
    order = "mru";
    hotkey = [ { keys = "Super+Alt+N"; action = "next"; } ];
  };
};
```

This:

- installs the `touchpad-snap` package and runs `touchpad-snap daemon` as a
  systemd **user** service, bound to `graphical-session.target` with
  `Restart=on-failure`;
- renders `settings` (the normal TOML schema) to
  `/etc/touchpad-snap/config.toml` and starts the daemon with
  `--config /etc/touchpad-snap/config.toml`;
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
      order = "mru";
      hotkey = [ { keys = "Super+Alt+N"; action = "next"; } ];
    };
  };
}
```

The option names match the NixOS module. Home Manager renders the config to
`$XDG_CONFIG_HOME/touchpad-snap/config.toml` (usually
`~/.config/touchpad-snap/config.toml`), which is the path the daemon reads by
default, so the user service runs plain `touchpad-snap daemon`.
`gnome.enable` uses Home Manager's `programs.gnome-shell.extensions`, which
also writes `org/gnome/shell/enabled-extensions`. That list is managed as a
whole: add any other extensions you use to `programs.gnome-shell.extensions`
too, rather than expecting the module to append to the existing list.

Historically the program was called **window-jump**, and the binary copies
`~/.config/window-jump/config.toml` to
`~/.config/touchpad-snap/config.toml` on first run. Both modules write the
new file themselves (and NixOS passes an explicit `--config`), so that
automatic copy does not happen; move any old settings into `settings`.

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
   (set them to *Nothing*), then use the same `[[swipe]]` bindings as on GNOME:
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

Windows on other virtual desktops are included when `workspace = "all"` (the
default) and skipped when it is `"current"`. The current desktop is `ws 0`;
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
  entry. On Windows, quit window-jump from its tray icon first.
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
| `touchpad-snap daemon [-v]` | Serve the configured hotkeys and swipes. |
| `touchpad-snap list [--json]` | Show windows grouped by app. |
| `touchpad-snap activate <id>` | Focus one window by id. |
| `touchpad-snap doctor` | Check config, backend and window list. |
| `touchpad-snap config init\|path\|check` | Write / locate / validate the config file. |
| `touchpad-snap autostart enable\|disable` | Start the daemon at login. |

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
GlobalShortcuts. The daemon serves the configured `[[swipe]]` bindings from the
touchpad. On GNOME keep using the extension: it can claim swipes, so the shell
does not also act on them.

libinput cannot stop the compositor from seeing the same gesture. touchpad-snap
does not intercept it, so the compositor may switch workspace, move a window
or do whatever else it has bound to those fingers at the same time. Use a
modifier + finger count the compositor does not use for its own gestures (the
default `Ctrl` + three fingers is usually safe), or turn the compositor
gesture off. This is the same limitation as Windows Precision Touchpad input.

## Configuration

`touchpad-snap config init` writes a commented file to
`~/.config/touchpad-snap/config.toml` (`%APPDATA%\touchpad-snap\config.toml` on
Windows). Everything is optional; these are the defaults:

```toml
order = "stable"          # "stable" | "position" | "mru"
workspace = "all"         # "all" | "current"
include_minimized = true
mru_session_ms = 1000     # order = "mru", for `touchpad-snap next` from other tools: repeats closer than this are one session

[gesture]
threshold = 120.0         # finger travel before a swipe counts (GNOME's full workspace swipe is 400)
repeat_step = 220.0       # further travel per extra jump while the fingers stay down; 0 = one jump per swipe

[hud]                     # the overlay after a jump
enabled = true
duration_ms = 700

[[hotkey]]
keys = "Super+Alt+N"      # modifiers: Super (Win), Ctrl, Alt, Shift
action = "next"

[[hotkey]]
keys = "Super+Alt+P"
action = "prev"

[[swipe]]                 # GNOME; Windows and libinput builds elsewhere
fingers = 3               # 3–5
modifiers = ["ctrl"]
direction = "left"        # left | right | up | down
action = "next"

[[swipe]]
fingers = 3
modifiers = ["ctrl"]
direction = "right"
action = "prev"

[[swipe]]
fingers = 3
modifiers = ["alt"]       # any of ctrl, alt, shift, super
direction = "up"
action = "next"

[[swipe]]
fingers = 3
modifiers = ["alt"]
direction = "down"
action = "prev"
```

Each `[[swipe]]` is one combination of fingers, modifiers and direction, so
any of them can be changed, removed or added; once the file has a `[[swipe]]`
(or `[[hotkey]]`), only the ones in the file apply.
`hotkey = []` / `swipe = []` turn them off. The daemon picks up changes to the
file within about two seconds; an invalid edit is reported in its log and the
previous config stays in effect.

Tips:

- Avoid `Super` alone as the swipe modifier: releasing it opens the GNOME
  Activities overview (and may open the Start menu on Windows). `Ctrl`,
  `Ctrl+Alt` or `Shift` are safe.
- On Windows, turn off the built-in three/four-finger gestures first.
  Precision Touchpads report their contacts as raw HID input, and Windows'
  shell sees the same gesture independently — it cannot be intercepted.
- If a hotkey cannot be grabbed (GNOME or another app already uses it), the
  daemon says so at startup; `touchpad-snap daemon -v` logs every event.

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
test checks that Ctrl + three fingers left and Alt + three fingers up/down
jump without switching workspace or opening the overview, while plain three
fingers still do both. See
[gnome-extension/README.md](gnome-extension/README.md) for the D-Bus protocol.
CI runs all of the above on GNOME 46, 48, 49, 50 and 51 (Ubuntu, Debian, Fedora and Arch containers), in the NixOS VM, and on Windows Server 2022.

## License

MIT
