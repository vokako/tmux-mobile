// The slop rule: what makes a long-pressable list still scrollable.
import test from 'node:test';
import assert from 'node:assert/strict';
import { isScroll, longpress } from './longpress.ts';

test('a hold that travels is a scroll, not a press', () => {
  const from = { x: 100, y: 200 };
  // Resting fingers wobble by a pixel or three; that is still a press.
  assert.equal(isScroll(from, { x: 100, y: 200 }), false);
  assert.equal(isScroll(from, { x: 103, y: 197 }), false);
  assert.equal(isScroll(from, { x: 110, y: 210 }), false, '10px is the limit, not past it');
  // Past the slop in either axis it is a scroll — a list must stay flickable.
  assert.equal(isScroll(from, { x: 100, y: 211 }), true);
  assert.equal(isScroll(from, { x: 89, y: 200 }), true);
  // Direction does not matter.
  assert.equal(isScroll(from, { x: 100, y: 189 }), true);
  // The slop is a parameter, so a surface with different needs can say so.
  assert.equal(isScroll(from, { x: 100, y: 205 }, 2), true);
});

test('a parent can exclude child gestures without swallowing their next tap (#164)', context => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  function node() {
    const handlers = new Map<string, (event: any) => void>();
    // Only the listener boundary is faked; the production timer/click logic runs.
    const element = {
      addEventListener: (name: string, handler: (event: any) => void) => handlers.set(name, handler),
      removeEventListener: (name: string) => handlers.delete(name),
      getBoundingClientRect: () => ({ left: 0, top: 0, right: 0, bottom: 0 }),
    } as unknown as HTMLElement;
    return { element, fire: (name: string, event: unknown) => handlers.get(name)?.(event) };
  }
  const parent = node(), child = node(), target = new EventTarget();
  let parents = 0, children = 0;
  const outer = longpress(parent.element, { accept: eventTarget => eventTarget !== target, onlongpress: () => parents++ });
  const inner = longpress(child.element, { onlongpress: () => children++ });
  try {
    const touch = { target, touches: [{ clientX: 20, clientY: 40 }] };
    child.fire('touchstart', touch); parent.fire('touchstart', touch);
    context.mock.timers.tick(500);
    assert.equal(children, 1);
    assert.equal(parents, 0, 'background menu must not arm on a file row');
    const click = () => {
      let prevented = false, stopped = false;
      const event = { preventDefault: () => prevented = true, stopPropagation: () => stopped = true };
      parent.fire('click', event);
      if (!stopped) child.fire('click', event);
      return prevented;
    };
    assert.equal(click(), true, 'the child consumes the release click once');
    assert.equal(click(), false, 'no unspent child flag swallows a later keyboard/mouse activation');
    child.fire('touchstart', touch); parent.fire('touchstart', touch);
    child.fire('touchend', touch); parent.fire('touchend', touch);
    assert.equal(click(), false, 'the next genuine tap must reach the row');
  } finally { outer.destroy(); inner.destroy(); }
});

test('a press hands its menu the ELEMENT, left-aligned, not the finger (board #196)', context => {
  // Owner 2026-09-13: "手机上展开选项卡不是以点击焦点展开选项卡，是以元素的左对齐展开，
  // 手机长按之类的也类似，和鼠标操作不一样". A right-click opens at the pointer (OS
  // convention); a hold opens at the thing held. The point still rides along
  // for the two SURFACE callers (Files' directory background) that want it.
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const handlers = new Map<string, (event: any) => void>();
  const element = {
    addEventListener: (name: string, handler: (event: any) => void) => handlers.set(name, handler),
    removeEventListener: (name: string) => handlers.delete(name),
    getBoundingClientRect: () => ({ left: 12, top: 700, right: 123, bottom: 746, width: 111, height: 46 }),
  } as unknown as HTMLElement;
  const got: unknown[] = [];
  const action = longpress(element, { onlongpress: (at) => got.push(at) });
  try {
    handlers.get('touchstart')?.({ target: element, touches: [{ clientX: 60, clientY: 720 }] });
    context.mock.timers.tick(500);
    assert.equal(got.length, 1);
    const at = got[0] as Record<string, unknown>;
    assert.equal(at.x, 60); assert.equal(at.y, 720);
    assert.equal(at.trigger, element);
    assert.equal(at.align, 'left');
    assert.equal(at.keepTriggerClear, true, 'the thing you held stays visible beside its menu');
    assert.deepEqual(at.anchor, { left: 12, top: 700, right: 123, bottom: 746 }, 'anchorOf: the rect in the fixed layer\'s space');
  } finally { action.destroy(); }
});
