// SPDX-License-Identifier: MIT
//
// The parts of the extension that do not touch GNOME Shell, so they can be
// unit-tested with plain Node (see test/extension/lib.test.mjs).

// Modifier bits shared with the Rust side (src/hotkey.rs, `Mods`).
// Part of the D-Bus protocol: do not renumber.
export const Mods = Object.freeze({SHIFT: 1, CTRL: 2, ALT: 4, SUPER: 8});

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
