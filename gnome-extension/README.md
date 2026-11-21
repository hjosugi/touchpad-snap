# window-jump GNOME Shell extension

`window-jump@hjosugi.github.io` is the part of window-jump that runs inside
GNOME Shell. It holds no policy: it lists and activates windows, grabs the
hotkeys the daemon asks for, and forwards the touchpad swipes the daemon
claims. Install it together with the `window-jump` binary.

`lib.js` is the GNOME-free part (modifier mapping, the swipe claim state
machine), unit-tested with Node: `npm test` at the repository root.

## D-Bus protocol (version 1)

| | |
|---|---|
| bus name | `org.hjosugi.WindowJump` |
| object | `/org/hjosugi/WindowJump` |
| interface | `org.hjosugi.WindowJump1` |

### Methods

- `List() → s json` — the window list, most recently used first:
  ```json
  {"version": 1, "active_workspace": 0, "windows": [
    {"id": 123, "title": "…", "app_id": "org.gnome.TextEditor.desktop",
     "app_name": "Text Editor", "wm_class": "org.gnome.TextEditor", "pid": 4242,
     "workspace": 0, "minimized": false, "focused": true, "mru": 0,
     "x": 290, "y": 116, "width": 700, "height": 520}
  ]}
  ```
  Only normal windows (and dialogs without a parent) are listed. `workspace`
  is -1 for a window on every workspace. Focus on a dialog counts as focus on
  its parent.
- `Activate(t id) → b ok` — focus a window, switching workspace if needed.
- `SetAccelerators(a(ss) bindings) → as failed` — grab
  `(accelerator, action)` pairs, e.g. `("<Super><Alt>n", "hotkey:0")`, for
  the calling connection. Replaces that caller's previous grabs. Returns the
  accelerators that could not be grabbed (already used by GNOME or another
  client). Active in normal mode and in the overview.
- `SetSwipeClaims(a(uu) claims)` — `(fingers, mods)` pairs. A touchpad swipe
  whose finger count and held modifiers exactly match a claim at its first
  event is stopped before GNOME sees it and forwarded. Normal mode only.

### Signals (sent to the registered caller only)

- `AcceleratorActivated(s action)`
- `Swipe(u phase, u fingers, u mods, d dx, d dy)` — `phase`: 0 begin,
  1 update, 2 end, 3 cancel; `dx`/`dy`: unaccelerated motion.

`mods` bits: 1 Shift, 2 Ctrl, 4 Alt, 8 Super.

### Lifetime

One registered caller at a time; a new caller replaces the previous one.
When the caller's connection closes, its grabs and claims are released.

### Property

- `ProtocolVersion` (u) — 1.
