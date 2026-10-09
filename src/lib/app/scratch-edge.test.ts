import test from 'node:test';
import assert from 'node:assert/strict';
import { readScratchEdge, SCRATCH_EDGE_KEY, writeScratchEdge, type ScratchEdge } from './scratch-edge.ts';

const store = (seed?: string) => {
  const map = new Map<string, string>(seed === undefined ? [] : [[SCRATCH_EDGE_KEY, seed]]);
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    raw: () => map.get(SCRATCH_EDGE_KEY),
  };
};

test('a stored left edge becomes the right edge, once (board #326)', () => {
  // Owner, 2026-10-09: "这个 terminal 应该可以显示在下方或右侧". Same axis,
  // same stored width — anyone who had chosen `left` wants the vertical dock,
  // not a trip back to the bottom.
  const s = store('left');
  assert.equal(readScratchEdge(s), 'right');
  assert.equal(s.raw(), 'right', 'written back, so the old value cannot resurface');
  assert.equal(readScratchEdge(s), 'right', 'and the second read is a plain read');
});

test('right and bottom round-trip; anything else is the bottom edge', () => {
  for (const [seed, expected] of [['right', 'right'], ['bottom', 'bottom'],
    [undefined, 'bottom'], ['', 'bottom'], ['top', 'bottom'], ['LEFT', 'bottom']] as const) {
    const s = store(seed);
    assert.equal(readScratchEdge(s), expected, `${String(seed)} → ${expected}`);
  }
  // An unset key stays unset: a reader never invents a preference.
  const fresh = store();
  readScratchEdge(fresh);
  assert.equal(fresh.raw(), undefined);
  for (const edge of ['bottom', 'right'] as ScratchEdge[]) {
    const s = store();
    writeScratchEdge(s, edge);
    assert.equal(readScratchEdge(s), edge);
  }
});
