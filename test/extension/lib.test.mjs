// Unit tests for the GNOME-free part of the extension: node --test test/
import assert from 'node:assert/strict';
import {test} from 'node:test';

import {
    KEYS_WAIT_MS, KeyQueue, MODIFIER_KEYS, Mods, NEUTRAL_KEYSYM, Phase, SwipeClaimer,
    appMatches, keySequence, modsFromState, patternMatches, shouldNeutralize,
} from '../../gnome-extension/touchpad-snap@hjosugi.github.io/lib.js';

// Clutter.ModifierType values.
const M = {
    SHIFT_MASK: 1 << 0,
    LOCK_MASK: 1 << 1,
    CONTROL_MASK: 1 << 2,
    MOD1_MASK: 1 << 3,
    MOD2_MASK: 1 << 4,
    MOD4_MASK: 1 << 6,
    BUTTON1_MASK: 1 << 8,
    SUPER_MASK: 1 << 26,
};

test('modsFromState maps modifiers and ignores lock keys and buttons', () => {
    assert.equal(modsFromState(0, M), 0);
    assert.equal(modsFromState(M.CONTROL_MASK | M.LOCK_MASK | M.MOD2_MASK | M.BUTTON1_MASK, M), Mods.CTRL);
    assert.equal(modsFromState(M.SHIFT_MASK | M.MOD1_MASK, M), Mods.SHIFT | Mods.ALT);
    assert.equal(modsFromState(M.SUPER_MASK, M), Mods.SUPER);
    assert.equal(modsFromState(M.MOD4_MASK, M), Mods.SUPER);
});

test('modifier bits match the Rust side', () => {
    // src/hotkey.rs: SHIFT=1, CTRL=2, ALT=4, SUPER=8
    assert.deepEqual({...Mods}, {SHIFT: 1, CTRL: 2, ALT: 4, SUPER: 8});
    // src/gesture.rs: Begin=0, Update=1, End=2, Cancel=3
    assert.deepEqual({...Phase}, {BEGIN: 0, UPDATE: 1, END: 2, CANCEL: 3});
});

test('app patterns are case-insensitive globs like the daemon', () => {
    assert.ok(patternMatches('firefox', 'Firefox'));
    assert.ok(patternMatches('FIRE*', 'firefox.desktop'));
    assert.ok(patternMatches('*.desktop', 'org.gnome.TextEditor.desktop'));
    assert.ok(patternMatches('*', 'anything'));
    assert.ok(patternMatches('', ''));
    assert.ok(!patternMatches('', 'x'));
    assert.ok(!patternMatches('firefox', 'firefoxes'));
    assert.ok(patternMatches('a*b*c', 'aXXbYYc'));
    assert.ok(!patternMatches('a*b*c', 'aXXbYY'));

    // An empty filter is everywhere; otherwise id, name or WM class may match.
    assert.ok(appMatches([], ['firefox.desktop', 'Firefox', 'firefox']));
    assert.ok(appMatches(['firefox'], ['firefox.desktop', 'Firefox', 'firefox']));
    assert.ok(appMatches(['Text Editor'], ['org.gnome.TextEditor.desktop', 'Text Editor', 'org.gnome.TextEditor']));
    assert.ok(!appMatches(['chrom*'], ['firefox.desktop', 'Firefox', 'firefox']));
});

test('an unclaimed swipe is left to the shell', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, Mods.CTRL, []]]);
    assert.deepEqual(c.handle(Phase.BEGIN, 3, 0, ['firefox'], 0, 0), {stop: false, emit: null});
    assert.deepEqual(c.handle(Phase.UPDATE, 3, 0, [], 5, 0), {stop: false, emit: null});
    // Extra modifiers do not match either.
    assert.equal(c.handle(Phase.BEGIN, 3, Mods.CTRL | Mods.SHIFT, ['firefox'], 0, 0).stop, false);
    // Nor does another finger count.
    assert.equal(c.handle(Phase.BEGIN, 4, Mods.CTRL, ['firefox'], 0, 0).stop, false);
});

