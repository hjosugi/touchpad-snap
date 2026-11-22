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
 * Case-insensitive glob match where `*` matches any run of characters. Kept
 * identical to `src/when.rs`'s matcher.
 *
 * @param {string} pattern
 * @param {string} value
 * @returns {boolean}
 */
export function patternMatches(pattern, value) {
    const p = pattern.toLowerCase();
    const v = value.toLowerCase();
    let pi = 0;
    let vi = 0;
    let star = -1;
    let starValue = 0;
    while (vi < v.length) {
        if (pi < p.length && p[pi] === '*') {
            star = pi;
            pi += 1;
            starValue = vi;
        } else if (pi < p.length && p[pi] === v[vi]) {
            pi += 1;
            vi += 1;
        } else if (star !== -1) {
            pi = star + 1;
            starValue += 1;
            vi = starValue;
        } else {
            return false;
        }
    }
    while (pi < p.length && p[pi] === '*')
        pi += 1;
    return pi === p.length;
}

/**
 * Whether an app matches a claim's app filter. An empty filter means every
 * app; otherwise any pattern may match the app id, name or WM class.
 *
 * @param {string[]} patterns
 * @param {string[]} app
 * @returns {boolean}
 */
export function appMatches(patterns, app) {
    if (!patterns || patterns.length === 0)
        return true;
    const candidates = Array.isArray(app) ? app : [app ?? ''];
    return candidates.some(value =>
        value && patterns.some(pattern => patternMatches(pattern, value)));
}

/**
 * Decides which touchpad swipes to take away from GNOME Shell.
 *
 * A swipe is claimed when, at its first event, the finger count and the held
 * modifiers exactly match one of the claims and the focused app matches the
 * claim's app filter. From then on every event of that swipe is stopped (so
 * the shell does not switch workspace) and forwarded, even if the modifier is
 * released half-way. The app is the one focused at the start, for the whole
 * gesture.
 */
export class SwipeClaimer {
    constructor() {
        this._claims = [];
        this._active = null;
    }

    /** @param {Array<[number, number, string[]]>} claims - [fingers, mods, apps] */
    setClaims(claims) {
        this._claims = claims.map(([fingers, mods, apps]) => ({
            fingers,
            mods,
            apps: Array.isArray(apps) ? apps : [],
        }));
        this._active = null;
    }

    get hasClaims() {
        return this._claims.length > 0;
    }

    reset() {
        this._active = null;
    }

    /**
     * @param {string[]|string} app - the focused app's identifiers (id, name,
     *   WM class); only read at BEGIN
     * @returns {{stop: boolean, emit: Array|null}} whether to stop the event,
     *   and the Swipe signal arguments
     *   (phase, fingers, mods, app_id, dx, dy) if any
     */
    handle(phase, fingers, mods, app, dx, dy) {
        if (phase === Phase.BEGIN) {
            const claim = this._claims.find(c =>
                c.fingers === fingers && c.mods === mods && appMatches(c.apps, app));
            if (!claim) {
                this._active = null;
                return {stop: false, emit: null};
            }
            const appId = Array.isArray(app) ? (app[0] ?? '') : (app ?? '');
            this._active = {claim, appId};
            return {stop: true, emit: [Phase.BEGIN, fingers, mods, appId, dx, dy]};
        }

        const active = this._active;
        if (!active)
            return {stop: false, emit: null};

        if (phase === Phase.END || phase === Phase.CANCEL)
            this._active = null;

        return {
            stop: true,
            emit: [phase, active.claim.fingers, active.claim.mods, active.appId, dx, dy],
        };
    }
}
