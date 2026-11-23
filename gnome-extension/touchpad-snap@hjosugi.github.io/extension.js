// SPDX-License-Identifier: MIT
//
// GNOME Shell side of touchpad-snap.
//
// Wayland clients cannot list or focus other clients' windows, grab global
// keys or see the touchpad swipes the shell uses for itself. This extension
// runs inside the compositor and offers exactly that on the session bus, for
// the `touchpad-snap` program to use:
//
//   bus name   io.github.hjosugi.TouchpadSnap
//   object     /io/github/hjosugi/TouchpadSnap
//   interface  io.github.hjosugi.TouchpadSnap1
//
// List / Activate serve one-shot jumps (`touchpad-snap next`). The daemon
// (`touchpad-snap daemon`) additionally hands over its hotkeys and swipe claims;
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
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {
    KeyQueue, NEUTRAL_KEYSYM, Phase, SwipeClaimer, keySequence, modsFromState,
    shouldNeutralize,
} from './lib.js';

const BUS_NAME = 'io.github.hjosugi.TouchpadSnap';
const OBJECT_PATH = '/io/github/hjosugi/TouchpadSnap';
const INTERFACE = 'io.github.hjosugi.TouchpadSnap1';
const PROTOCOL_VERSION = 5;

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
    <method name="ShowHud">
      <arg type="t" direction="in" name="id"/>
      <arg type="u" direction="in" name="index"/>
      <arg type="u" direction="in" name="total"/>
      <arg type="u" direction="in" name="duration_ms"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="SendKeys">
      <arg type="u" direction="in" name="keysym"/>
      <arg type="u" direction="in" name="mods"/>
      <arg type="b" direction="out" name="ok"/>
    </method>
    <method name="SetAccelerators">
      <arg type="a(ss)" direction="in" name="bindings"/>
      <arg type="as" direction="out" name="failed"/>
    </method>
    <method name="SetSwipeClaims">
      <arg type="a(uuas)" direction="in" name="claims"/>
    </method>
    <signal name="AcceleratorActivated">
      <arg type="s" name="action"/>
    </signal>
    <signal name="AcceleratorReleased">
      <arg type="s" name="action"/>
    </signal>
    <signal name="Swipe">
      <arg type="u" name="phase"/>
      <arg type="u" name="fingers"/>
      <arg type="u" name="mods"/>
      <arg type="s" name="app_id"/>
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

// The modifiers whose release ends a hotkey session; lock keys (Caps Lock,
// Num Lock) and AltGr are not held for a shortcut.
const HOLD_MASK = Clutter.ModifierType.SHIFT_MASK | Clutter.ModifierType.CONTROL_MASK |
    Clutter.ModifierType.MOD1_MASK | Clutter.ModifierType.MOD4_MASK |
    Clutter.ModifierType.SUPER_MASK | Clutter.ModifierType.HYPER_MASK |
    Clutter.ModifierType.META_MASK;

// How often to look at the modifiers while a hotkey's are held. Only while
// they are: the watch stops at the release.
const RELEASE_POLL_MS = 25;

const PHASES = new Map([
    [Clutter.TouchpadGesturePhase.BEGIN, Phase.BEGIN],
    [Clutter.TouchpadGesturePhase.UPDATE, Phase.UPDATE],
    [Clutter.TouchpadGesturePhase.END, Phase.END],
    [Clutter.TouchpadGesturePhase.CANCEL, Phase.CANCEL],
]);

// A short-lived overlay naming the window just jumped to: app icon, window
// title, and its place in the cycle ("Text Editor · 2 / 3"). Centred on the
// window's monitor, never reactive, replaced by the next one.
class Hud {
    constructor(tracker) {
        this._tracker = tracker;
        this._actor = null;
        this._timeoutId = 0;
    }

