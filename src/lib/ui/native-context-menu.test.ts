import test from 'node:test';
import assert from 'node:assert/strict';
import { handleNativeContextMenu, systemOwnsContextMenu } from './native-context-menu.ts';

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
