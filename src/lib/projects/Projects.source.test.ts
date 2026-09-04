import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Projects.svelte', import.meta.url), 'utf8');

test('Terminal Projects uses the same update clock as Chat', () => {
  assert.match(source, /import \{ declaredWindowChips, liveWindowChips, projectAgeLabel, shortPath, sortRows \} from '\.\/projects\.ts';/u);
  assert.match(source, /projectList\(\),\s*\n\s*hubRooms\(\)\.catch\(\(\) => null\)/u,
    'the component loads the shared conversation timestamp map');
  assert.match(source, /if \(rooms\) talkMap = rooms\.rooms \?\? \{\};/u,
    'a failed rooms read keeps the last map');
  assert.match(source, /sortRows\(rows, talkMap\)/u, 'sorting consumes that same map');
  assert.equal([...source.matchAll(/projectAgeLabel\(row, talkMap, tick\)/gu)].length, 2,
    'the row and its hover card use the one shared formatter');
});
