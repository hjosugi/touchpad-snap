# window-jump

Switch between the windows **of the app you are in**, with a hotkey or a
modifier + touchpad swipe — like `Cmd+`` ` on macOS or the "cycle windows of
the current app" action in BetterTouchTool, for **GNOME (Wayland and X11)** and
**Windows**. Written in Rust; a small GNOME Shell extension bridges to the
compositor where Wayland allows nothing else.

[日本語の README はこちら](README.ja.md)

- **Hotkeys**: `Super+Alt+N` / `Super+Alt+P` by default (configurable).
- **Swipes on GNOME**: hold `Ctrl` and swipe with three fingers. Only the
  swipes you claim (fingers + modifiers) are taken from GNOME; a plain
  three-finger swipe still switches workspace. Keep the fingers down and keep
  moving to step through several windows; move back to step back.
- **Swipes on Windows**: map a touchpad gesture to the hotkey in Windows
  Settings (see below).
- **Overlay**: after each jump the window title and "App · 2 / 5" (plus the
  app icon on GNOME) show briefly in the middle of the screen, so you can
  follow where you are while scrubbing. On Windows this needs the daemon.
- **Orders**: `stable` (fixed order, walks every window), `position` (left to
  right on screen) or `mru` (like Alt+Tab: one press toggles between the two
  most recent windows; pressing again while holding the modifiers digs
  deeper, and releasing settles there).
- **Any binder**: `window-jump next` / `window-jump prev` are plain commands,
  so they also work from GNOME's own custom shortcuts, AutoHotkey, keyd, a
  mouse driver…

## How it works

```
GNOME                                           Windows
─────                                           ───────
window-jump next / daemon                       window-jump next / daemon
        │  D-Bus (session bus)                          │  Win32
        ▼                                               ▼
window-jump@hjosugi.github.io                   EnumWindows, SetForegroundWindow,
(GNOME Shell extension, inside the compositor)  RegisterHotKey
 · List / Activate windows
 · grabs the daemon's hotkeys
 · claims modifier + touchpad swipes
```

On Wayland a normal program cannot list or focus other programs' windows,
grab global keys or see the touchpad gestures GNOME uses for itself. The
extension runs inside GNOME Shell and offers exactly that on the session bus;
all the logic (which window is next, what a swipe means, the config) is in the
Rust program. Everything the daemon registered is dropped the moment it exits.

## Install

### GNOME (any distribution)

