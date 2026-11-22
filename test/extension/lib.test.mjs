// Unit tests for the GNOME-free part of the extension: node --test test/
import assert from 'node:assert/strict';
import {test} from 'node:test';

import {
    Mods, Phase, SwipeClaimer, appMatches, modsFromState, patternMatches,
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