    show(window, index, total, durationMs) {
        this.destroy();

        const app = this._tracker.get_window_app(window);
        const box = new St.BoxLayout({
            style_class: 'touchpad-snap-hud',
            reactive: false,
            opacity: 0,
        });
        // St.BoxLayout grew `orientation` in GNOME 48 and deprecated `vertical`.
        if ('orientation' in box)
            box.orientation = Clutter.Orientation.VERTICAL;
        else
            box.vertical = true;

        const icon = app?.create_icon_texture(64);
        if (icon) {
            icon.x_align = Clutter.ActorAlign.CENTER;
            box.add_child(icon);
        }

        const title = new St.Label({
            style_class: 'touchpad-snap-hud-title',
            text: window.get_title() || app?.get_name() || '',
            x_align: Clutter.ActorAlign.CENTER,
        });
        title.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        box.add_child(title);

        const name = app?.get_name() ?? window.get_wm_class() ?? '';
        box.add_child(new St.Label({
            style_class: 'touchpad-snap-hud-detail',
            text: total > 0 ? `${name} · ${index} / ${total}` : name,
            x_align: Clutter.ActorAlign.CENTER,
        }));

        Main.uiGroup.add_child(box);
        this._actor = box;

        const monitors = Main.layoutManager.monitors;
        const monitor = monitors[window.get_monitor()] ?? Main.layoutManager.primaryMonitor;
        const [, width] = box.get_preferred_width(-1);
        const [, height] = box.get_preferred_height(width);
        box.set_position(
            Math.floor(monitor.x + (monitor.width - width) / 2),
            Math.floor(monitor.y + (monitor.height - height) / 2));

        box.ease({opacity: 255, duration: 80, mode: Clutter.AnimationMode.EASE_OUT_QUAD});
        this._timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, Math.max(durationMs, 100), () => {
            this._timeoutId = 0;
            box.ease({
                opacity: 0,
                duration: 150,
                mode: Clutter.AnimationMode.EASE_IN_QUAD,
                onComplete: () => {
                    if (this._actor === box)
                        this.destroy();
                },
            });
            return GLib.SOURCE_REMOVE;
        });
    }

    destroy() {
        if (this._timeoutId) {
            GLib.Source.remove(this._timeoutId);
            this._timeoutId = 0;
        }
        this._actor?.destroy();
        this._actor = null;
    }
}

class TouchpadSnapService {
    constructor() {
        this._tracker = Shell.WindowTracker.get_default();
        this._hud = new Hud(this._tracker);
        this._claimer = new SwipeClaimer();
        // Meta keybinding action id -> action name chosen by the daemon.
        this._accels = new Map();
        this._owner = null;
        this._ownerWatchId = 0;
        this._connection = null;
        this._keyboard = null;
        this._keyQueue = new KeyQueue((sym, mods) => this._typeKeys(sym, mods),
            item => console.warn(`touchpad-snap: dropped keys ${item.keysym.toString(16)}: modifiers stayed down`));
        this._keysTimerId = 0;
        this._busId = 0;
        this._dbus = null;
        this._acceleratorId = 0;
        this._capturedId = 0;
        // Watching for the release of the modifiers of the last hotkey.
        this._releaseId = 0;
        this._releaseMask = 0;
        this._releaseAction = '';
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
        this._hud.destroy();
        this._keyQueue.clear();
        if (this._keysTimerId) {
            GLib.Source.remove(this._keysTimerId);
            this._keysTimerId = 0;
        }
        this._keyboard = null;

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

    ShowHud(id, index, total, durationMs) {
        const window = global.display.list_all_windows().find(w => w.get_id() === id);
        if (!window)
            return false;
        this._hud.show(window, index, total, durationMs);
        return true;
    }

    // Types a shortcut into the focused window with a virtual keyboard, as
    // GNOME's on-screen keyboard does. Only the daemon that registered its
    // hotkeys may ask: anybody on the session bus could otherwise type.
    //
    // The shortcut waits until no modifier outside it is down: a virtual
    // keyboard cannot release what the user's hand holds (mutter ignores such
    // a release), so Alt + swipe sending Ctrl+Tab goes out when Alt comes up.
    SendKeysAsync([keysym, mods], invocation) {
        if (!this._owner || invocation.get_sender() !== this._owner) {
            invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied',
                'only the touchpad-snap daemon may send keys');
            return;
        }
        this._keyQueue.push(keysym, mods, GLib.get_monotonic_time() / 1000);
        this._pumpKeys();
        invocation.return_value(new GLib.Variant('(b)', [true]));
    }

    _heldMods() {
        const [, , state] = global.get_pointer();
        return modsFromState(state & HOLD_MASK, Clutter.ModifierType);
    }

    // Turns the release of a modifier held through a claimed swipe into an
    // ordinary one: without a key in between, Alt opens the focused window's
    // menu bar and Super the overview. F19 stays F19 under either modifier and
    // is unbound by default; see NEUTRAL_KEYSYM in lib.js. Nothing is sent if
    // the modifier is already up.
    _neutralizeLoneModifier(mods) {
        if (shouldNeutralize(mods, this._heldMods()))
            this._typeKeys(NEUTRAL_KEYSYM, 0);
    }

