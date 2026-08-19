// The popover placement contract — pure geometry, no browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { menuPlacement } from './placement.ts';

test('menuPlacement puts a context menu beside its trigger, inside the viewport', () => {
  const view = { w: 1200, h: 800 };
  const size = { w: 180, h: 200 };
  // Normal case: below the trigger, right edges aligned (the trigger is a dot
  // menu at a chip's right edge).
  const mid = { left: 420, right: 460, top: 300, bottom: 330 };
  assert.deepEqual(menuPlacement(mid, size, view), { x: 280, y: 336 });
  // Not enough room below → flip above, keeping the same 6px gap.
  const low = { left: 420, right: 460, top: 700, bottom: 730 };
  assert.deepEqual(menuPlacement(low, size, view), { x: 280, y: 494 });
  // A trigger near the left edge would put a right-aligned menu off screen.
  const left = { left: 10, right: 40, top: 100, bottom: 130 };
  assert.deepEqual(menuPlacement(left, size, view), { x: 8, y: 136 });
  // …and near the right edge it must not overflow either.
  const right = { left: 1180, right: 1198, top: 100, bottom: 130 };
  assert.equal(menuPlacement(right, size, view).x, 1012, 'clamped to view.w - w - 8');
  // Taller than the viewport: pinned to the top edge rather than pushed off it.
  assert.equal(menuPlacement(low, { w: 180, h: 900 }, view).y, 8);
  // Unmeasured height (first frame) must not trigger a flip on a guess.
  assert.equal(menuPlacement(low, { w: 180, h: 0 }, view).y, 736);
});
