// Unit tests for the GNOME-free part of the extension: node --test test/
import assert from 'node:assert/strict';
import {test} from 'node:test';

import {Mods, Phase, SwipeClaimer, modsFromState} from
    '../../gnome-extension/window-jump@hjosugi.github.io/lib.js';

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

test('an unclaimed swipe is left to the shell', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, Mods.CTRL]]);
    assert.deepEqual(c.handle(Phase.BEGIN, 3, 0, 0, 0), {stop: false, emit: null});
    assert.deepEqual(c.handle(Phase.UPDATE, 3, 0, 5, 0), {stop: false, emit: null});
    // Extra modifiers do not match either.
    assert.equal(c.handle(Phase.BEGIN, 3, Mods.CTRL | Mods.SHIFT, 0, 0).stop, false);
    // Nor does another finger count.
    assert.equal(c.handle(Phase.BEGIN, 4, Mods.CTRL, 0, 0).stop, false);
});

test('a claimed swipe is stopped and forwarded until it ends', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, Mods.CTRL], [4, 0]]);
    assert.deepEqual(c.handle(Phase.BEGIN, 3, Mods.CTRL, 0, 0),
        {stop: true, emit: [Phase.BEGIN, 3, Mods.CTRL, 0, 0]});
    // Modifier released mid-swipe: still ours, reported with the claim's mods.
    assert.deepEqual(c.handle(Phase.UPDATE, 3, 0, -12.5, 1),
        {stop: true, emit: [Phase.UPDATE, 3, Mods.CTRL, -12.5, 1]});
    assert.deepEqual(c.handle(Phase.END, 3, 0, 0, 0),
        {stop: true, emit: [Phase.END, 3, Mods.CTRL, 0, 0]});
    // After the end, motion goes back to the shell.
    assert.equal(c.handle(Phase.UPDATE, 3, 0, 1, 1).stop, false);
    // A claim without modifiers.
    assert.equal(c.handle(Phase.BEGIN, 4, 0, 0, 0).stop, true);
    assert.equal(c.handle(Phase.CANCEL, 4, 0, 0, 0).stop, true);
    assert.equal(c.handle(Phase.UPDATE, 4, 0, 0, 0).stop, false);
});

test('clearing the claims drops a swipe in progress', () => {
    const c = new SwipeClaimer();
    c.setClaims([[3, Mods.CTRL]]);
    c.handle(Phase.BEGIN, 3, Mods.CTRL, 0, 0);
    c.setClaims([]);
    assert.equal(c.hasClaims, false);
    assert.equal(c.handle(Phase.UPDATE, 3, Mods.CTRL, 1, 0).stop, false);
});