1. Download from the [latest release](https://github.com/hjosugi/window-jump/releases/latest):
   - `window-jump-<version>-x86_64-unknown-linux-musl.tar.gz` (or `aarch64`) —
     a static binary, no dependencies. Put `window-jump` on your `PATH`, e.g.
     `~/.local/bin`.
   - `window-jump@hjosugi.github.io.shell-extension.zip`
2. Install the extension, then **log out and back in** (GNOME on Wayland only
   discovers new extensions at login):
   ```sh
   gnome-extensions install --force window-jump@hjosugi.github.io.shell-extension.zip
   # after logging back in:
   gnome-extensions enable window-jump@hjosugi.github.io
   ```
3. Check, start the daemon, and have it start at every login:
   ```sh
   window-jump doctor
   window-jump autostart enable     # ~/.config/autostart/window-jump.desktop
   window-jump daemon &             # for this session
   ```

Supported GNOME Shell versions: 46–51. CI runs the end-to-end test on 46 (Ubuntu 24.04), 48 (Debian 13), 49 (Fedora 43), 50 (Fedora 44) and the latest on Arch Linux (51 at the time of writing); 47 is not tested.

### NixOS / Nix flake

```nix
# flake.nix
inputs.window-jump.url = "github:hjosugi/window-jump";

# configuration.nix (with `window-jump` passed in through specialArgs)
environment.systemPackages = [
  window-jump.packages.${pkgs.stdenv.hostPlatform.system}.window-jump
  window-jump.packages.${pkgs.stdenv.hostPlatform.system}.gnome-shell-extension
];
```

Then enable the extension (`gnome-extensions enable window-jump@hjosugi.github.io`
or through dconf) and run `window-jump autostart enable`. Without NixOS:
`nix run github:hjosugi/window-jump -- doctor`.

### Windows 10 / 11

1. Download `window-jump-<version>-x86_64-pc-windows-msvc.zip` (or
   `aarch64`) and unpack it somewhere permanent, e.g.
   `%LOCALAPPDATA%\Programs\window-jump`.
2. Start at login, and now:
   ```powershell
   .\window-jump.exe autostart enable   # runs window-jumpw.exe (no console window) at login
   .\window-jumpw.exe                    # start it now
   ```
3. Touchpad: **Settings → Bluetooth & devices → Touchpad → Advanced gestures**,
   set e.g. *Three-finger swipe left* to **Custom shortcut** and press
   `Win+Alt+N`; *right* to `Win+Alt+P`.

Windows are grouped by executable (`notepad.exe`, `Code.exe`…); UWP apps are
looked up behind their ApplicationFrameHost frame.

Windows on other virtual desktops are included when `workspace = "all"` (the
default) and skipped when it is `"current"`. The current desktop is `ws 0`;
other desktops get positive numbers. Jumping to such a window is best effort:
Windows has no documented API to switch desktops, so window-jump sends
`Win+Ctrl+Left/Right` (at most eight steps each way) and reports an error if
the window cannot be reached. Suspended UWP windows stay hidden.

Manual test: create a second desktop with `Win+Ctrl+D`, open a second Notepad
there (or move one with Task View), return with `Win+Ctrl+Left`, then run
`window-jump list` (the second Notepad shows a positive `ws`) and
`window-jump next` from the first Notepad.

### From source

```sh
cargo install --git https://github.com/hjosugi/window-jump window-jump
```

The GNOME extension is the `gnome-extension/window-jump@hjosugi.github.io`
directory: copy it to `~/.local/share/gnome-shell/extensions/`.

## Usage

| Command | |
|---|---|
| `window-jump next` / `prev` | Focus the next / previous window of the focused app. `--order`, `--workspace` override the config. |
| `window-jump daemon [-v]` | Serve the configured hotkeys and swipes. |
| `window-jump list [--json]` | Show windows grouped by app. |
| `window-jump activate <id>` | Focus one window by id. |
| `window-jump doctor` | Check config, extension and window list. |
| `window-jump config init\|path\|check` | Write / locate / validate the config file. |
| `window-jump autostart enable\|disable` | Start the daemon at login. |

### Without the daemon

Bind the commands to any shortcut tool instead. GNOME: **Settings → Keyboard →
View and Customise Shortcuts → Custom Shortcuts**, command `window-jump next`.
(The extension is still needed on GNOME, for listing and focusing windows.)

## Configuration

`window-jump config init` writes a commented file to
`~/.config/window-jump/config.toml` (`%APPDATA%\window-jump\config.toml` on
Windows). Everything is optional; these are the defaults:

```toml
order = "stable"          # "stable" | "position" | "mru"
workspace = "all"         # "all" | "current"
include_minimized = true
mru_session_ms = 1000     # order = "mru", for `window-jump next` from other tools: repeats closer than this are one session

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

[[swipe]]                 # GNOME only
fingers = 3               # 3–5
modifiers = ["ctrl"]
direction = "left"        # left | right | up | down
action = "next"

[[swipe]]
fingers = 3
modifiers = ["ctrl"]
direction = "right"
action = "prev"
```

`hotkey = []` / `swipe = []` turn them off. The daemon picks up changes to the
file within about two seconds; an invalid edit is reported in its log and the
previous config stays in effect.

Tips:

- Avoid `Super` alone as the swipe modifier: releasing it opens the GNOME
  Activities overview. `Ctrl`, `Ctrl+Alt` or `Shift` are safe.
- If a hotkey cannot be grabbed (GNOME or another app already uses it), the
  daemon says so at startup; `window-jump daemon -v` logs every event.

## Development

```sh
cargo test                     # Rust unit tests (platform-neutral core)
npm test                       # unit tests of the extension's pure JS
cargo build --all-targets && ./test/e2e/gnome.sh
                               # end to end in a throw-away headless GNOME Shell:
                               # real windows, hotkeys typed on a virtual keyboard
pwsh test/e2e/windows.ps1 -Bin target/debug/window-jump.exe   # on Windows
pwsh test/e2e/windows.ps1 -Bin target/debug/window-jump.exe -VirtualDesktops
                               # also covers a window on a second virtual desktop
nix flake check                # Nix build (runs cargo test)
```

The e2e test never touches your running desktop: it starts its own
gnome-shell with its own D-Bus, HOME and dconf. See
[gnome-extension/README.md](gnome-extension/README.md) for the D-Bus protocol.
CI runs all of the above on GNOME 46, 48, 49, 50 and 51 (Ubuntu, Debian, Fedora and Arch containers) and on Windows Server 2022.

## License

MIT
