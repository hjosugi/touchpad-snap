# touchpad-snap GNOME Shell extension

`touchpad-snap@hjosugi.github.io` is the part of touchpad-snap that runs inside
GNOME Shell. It holds no policy: it lists and activates windows, grabs the
hotkeys the daemon asks for, and forwards the touchpad swipes the daemon
claims. Install it together with the `touchpad-snap` binary.

`lib.js` is the GNOME-free part (modifier mapping, the swipe claim state
machine), unit-tested with Node: `npm test` at the repository root.

## D-Bus protocol (version 3)

| | |
|---|---|
| bus name | `io.github.hjosugi.TouchpadSnap` |
| object | `/io/github/hjosugi/TouchpadSnap` |
| interface | `io.github.hjosugi.TouchpadSnap1` |

### Methods

- `List() → s json` — the window list, most recently used first:
  ```json
  {"version": 3, "active_workspace": 0, "windows": [
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
- `ShowHud(t id, u index, u total, u duration_ms) → b ok` — briefly show an
  overlay for a window (app icon, title, "index / total") centred on its
  monitor; a new one replaces the previous. *(Since version 2.)*
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
- `AcceleratorReleased(s action)` — after an `AcceleratorActivated`, once
  none of the modifiers held at that moment is down any more (polled every
  25 ms while they are). Presses of hotkeys in between are one session, as
  with Alt+Tab. *(Since version 3.)*
- `Swipe(u phase, u fingers, u mods, d dx, d dy)` — `phase`: 0 begin,
  1 update, 2 end, 3 cancel; `dx`/`dy`: unaccelerated motion.

`mods` bits: 1 Shift, 2 Ctrl, 4 Alt, 8 Super.

### Lifetime

One registered caller at a time; a new caller replaces the previous one.
When the caller's connection closes, its grabs and claims are released.

### Property

- `ProtocolVersion` (u) — 3. Version 1 had no `ShowHud`, version 2 no
  `AcceleratorReleased`; the binary works with any of them, skipping the
  overlay on version 1 and ending MRU sessions by time on versions 1 and 2.
