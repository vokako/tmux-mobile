import test from 'node:test';
import assert from 'node:assert/strict';
import { horizontalEdges, rowOverflows, scrollEdges } from './scroll-edges.ts';

test('only real hidden horizontal content gets an edge cue (#176)', () => {
  const edges = (scrollLeft: number, scrollWidth = 600, clientWidth = 300) =>
    horizontalEdges({ scrollLeft, scrollWidth, clientWidth });
  assert.deepEqual(edges(0), { before: false, after: true });
  assert.deepEqual(edges(100), { before: true, after: true });
  assert.deepEqual(edges(300), { before: true, after: false });
  assert.deepEqual(edges(299.5), { before: true, after: false });
  assert.deepEqual(edges(-12), { before: false, after: true });
  assert.deepEqual(edges(320), { before: true, after: false });
  assert.deepEqual(edges(0, 300), { before: false, after: false });
  assert.deepEqual(edges(-12, 300), { before: false, after: false });
  assert.deepEqual(edges(0, 600, 0), { before: false, after: false });
});

test('edge cues follow scroll, resized content and child replacement, and clear on disable/destroy', () => {
  const resizeCallbacks: (() => void)[] = [], mutationCallbacks: (() => void)[] = [];
  const observed = new Set<unknown>();
  let disconnected = 0;
  class Resize {
    constructor(fn: () => void) { resizeCallbacks.push(fn); }
    observe(node: unknown) { observed.add(node); }
    disconnect() { observed.clear(); disconnected++; }
  }
  class Mutation {
    constructor(fn: () => void) { mutationCallbacks.push(fn); }
    observe() {}
    disconnect() { disconnected++; }
  }
  const classes = new Set<string>();
  const handlers = new Map<string, () => void>();
  const first = {}, second = {};
  const node = {
    ownerDocument: { defaultView: { ResizeObserver: Resize, MutationObserver: Mutation } },
    scrollLeft: 0, scrollWidth: 600, clientWidth: 300, children: [first],
    classList: {
      toggle(name: string, on: boolean) { if (on) classes.add(name); else classes.delete(name); },
      remove(...names: string[]) { names.forEach(name => classes.delete(name)); },
    },
    addEventListener(name: string, fn: () => void) { handlers.set(name, fn); },
    removeEventListener(name: string) { handlers.delete(name); },
  };
  const action = scrollEdges(node as unknown as HTMLElement);
  assert.ok(classes.has('edge-after'));
  node.scrollLeft = 300; handlers.get('scroll')!();
  assert.deepEqual([...classes], ['edge-before']);
  node.children = [second]; node.scrollLeft = 0; node.scrollWidth = 200;
  mutationCallbacks[0]!();
  assert.equal(observed.has(first), false); assert.ok(observed.has(second));
  assert.equal(classes.size, 0);
  node.scrollWidth = 500; resizeCallbacks[0]!();
  assert.ok(classes.has('edge-after'));
  action.update({ active: false }); assert.equal(classes.size, 0);
  resizeCallbacks[0]!(); assert.equal(classes.size, 0);
  action.update({ active: true }); assert.ok(classes.has('edge-after'));
  action.destroy();
  assert.equal(handlers.size, 0); assert.equal(classes.size, 0);
  assert.ok(disconnected >= 2);
});

test('the single row is measured in its single-row form, even while wrapped (#266)', () => {
  const classes = new Set(['cards', 'expanded']);
  const seen: boolean[] = [];
  const node = {
    get scrollWidth() { seen.push(classes.has('expanded')); return classes.has('expanded') ? 300 : 640; },
    clientWidth: 300,
    classList: {
      contains: (name: string) => classes.has(name),
      add: (name: string) => { classes.add(name); },
      remove: (name: string) => { classes.delete(name); },
    },
  };
  assert.equal(rowOverflows(node, 'expanded'), true, 'the wrapped class is lifted for the read');
  assert.deepEqual(seen, [false]);
  assert.ok(classes.has('expanded'), 'and restored in the same block');
  assert.equal(rowOverflows(node), false, 'without a wrapped class the current form is read');
  assert.equal(rowOverflows({ ...node, clientWidth: 0, scrollWidth: 640 }), false, 'an unlaid-out strip does not overflow');
  classes.delete('expanded');
  assert.equal(rowOverflows({ ...node, scrollWidth: 301 }, 'expanded'), false, 'a subpixel remainder is not overflow');
  assert.equal(classes.has('expanded'), false, 'a strip not wrapped is not wrapped by the probe');
});
