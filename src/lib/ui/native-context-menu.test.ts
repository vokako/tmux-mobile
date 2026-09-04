import test from 'node:test';
import assert from 'node:assert/strict';
import { handleNativeContextMenu, selectionClickGuard, systemOwnsContextMenu } from './native-context-menu.ts';

function event(pointerType?: string, firesTouchEvents = false) {
  let prevented = false;
  return {
    pointerType,
    sourceCapabilities: { firesTouchEvents },
    preventDefault() { prevented = true; },
    get prevented() { return prevented; },
  };
}

test('desktop contextmenu never exposes browser chrome', () => {
  for (const pointerType of ['mouse', '', undefined]) {
    const e = event(pointerType);
    assert.equal(systemOwnsContextMenu(e), false);
    assert.equal(handleNativeContextMenu(e), true);
    assert.equal(e.prevented, true, `${String(pointerType)} is suppressed`);
  }
});

test('touch and pen holds remain native text-selection gestures', () => {
  for (const pointerType of ['touch', 'pen']) {
    const e = event(pointerType);
    assert.equal(systemOwnsContextMenu(e), true);
    assert.equal(handleNativeContextMenu(e), false);
    assert.equal(e.prevented, false, `${pointerType} stays native`);
  }
  // Older WebKit exposes a MouseEvent, but sourceCapabilities still identifies
  // the synthetic mouse event fired from a finger.
  const legacyTouch = event(undefined, true);
  assert.equal(systemOwnsContextMenu(legacyTouch), true);
  assert.equal(handleNativeContextMenu(legacyTouch), false);
  assert.equal(legacyTouch.prevented, false);
});


test('a native selection hold consumes exactly its following bubble click', () => {
  const guard = selectionClickGuard(800);
  const touch = event('touch');

  assert.equal(guard.mark(touch, 'message-1', 100), true, 'touch contextmenu starts the click guard');
  assert.equal(touch.prevented, false, 'native selection is never prevented');
  assert.equal(guard.consume('message-1', 200), true, 'the compatibility click is swallowed');
  assert.equal(guard.consume('message-1', 201), false, 'only one click is consumed');

  assert.equal(guard.mark(event('mouse'), 'message-1', 300), false, 'mouse contextmenu never arms it');
  assert.equal(guard.consume('message-1', 301), false, 'ordinary clicks stay active');

  guard.mark(event(undefined, true), 'message-1', 400);
  assert.equal(guard.consume('message-2', 401), false, 'another bubble is never swallowed');
  assert.equal(guard.consume('message-1', 402), false, 'a different click also clears the stale mark');

  guard.mark(event('pen'), 'message-1', 500);
  assert.equal(guard.consume('message-1', 1301), false, 'an expired mark cannot swallow a later tap');
});