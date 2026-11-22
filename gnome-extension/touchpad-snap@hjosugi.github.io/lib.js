// SPDX-License-Identifier: MIT
//
// The parts of the extension that do not touch GNOME Shell, so they can be
// unit-tested with plain Node (see test/extension/lib.test.mjs).

// Modifier bits shared with the Rust side (src/hotkey.rs, `Mods`).
// Part of the D-Bus protocol: do not renumber.
export const Mods = Object.freeze({SHIFT: 1, CTRL: 2, ALT: 4, SUPER: 8});

// The modifier keys typed by SendKeys: [bit, X keysym of the left key].
export const MODIFIER_KEYS = Object.freeze([
    [Mods.SUPER, 0xffeb],
    [Mods.CTRL, 0xffe3],
    [Mods.ALT, 0xffe9],
    [Mods.SHIFT, 0xffe1],
]);

/**
 * The key events that type a shortcut: its modifiers down, the key tapped,
 * its modifiers up.
 *
 * @param {number} keyval - X keysym of the key
 * @param {number} mods - Mods bits of the shortcut
 * @returns {Array<[number, boolean]>} [keysym, pressed] in order
 */
export function keySequence(keyval, mods) {
    const keys = MODIFIER_KEYS.filter(([bit]) => mods & bit).map(([, sym]) => sym);
    return [
        ...keys.map(sym => [sym, true]),
        [keyval, true],
        [keyval, false],
        ...keys.reverse().map(sym => [sym, false]),
    ];
}

// How long a shortcut waits for the trigger's modifiers to go up.
export const KEYS_WAIT_MS = 3000;

/**
 * Shortcuts to type, in order, each once no modifier that does not belong to
 * it is down.
 *
 * A virtual keyboard cannot release a key somebody else holds (mutter
 * ignores such a release), so `Alt` + swipe sending `Ctrl+Tab` cannot be
 * typed while Alt is down: it would arrive as `Alt+Ctrl+Tab`. It is typed
 * as soon as Alt comes up instead. The queue keeps the order of a chain,
 * and drops what has waited longer than KEYS_WAIT_MS (a modifier held
 * for that long is not part of a gesture any more).
 */
export class KeyQueue {
    /**
     * @param {function(number, number)} type - types keysym with Mods bits
     * @param {function(object)} [onDrop] - told about each dropped shortcut
     */
    constructor(type, onDrop = () => {}) {
        this._type = type;
        this._onDrop = onDrop;
        this._items = [];
    }

    get pending() {
        return this._items.length > 0;
    }

    push(keysym, mods, nowMs) {
        this._items.push({keysym, mods, at: nowMs});
    }

    clear() {
        this._items = [];
    }

    /**
     * Types what can be typed now.
     *
     * @param {number} down - Mods bits physically down right now
     * @param {number} nowMs
     * @returns {boolean} whether something still waits
     */
    pump(down, nowMs) {
        while (this._items.length > 0) {
            const item = this._items[0];
            if (nowMs - item.at > KEYS_WAIT_MS) {
                this._items.shift();
                this._onDrop(item);
                continue;
            }
            if (down & ~item.mods)
                return true;
            this._items.shift();
            this._type(item.keysym, item.mods);
        }
        return false;
    }
}

// Swipe phases as sent in the Swipe signal (src/gesture.rs, `Phase`).
export const Phase = Object.freeze({BEGIN: 0, UPDATE: 1, END: 2, CANCEL: 3});

/**
 * Converts a Clutter modifier state into our bit set. Lock keys (Caps Lock,
 * Num Lock) and mouse buttons are ignored.
 *
 * @param {number} state - a Clutter.ModifierType bit mask
 * @param {object} M - the Clutter.ModifierType enum (passed in for testing)
 * @returns {number}
 */
export function modsFromState(state, M) {
    let mods = 0;
    if (state & M.SHIFT_MASK)
        mods |= Mods.SHIFT;
    if (state & M.CONTROL_MASK)
        mods |= Mods.CTRL;
    if (state & M.MOD1_MASK)
        mods |= Mods.ALT;
    if (state & (M.SUPER_MASK | M.MOD4_MASK))
        mods |= Mods.SUPER;
    return mods;
}

/**
 * Decides which touchpad swipes to take away from GNOME Shell.
 *
 * A swipe is claimed when, at its first event, the finger count and the held
 * modifiers exactly match one of the claims. From then on every event of that
 * swipe is stopped (so the shell does not switch workspace) and forwarded,
 * even if the modifier is released half-way.
 */
export class SwipeClaimer {
    constructor() {
        this._claims = [];
        this._active = null;
    }

    /** @param {Array<[number, number]>} claims - [fingers, mods] pairs */
    setClaims(claims) {
        this._claims = claims.map(([fingers, mods]) => ({fingers, mods}));
        this._active = null;
    }

    get hasClaims() {
        return this._claims.length > 0;
    }

    reset() {
        this._active = null;
    }

    /**
     * @returns {{stop: boolean, emit: Array|null}} whether to stop the event,
     *   and the Swipe signal arguments (phase, fingers, mods, dx, dy) if any
     */
    handle(phase, fingers, mods, dx, dy) {
        if (phase === Phase.BEGIN) {
            this._active = this._claims.find(c => c.fingers === fingers && c.mods === mods) ?? null;
            if (!this._active)
                return {stop: false, emit: null};
            return {stop: true, emit: [Phase.BEGIN, fingers, mods, dx, dy]};
        }

        const claim = this._active;
        if (!claim)
            return {stop: false, emit: null};

        if (phase === Phase.END || phase === Phase.CANCEL)
            this._active = null;

        return {stop: true, emit: [phase, claim.fingers, claim.mods, dx, dy]};
    }
}
