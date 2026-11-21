// SPDX-License-Identifier: MIT
//
// GNOME Shell side of window-jump.
//
// Wayland clients cannot list or focus other clients' windows, grab global
// keys or see the touchpad swipes the shell uses for itself. This extension
// runs inside the compositor and offers exactly that on the session bus, for
// the `window-jump` program to use:
//
//   bus name   org.hjosugi.WindowJump
//   object     /org/hjosugi/WindowJump
//   interface  org.hjosugi.WindowJump1
//
// List / Activate serve one-shot jumps (`window-jump next`). The daemon
// (`window-jump daemon`) additionally hands over its hotkeys and swipe claims;
// the extension grabs them and reports activity back through signals sent to
// that daemon only. Everything the daemon registered is dropped when it
// disconnects, so a crashed daemon never leaves keys or swipes stolen.
//
// All policy (which window is next, what a swipe means) lives in the Rust
// program; this file only bridges.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {Phase, SwipeClaimer, modsFromState} from './lib.js';

const BUS_NAME = 'org.hjosugi.WindowJump';
const OBJECT_PATH = '/org/hjosugi/WindowJump';
const INTERFACE = 'org.hjosugi.WindowJump1';
const PROTOCOL_VERSION = 1;

const IFACE_XML = `
<node>
  <interface name="${INTERFACE}">
    <method name="List">
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="Activate">
      <arg type="t" direction="in" name="id"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="SetAccelerators">
      <arg type="a(ss)" direction="in" name="bindings"/>
      <arg type="as" direction="out" name="failed"/>
    </method>
    <method name="SetSwipeClaims">
      <arg type="a(uu)" direction="in" name="claims"/>
    </method>
    <signal name="AcceleratorActivated">
      <arg type="s" name="action"/>
    </signal>
    <signal name="Swipe">
      <arg type="u" name="phase"/>
      <arg type="u" name="fingers"/>
      <arg type="u" name="mods"/>
      <arg type="d" name="dx"/>
      <arg type="d" name="dy"/>
    </signal>
    <property name="ProtocolVersion" type="u" access="read"/>
  </interface>
</node>`;

// Hotkeys also work in the overview (where a GNOME session starts): jumping
// activates the window, which closes it. Swipes only outside it, where three
// fingers are not already busy moving the overview around.
const HOTKEY_MODES = Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW;

const PHASES = new Map([
    [Clutter.TouchpadGesturePhase.BEGIN, Phase.BEGIN],
    [Clutter.TouchpadGesturePhase.UPDATE, Phase.UPDATE],
    [Clutter.TouchpadGesturePhase.END, Phase.END],
    [Clutter.TouchpadGesturePhase.CANCEL, Phase.CANCEL],
]);

class WindowJumpService {
    constructor() {
        this._tracker = Shell.WindowTracker.get_default();
        this._claimer = new SwipeClaimer();
        // Meta keybinding action id -> action name chosen by the daemon.
        this._accels = new Map();
        this._owner = null;
        this._ownerWatchId = 0;
        this._connection = null;
        this._busId = 0;
        this._dbus = null;
        this._acceleratorId = 0;
        this._capturedId = 0;
    }

