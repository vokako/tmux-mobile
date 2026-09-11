import assert from 'node:assert/strict';
import test from 'node:test';
import { ALL_TARGET, attachmentBody, attachToken, busyTargetsFor, paletteBackendFor } from './hub-composer.ts';
import { addressed, commandPalette } from './hub.ts';

const roster = [
  { name: 'alice', managed: true, agent: 'kiro' },
  { name: 'bob', managed: true, agent: 'codex' },
  { name: 'shell', managed: false, agent: 'grok' },
];

// #168: card Stop and double Ctrl+C share exactly this target boundary.
test('interrupt targets include only busy managed members of the selected card', () => {
  const agents = [
    ...['running', 'working', 'waiting', 'blocked', 'idle', 'done', 'unknown']
      .map((state) => ({ name: state, state, managed: true })),
    { name: 'shell', state: 'running', managed: false },
    { name: 'missing-state', managed: true },
  ];
  assert.deepEqual(busyTargetsFor(ALL_TARGET, agents), ['running', 'working', 'waiting', 'blocked']);
  for (const name of ['running', 'working', 'waiting', 'blocked']) {
    assert.deepEqual(busyTargetsFor(name, agents), [name]);
  }
  for (const name of ['', 'idle', 'done', 'unknown', 'shell', 'missing-state', 'absent']) {
    assert.deepEqual(busyTargetsFor(name, agents), [], name);
  }
  assert.deepEqual(busyTargetsFor(ALL_TARGET, []), []);
});

test('interrupt targets are a deduplicated activation snapshot, not live roster objects', () => {
  const agents = [
    { name: 'alice', state: 'running', managed: true },
    { name: 'alice', state: 'working', managed: true },
    { name: 'bob', state: 'idle', managed: true },
  ];
  const captured = busyTargetsFor(ALL_TARGET, agents);
  assert.deepEqual(captured, ['alice']);
  agents[0]!.name = 'renamed';
  agents[2]!.state = 'waiting';
  assert.deepEqual(captured, ['alice']);
  assert.deepEqual(busyTargetsFor(ALL_TARGET, agents), ['renamed', 'alice', 'bob']);
});

test('palette uses a leading addressee before the recipient chip', () => {
  assert.equal(paletteBackendFor(' @bob /', 'alice', roster), 'codex');
  assert.equal(paletteBackendFor('note @bob /', 'alice', roster), 'kiro');
  assert.equal(paletteBackendFor('@bob', 'alice', roster), 'kiro',
    'the existing prefix grammar requires whitespace after the name');
  assert.equal(paletteBackendFor('@bob\n/', 'alice', roster), 'codex');
});

test('direct or unknown targets never fall back to the chip or another agent', () => {
  assert.equal(paletteBackendFor('@shell /', 'alice', roster), '');
  assert.equal(paletteBackendFor('@missing /', 'alice', roster), '');
  assert.equal(paletteBackendFor('/', 'shell', roster), '');
  assert.equal(paletteBackendFor('/', 'missing', roster), '');
});

test('all and room choices offer a palette only for one managed dialect', () => {
  const homogeneous = [
    roster[0]!,
    { name: 'reviewer', managed: true, agent: 'kiro' },
    roster[2]!,
  ];
  for (const recipient of [ALL_TARGET, '']) {
    assert.equal(paletteBackendFor('/', recipient, homogeneous), 'kiro',
      'a direct window does not make a managed roster mixed');
    assert.equal(paletteBackendFor('/', recipient, roster), 'mixed');
    assert.equal(commandPalette('/', [], paletteBackendFor('/', recipient, roster)), null);
    assert.equal(paletteBackendFor('/', recipient, []), 'mixed');
  }
});

test('an explicit all prefix retains its existing named-target lookup', () => {
  // Do not turn the prefix parser into mentionTokens during a mechanical move.
  assert.equal(paletteBackendFor('@all /', 'alice', roster), '');
});

test('missing backend values stay empty, not a guessed dialect', () => {
  const unknown = [{ name: 'unknown', managed: true }];
  assert.equal(paletteBackendFor(null, 'unknown', unknown), '');
  assert.equal(paletteBackendFor(undefined, '', unknown), '');
  assert.equal(paletteBackendFor('/', '', [...unknown, roster[0]!]), 'mixed');
});

const image = { kind: 'image' as const, n: 1, path: '/work/shot.webp' };
const file = { kind: 'file' as const, n: 2, path: '/work/report.pdf' };

test('attachment tokens use one spelling for staging, removal and interpolation', () => {
  assert.equal(attachToken(image), '[img:1]');
  assert.equal(attachToken(file), '[file:2]');
});

test('images and files replace tokens at their positions, not in upload order', () => {
  const raw = 'Read [file:2], then inspect [img:1].';
  const body = attachmentBody(raw, [image, file]);
  assert.equal(body, 'Read /work/report.pdf, then inspect ![](/work/shot.webp).');
  assert.equal(addressed(body, 'alice'),
    '@alice Read /work/report.pdf, then inspect ![](/work/shot.webp).');
});

test('deleted tokens append references in attachment order without dropping content', () => {
  assert.equal(attachmentBody('Read this', [image, file]),
    'Read this\n![](/work/shot.webp)\n/work/report.pdf');
  assert.equal(attachmentBody('', [file, image]), '/work/report.pdf\n![](/work/shot.webp)');
  assert.equal(attachmentBody('See [img:1]', [image, file]),
    'See ![](/work/shot.webp)\n/work/report.pdf');
});

test('interpolation replaces only the first matching token and leaves unknown tokens', () => {
  assert.equal(attachmentBody('[img:1] [img:1] [file:99]', [image]),
    '![](/work/shot.webp) [img:1] [file:99]');
  assert.equal(attachmentBody('[img:10] [img:1]', [image]),
    '[img:10] ![](/work/shot.webp)');
  assert.equal(attachmentBody(' \n text \n ', []), ' \n text \n ',
    'trimming remains the send caller responsibility');
});

test('interpolation preserves String.replace replacement-string semantics', () => {
  // A replacement callback would change dollar expansion in existing paths.
  assert.equal(attachmentBody('[file:2]', [{ ...file, path: '/work/$&.txt' }]),
    '/work/[file:2].txt');
});