test('a claimed swipe is stopped and forwarded until it ends', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, Mods.CTRL, []], [4, 0, []]]);
    assert.deepEqual(c.handle(Phase.BEGIN, 3, Mods.CTRL, ['firefox.desktop'], 0, 0),
        {stop: true, emit: [Phase.BEGIN, 3, Mods.CTRL, 'firefox.desktop', 0, 0]});
    // Modifier released mid-swipe: still ours, reported with the claim's mods.
    assert.deepEqual(c.handle(Phase.UPDATE, 3, 0, [], -12.5, 1),
        {stop: true, emit: [Phase.UPDATE, 3, Mods.CTRL, 'firefox.desktop', -12.5, 1]});
    assert.deepEqual(c.handle(Phase.END, 3, 0, [], 0, 0),
        {stop: true, emit: [Phase.END, 3, Mods.CTRL, 'firefox.desktop', 0, 0]});
    // After the end, motion goes back to the shell.
    assert.equal(c.handle(Phase.UPDATE, 3, 0, [], 1, 1).stop, false);
    // A claim without modifiers.
    assert.equal(c.handle(Phase.BEGIN, 4, 0, ['x'], 0, 0).stop, true);
    assert.equal(c.handle(Phase.CANCEL, 4, 0, [], 0, 0).stop, true);
    assert.equal(c.handle(Phase.UPDATE, 4, 0, [], 0, 0).stop, false);
});

test('an app-filtered claim only takes the swipe in that app', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, 0, ['*Firefox*']]]);
    // The calculator's swipe stays GNOME's.
    assert.deepEqual(c.handle(Phase.BEGIN, 3, 0, ['org.gnome.Calculator.desktop'], 0, 0),
        {stop: false, emit: null});
    // Firefox's is claimed, and the signal names the app it started in.
    assert.deepEqual(c.handle(Phase.BEGIN, 3, 0, ['firefox.desktop', 'Firefox'], 0, 0),
        {stop: true, emit: [Phase.BEGIN, 3, 0, 'firefox.desktop', 0, 0]});
    // The app is fixed for the whole gesture, even if focus would change.
    assert.deepEqual(c.handle(Phase.UPDATE, 3, 0, ['org.gnome.Calculator.desktop'], -10, 0),
        {stop: true, emit: [Phase.UPDATE, 3, 0, 'firefox.desktop', -10, 0]});
    assert.equal(c.handle(Phase.END, 3, 0, [], 0, 0).stop, true);
    // A claim with no app filter applies everywhere.
    c.setClaims([[3, 0, []]]);
    assert.equal(c.handle(Phase.BEGIN, 3, 0, ['org.gnome.Calculator.desktop'], 0, 0).stop, true);
});

test('clearing the claims drops a swipe in progress', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, Mods.CTRL, []]]);
    c.handle(Phase.BEGIN, 3, Mods.CTRL, ['firefox'], 0, 0);
    c.setClaims([]);
    assert.equal(c.hasClaims, false);
    assert.equal(c.handle(Phase.UPDATE, 3, Mods.CTRL, [], 1, 0).stop, false);
});

test('keySequence presses the modifiers around the key and releases them in reverse', () => {
    const [, CTRL, , SHIFT] = MODIFIER_KEYS.map(([, sym]) => sym);
    assert.deepEqual(keySequence(0x77, Mods.CTRL), [
        [CTRL, true], [0x77, true], [0x77, false], [CTRL, false],
    ]);
    assert.deepEqual(keySequence(0xff09, Mods.CTRL | Mods.SHIFT), [
        [CTRL, true], [SHIFT, true], [0xff09, true], [0xff09, false],
        [SHIFT, false], [CTRL, false],
    ]);
    assert.deepEqual(keySequence(0xffbe, 0), [[0xffbe, true], [0xffbe, false]]);
});

