import test from 'node:test';
import assert from 'node:assert/strict';

// Board #323 review P1-b: an OPEN card re-reads its getter, so a state that
// changes while it is shown (the local server going starting -> failed) is
// what the card says; a card shown without a getter keeps its snapshot.
(globalThis as any).$state = (v: unknown) => v;
const { hoverCard } = await import('./hover.svelte.ts');

test('an open hover card follows its getter (#323)', () => {
  const anchor = { left: 0, top: 0, right: 1, bottom: 1, width: 1, height: 1 } as never;
  let mode = 'starting';
  const info = () => ({ title: 'srv', lines: [{ label: 'This computer', value: mode }] });
  hoverCard.show(anchor, info(), 'left', info);
  assert.equal(hoverCard.current!.info.lines![0]!.value, 'starting');
  mode = 'failed';
  assert.equal(hoverCard.current!.info.lines![0]!.value, 'failed', 'the open card changed with the state');
  hoverCard.show(anchor, { title: 'plain' });
  assert.equal(hoverCard.current!.info.title, 'plain', 'no getter: the snapshot');
  hoverCard.hide();
  assert.equal(hoverCard.current, null);
});