    _typeKeys(keysym, mods) {
        if (!this._keyboard) {
            // Clutter.get_default_backend() was removed in GNOME Shell 51.
            const backend = typeof Clutter.get_default_backend === 'function'
                ? Clutter.get_default_backend()
                : global.stage.context.get_backend();
            const seat = backend.get_default_seat();
            this._keyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        }
        try {
            for (const [sym, pressed] of keySequence(keysym, mods)) {
                this._keyboard.notify_keyval(GLib.get_monotonic_time(), sym,
                    pressed ? Clutter.KeyState.PRESSED : Clutter.KeyState.RELEASED);
            }
        } catch (e) {
            console.error(`touchpad-snap: typing keys failed: ${e}`);
        }
    }

    // Types what can be typed; keeps polling while something waits for a
    // modifier to go up.
    _pumpKeys() {
        const waiting = this._keyQueue.pump(this._heldMods(), GLib.get_monotonic_time() / 1000);
        if (waiting && !this._keysTimerId) {
            this._keysTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RELEASE_POLL_MS, () => {
                if (this._keyQueue.pending && this._pumpKeys())
                    return GLib.SOURCE_CONTINUE;
                this._keysTimerId = 0;
                return GLib.SOURCE_REMOVE;
            });
        }
        return waiting;
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
        this._keyQueue.clear();
        this._stopReleaseWatch();
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
            logError(e, `touchpad-snap: emitting ${signal}`);
        }
    }

    // -- input -----------------------------------------------------------------

    _onAccelerator(action) {
        const name = this._accels.get(action);
        if (name === undefined)
            return;
        this._emit('AcceleratorActivated', new GLib.Variant('(s)', [name]));
        this._watchRelease(name);
    }

    // Like the shell's own Alt+Tab switcher: after a hotkey, wait until none
    // of the modifiers held for it is down any more, then tell the daemon,
    // which treats the presses in between as one session. Polled, because
    // key releases go to the focused window, not to the shell.
    _watchRelease(name) {
        const [, , mods] = global.get_pointer();
        this._releaseMask = mods & HOLD_MASK;
        this._releaseAction = name;
        if (!this._releaseId) {
            this._releaseId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, RELEASE_POLL_MS,
                () => this._pollRelease());
        }
    }

    _pollRelease() {
        const [, , mods] = global.get_pointer();
        if (mods & this._releaseMask)
            return GLib.SOURCE_CONTINUE;
        this._releaseId = 0;
        this._emit('AcceleratorReleased', new GLib.Variant('(s)', [this._releaseAction]));
        return GLib.SOURCE_REMOVE;
    }

    _stopReleaseWatch() {
        if (this._releaseId) {
            GLib.Source.remove(this._releaseId);
            this._releaseId = 0;
        }
    }

    _onTouchpad(event) {
        if (!this._claimer.hasClaims || event.type() !== Clutter.EventType.TOUCHPAD_SWIPE)
            return Clutter.EVENT_PROPAGATE;

        const phase = PHASES.get(event.get_gesture_phase());
        if (phase === undefined)
            return Clutter.EVENT_PROPAGATE;

        let mods = 0;
        let app = [];
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
            app = this._appIdentifiers();
        }

        const fingers = event.get_touchpad_gesture_finger_count();
        const [dx, dy] = event.get_gesture_motion_delta_unaccelerated();
        const {stop, emit} = this._claimer.handle(phase, fingers, mods, app, dx, dy);
        if (emit)
            this._emit('Swipe', new GLib.Variant('(uuusdd)', emit));
        // A claimed swipe is not a key, so a held Alt or Super would look like
        // a lone press when it is finally released. Tap a harmless key while
        // it is still down, after the swipe events so the claim is settled.
        if (emit && (phase === Phase.END || phase === Phase.CANCEL))
            this._neutralizeLoneModifier(emit[2]);
        return stop ? Clutter.EVENT_STOP : Clutter.EVENT_PROPAGATE;
    }

    // The focused app's identifiers, as `_snapshot` reports them. The first
    // one is sent to the daemon; all of them are matched against the claim's
    // app filter.
    _appIdentifiers() {
        let window = global.display.focus_window;
        while (window && !this._isListed(window) && window.get_transient_for())
            window = window.get_transient_for();
        if (!window)
            return [];
        const app = this._tracker.get_window_app(window);
        const wmClass = window.get_wm_class() ?? '';
        return [
            app?.get_id() ?? '',
            app?.get_name() ?? '',
            wmClass,
        ].filter((value, index, all) => value && all.indexOf(value) === index);
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

export default class TouchpadSnapExtension extends Extension {
    enable() {
        this._service = new TouchpadSnapService();
        this._service.start();
    }

    disable() {
        this._service?.stop();
        this._service = null;
    }
}