test('KeyQueue types at once when no stray modifier is down', () => {
    const typed = [];
    const q = new KeyQueue((sym, mods) => typed.push([sym, mods]));
    q.push(0x77, Mods.CTRL, 0);
    assert.equal(q.pump(0, 10), false);
    assert.deepEqual(typed, [[0x77, Mods.CTRL]]);
    // A modifier the shortcut uses itself may be down.
    q.push(0x77, Mods.CTRL, 20);
    assert.equal(q.pump(Mods.CTRL, 30), false);
    assert.equal(typed.length, 2);
});

test('KeyQueue waits for the trigger\'s modifiers to go up, then types', () => {
    const typed = [];
    const q = new KeyQueue((sym, mods) => typed.push([sym, mods]));
    q.push(0xff09, Mods.CTRL, 0);
    // Alt is held by the swipe: Alt+Ctrl+Tab would be the wrong shortcut.
    assert.equal(q.pump(Mods.ALT, 25), true);
    assert.equal(q.pump(Mods.ALT | Mods.CTRL, 50), true);
    assert.deepEqual(typed, []);
    assert.equal(q.pump(0, 75), false);
    assert.deepEqual(typed, [[0xff09, Mods.CTRL]]);
});

test('KeyQueue keeps the order of a chain while it waits', () => {
    const typed = [];
    const q = new KeyQueue((sym, mods) => typed.push([sym, mods]));
    q.push(0x61, Mods.CTRL, 0);
    q.push(0x62, 0, 1);
    q.push(0x63, Mods.SHIFT, 2);
    assert.equal(q.pump(Mods.SUPER, 10), true);
    assert.equal(q.pump(0, 20), false);
    assert.deepEqual(typed, [[0x61, Mods.CTRL], [0x62, 0], [0x63, Mods.SHIFT]]);
});

test('KeyQueue drops what has waited too long, and goes on with the rest', () => {
    const typed = [];
    const dropped = [];
    const q = new KeyQueue((sym, mods) => typed.push([sym, mods]), item => dropped.push(item.keysym));
    q.push(0x61, 0, 0);
    q.push(0x62, Mods.CTRL, KEYS_WAIT_MS - 1);
    // Alt is still down after the wait: the first goes, the second has
    // waited only a moment and keeps waiting.
    assert.equal(q.pump(Mods.ALT, KEYS_WAIT_MS + 1), true);
    assert.deepEqual(dropped, [0x61]);
    assert.deepEqual(typed, []);
    assert.equal(q.pump(0, KEYS_WAIT_MS + 2), false);
    assert.deepEqual(typed, [[0x62, Mods.CTRL]]);
    q.push(0x63, 0, 0);
    q.clear();
    assert.equal(q.pending, false);
});

test('a lone Alt or Super still down after a swipe needs a neutral key', () => {
    assert.equal(shouldNeutralize(Mods.ALT, Mods.ALT), true);
    assert.equal(shouldNeutralize(Mods.SUPER, Mods.SUPER), true);
    assert.equal(shouldNeutralize(Mods.ALT | Mods.CTRL, Mods.ALT), true);
    // Already released, or a modifier that means nothing alone.
    assert.equal(shouldNeutralize(Mods.ALT, 0), false);
    assert.equal(shouldNeutralize(Mods.SUPER, 0), false);
    assert.equal(shouldNeutralize(Mods.ALT, Mods.CTRL), false);
    assert.equal(shouldNeutralize(Mods.CTRL, Mods.CTRL), false);
    assert.equal(shouldNeutralize(Mods.SHIFT, Mods.SHIFT), false);
    assert.equal(shouldNeutralize(0, Mods.ALT), false);
});

test('the neutral key is F19 and is tapped without modifiers', () => {
    // 0xffbe (F1) + 18 = F19, above the keys real keyboards have.
    assert.equal(NEUTRAL_KEYSYM, 0xffd0);
    assert.deepEqual(keySequence(NEUTRAL_KEYSYM, 0), [
        [NEUTRAL_KEYSYM, true], [NEUTRAL_KEYSYM, false],
    ]);
});
