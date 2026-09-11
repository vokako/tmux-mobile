// The popover placement contract — pure geometry, no browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { menuHeightLimit, menuPlacement, pointAnchor, popOrigin } from './placement.ts';

test('an opt-in trigger-clear cap keeps a tall menu on one side even in a short viewport (#173)', () => {
  for (const [top, bottom, height, expected] of [
    [90, 118, 200, 76], [20, 48, 200, 138], [164, 192, 200, 150],
    [300, 328, 800, 458], [0, 200, 200, 0],
    [20.25, 48.25, 200, 137],
  ]) {
    const anchor = { left: 40, right: 150, top: top!, bottom: bottom! };
    const view = { w: 390, h: height! };
    const limit = menuHeightLimit(anchor, view);
    assert.equal(limit, expected);
    if (!limit) continue; // No room on either side is physically unsatisfiable.
    const h = Math.round(Math.min(500, limit)); // offsetHeight reports integer CSS pixels.
    const pos = menuPlacement(anchor, { w: 180, h }, view);
    assert.ok(pos.y + h <= anchor.top - 6 || pos.y >= anchor.bottom + 6);
    assert.ok(pos.y >= 8 && pos.y + h <= view.h - 8);
  }
  const anchor = { left: 40, right: 150, top: 90, bottom: 118 };
  const legacy = menuPlacement(anchor, { w: 180, h: 184 }, { w: 390, h: 200 });
  assert.ok(legacy.y < anchor.bottom && legacy.y + 184 > anchor.top,
    'negative control: viewport-only clamping covers the second click');
});

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

test('a right-click or a long press anchors the menu on the pointer', () => {
  const view = { w: 1200, h: 800 };
  const size = { w: 180, h: 200 };
  // A pointer is a zero-size rect, so the ONE placement rule applies: the menu's
  // right edge lands on the pointer, 6px below it.
  const at = pointAnchor(500, 300);
  assert.deepEqual(at, { left: 500, right: 500, top: 300, bottom: 300 });
  assert.deepEqual(menuPlacement(at, size, view), { x: 320, y: 306 });
  // Near the bottom it flips above the pointer instead of hanging off screen.
  assert.deepEqual(menuPlacement(pointAnchor(500, 780), size, view), { x: 320, y: 574 });
  // Near the left edge it is clamped, not right-aligned into the void.
  assert.deepEqual(menuPlacement(pointAnchor(40, 300), size, view), { x: 8, y: 306 });
});

test('left alignment expands a NAME downward — same clamp, same flip (board #32)', async () => {
  const view = { w: 1200, h: 800 };
  const size = { w: 260, h: 200 };
  // The menu's LEFT edge sits on the name's left edge, 6px below it.
  const name = { left: 300, right: 420, top: 50, bottom: 78 };
  assert.deepEqual(menuPlacement(name, size, view, 6, 8, 'left'), { x: 300, y: 84 });
  // Near the right viewport edge the clamp still wins — the menu is never
  // clipped, which is the bug this alignment exists to fix.
  const tight = { left: 1100, right: 1190, top: 50, bottom: 78 };
  assert.deepEqual(menuPlacement(tight, size, view, 6, 8, 'left'), { x: 932, y: 84 }, 'clamped to view.w - w - 8');
  // Near the left edge it cannot go under the 8px margin either.
  assert.equal(menuPlacement({ left: 2, right: 60, top: 50, bottom: 78 }, size, view, 6, 8, 'left').x, 8);
  // The flip above is shared verbatim with the right alignment.
  const low = { left: 300, right: 420, top: 700, bottom: 730 };
  assert.deepEqual(menuPlacement(low, size, view, 6, 8, 'left'), { x: 300, y: 494 });
  // And the DEFAULT stays right-aligned: every existing caller, unchanged.
  assert.deepEqual(menuPlacement(name, size, view), { x: 160, y: 84 });

  // The anchor is zoom-compatible: anchorOf divides the element's client rect
  // (visual px) by --ui-zoom, landing in the fixed layer's own pixel space —
  // the 46px-drift class of bug (board #21's lesson) cannot come back through
  // this entry. Simulated DOM: a rect at 2x zoom halves.
  const g = globalThis as Record<string, unknown>;
  const hadDoc = 'document' in g, oldDoc = g.document;
  const hadGcs = 'getComputedStyle' in g, oldGcs = g.getComputedStyle;
  g.document = { documentElement: {} };
  g.getComputedStyle = () => ({ getPropertyValue: () => '2' });
  try {
    const { anchorOf } = await import('./placement.ts');
    const el = { getBoundingClientRect: () => ({ left: 600, right: 840, top: 100, bottom: 128 }) } as unknown as Element;
    assert.deepEqual(anchorOf(el), { left: 300, right: 420, top: 50, bottom: 64 });
  } finally {
    if (hadDoc) g.document = oldDoc; else delete g.document;
    if (hadGcs) g.getComputedStyle = oldGcs; else delete g.getComputedStyle;
  }
});

test('popOrigin names the corner a popover grows from', () => {
  const anchor = { left: 420, right: 460, top: 300, bottom: 330 };
  assert.equal(popOrigin(anchor, { x: 280, y: 336 }), 'top right', 'below, right-aligned → grows from its top right');
  assert.equal(popOrigin(anchor, { x: 420, y: 336 }, 'left'), 'top left');
  assert.equal(popOrigin(anchor, { x: 280, y: 94 }), 'bottom right', 'flipped above → grows from its bottom edge');
  assert.equal(popOrigin(anchor, { x: 420, y: 94 }, 'left'), 'bottom left');
});