    start() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE_XML, this);
        this._busId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.REPLACE | Gio.BusNameOwnerFlags.ALLOW_REPLACEMENT,
            connection => {
                this._connection = connection;
                this._dbus.export(connection, OBJECT_PATH);
            },
            null,
            null);

        this._acceleratorId = global.display.connect('accelerator-activated',
            (_display, action) => this._onAccelerator(action));

        // The capture phase on the stage runs before the shell's own swipe
        // handling (which listens in the bubble phase), so a claimed swipe can
        // be stopped before it switches workspace.
        const onTouchpad = (_actor, event) => this._onTouchpad(event);
        try {
            this._capturedId = global.stage.connect('captured-event::touchpad', onTouchpad);
        } catch {
            // Older Clutter without per-event-type signal details.
            this._capturedId = global.stage.connect('captured-event', onTouchpad);
        }
    }

    stop() {
        this._release();

        if (this._capturedId) {
            global.stage.disconnect(this._capturedId);
            this._capturedId = 0;
        }
        if (this._acceleratorId) {
            global.display.disconnect(this._acceleratorId);
            this._acceleratorId = 0;
        }
        if (this._dbus) {
            this._dbus.unexport();
            this._dbus = null;
        }
        if (this._busId) {
            Gio.bus_unown_name(this._busId);
            this._busId = 0;
        }
        this._connection = null;
    }

    // -- D-Bus API -------------------------------------------------------------

    get ProtocolVersion() {
        return PROTOCOL_VERSION;
    }

    List() {
        return this._snapshot();
    }

    Activate(id) {
        const window = global.display.list_all_windows().find(w => w.get_id() === id);
        if (!window)
            return false;

        const time = global.display.get_current_time_roundtrip();
        // Main.activateWindow() dereferences the window's workspace, which is
        // null for windows shown on every workspace.
        if (window.is_on_all_workspaces()) {
            Main.activateWindow(window, time,
                global.workspace_manager.get_active_workspace_index());
        } else {
            Main.activateWindow(window, time);
        }
        return true;
    }

    SetAcceleratorsAsync([bindings], invocation) {
        this._adopt(invocation.get_sender());
        this._ungrabAll();

        const failed = [];
        for (const [accelerator, name] of bindings) {
            const action = global.display.grab_accelerator(accelerator, Meta.KeyBindingFlags.NONE);
            if (action === Meta.KeyBindingAction.NONE) {
                failed.push(accelerator);
                continue;
            }
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action), HOTKEY_MODES);
            this._accels.set(action, name);
        }
        invocation.return_value(new GLib.Variant('(as)', [failed]));
    }

    SetSwipeClaimsAsync([claims], invocation) {
        this._adopt(invocation.get_sender());
        this._claimer.setClaims(claims);
        invocation.return_value(null);
    }

    // -- daemon bookkeeping ----------------------------------------------------

    // One daemon at a time: a new caller replaces the previous one.
    _adopt(sender) {
        if (sender === this._owner)
            return;
        this._release();
        this._owner = sender;
        this._ownerWatchId = Gio.bus_watch_name(Gio.BusType.SESSION, sender,
            Gio.BusNameWatcherFlags.NONE, null, () => this._release());
    }

    _release() {
        this._ungrabAll();
        this._claimer.setClaims([]);
        if (this._ownerWatchId) {
            Gio.bus_unwatch_name(this._ownerWatchId);
            this._ownerWatchId = 0;
        }
        this._owner = null;
    }

    _ungrabAll() {
        for (const action of this._accels.keys()) {
            global.display.ungrab_accelerator(action);
            Main.wm.allowKeybinding(Meta.external_binding_name_for_action(action),
                Shell.ActionMode.NONE);
        }
        this._accels.clear();
    }

    _emit(signal, params) {
        if (!this._connection || !this._owner)
            return;
        try {
            this._connection.emit_signal(this._owner, OBJECT_PATH, INTERFACE, signal, params);
        } catch (e) {
            logError(e, `window-jump: emitting ${signal}`);
        }
    }

    // -- input -----------------------------------------------------------------

    _onAccelerator(action) {
        const name = this._accels.get(action);
        if (name !== undefined)
            this._emit('AcceleratorActivated', new GLib.Variant('(s)', [name]));
    }

    _onTouchpad(event) {
        if (!this._claimer.hasClaims || event.type() !== Clutter.EventType.TOUCHPAD_SWIPE)
            return Clutter.EVENT_PROPAGATE;

        const phase = PHASES.get(event.get_gesture_phase());
        if (phase === undefined)
            return Clutter.EVENT_PROPAGATE;

        let mods = 0;
        if (phase === Phase.BEGIN) {
            // Leave the overview, lock screen and modal dialogs alone.
            if (Main.actionMode !== Shell.ActionMode.NORMAL) {
                this._claimer.reset();
                return Clutter.EVENT_PROPAGATE;
            }
            // The event carries the modifier state; the seat's current state
            // is OR-ed in as a belt-and-braces measure.
            const [, , pointerMods] = global.get_pointer();
            mods = modsFromState(event.get_state() | pointerMods, Clutter.ModifierType);
        }

        const fingers = event.get_touchpad_gesture_finger_count();
        const [dx, dy] = event.get_gesture_motion_delta_unaccelerated();
        const {stop, emit} = this._claimer.handle(phase, fingers, mods, dx, dy);
        if (emit)
            this._emit('Swipe', new GLib.Variant('(uuudd)', emit));
        return stop ? Clutter.EVENT_STOP : Clutter.EVENT_PROPAGATE;
    }

    // -- window data -----------------------------------------------------------

    _isListed(window) {
        if (window.is_skip_taskbar() || window.is_override_redirect())
            return false;

        switch (window.get_window_type()) {
        case Meta.WindowType.NORMAL:
            return true;
        case Meta.WindowType.DIALOG:
            // A dialog that belongs to another window is reached through its parent.
            return !window.get_transient_for();
        default:
            return false;
        }
    }

    _snapshot() {
        const display = global.display;

        // Most recently used first. Windows missing from the tab list (it can
        // lag behind a just-created window) are appended in stacking order.
        const ordered = display.get_tab_list(Meta.TabList.NORMAL_ALL, null);
        const rest = display.list_all_windows().filter(w => !ordered.includes(w));

        // Focus on a dialog counts as focus on the window it belongs to.
        let focused = display.focus_window;
        while (focused && !this._isListed(focused) && focused.get_transient_for())
            focused = focused.get_transient_for();

        const windows = [];
        for (const window of [...ordered, ...rest]) {
            if (!this._isListed(window))
                continue;

            const app = this._tracker.get_window_app(window);
            const wmClass = window.get_wm_class() ?? '';
            const rect = window.get_frame_rect();
            windows.push({
                id: window.get_id(),
                title: window.get_title() ?? '',
                app_id: app?.get_id() ?? '',
                app_name: app?.get_name() ?? wmClass,
                wm_class: wmClass,
                pid: Math.max(window.get_pid(), 0),
                workspace: window.is_on_all_workspaces()
                    ? -1
                    : (window.get_workspace()?.index() ?? -1),
                minimized: window.minimized,
                focused: window === focused,
                mru: windows.length,
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            });
        }

        return JSON.stringify({
            version: PROTOCOL_VERSION,
            active_workspace: global.workspace_manager.get_active_workspace_index(),
            windows,
        });
    }
}

export default class WindowJumpExtension extends Extension {
    enable() {
        this._service = new WindowJumpService();
        this._service.start();
    }

    disable() {
        this._service?.stop();
        this._service = null;
    }
}
