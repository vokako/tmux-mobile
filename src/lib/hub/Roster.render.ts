import assert from 'node:assert/strict';
import test from 'node:test';
import { renderHarness, RENDER_TIMEOUT_MS } from '../test/ssr.ts';

// One warm server for the whole render tier (board #178): the environment
// and the svelte runtime are built at import, outside this suite's timer.
const h = await renderHarness();

test('Roster renders the controlled destination strip (#168)', { timeout: RENDER_TIMEOUT_MS }, async (ctx) => {
  const Roster = (await h.load('/src/lib/hub/Roster.svelte')).default;
  const { render } = h;
  const agents = [
    { name: 'runner', window: 1, managed: true, agent: 'kiro', state: 'working', team: 'dev', since: 10 },
    { name: 'solo', window: 3, managed: true, agent: 'claude', state: 'idle', since: 90 },
    { name: 'waiting', window: 2, managed: true, agent: 'codex', state: 'waiting', team: 'dev/review', since: 30 },
  ];
  const view = (props: Record<string, unknown> = {}) => h.fragment(render(Roster, { props: {
    selected: 'fixture', roomReady: true, managedAgents: agents, stopped: ['paused'],
    managedNames: agents.map((agent) => agent.name), busyNames: ['runner', 'waiting'],
    selectedRow: { project: { path: '/fixture' }, live: false, slots: [{ window_name: 'paused', command: 'codex' }] },
    recipient: 'runner', unread: new Set(['runner']), stateLabel: (state: string) => `State: ${state}`,
    ...props,
  } }).body as string);
  const card = (root: DocumentFragment, name: string) => root.querySelector(`.acard[data-agent="${name}"]`)!;
  const select = (root: DocumentFragment, name: string) => card(root, name).querySelector('button.agent-select')!;
  const stop = (root: DocumentFragment, name: string) => card(root, name).querySelector('.agent-stop > button');

  await ctx.test('native sibling controls use global turn order with team identity in ARIA, not group order', () => {
    const root = view();
    assert.equal(root.children.length, 1);
    assert.equal(root.querySelectorAll('.roster').length, 1);
    assert.equal(root.querySelectorAll('.tgroup').length, 0);
    assert.deepEqual([...root.querySelectorAll('.acard[data-agent]')].map((node) => node.getAttribute('data-agent')),
      ['waiting', 'runner', 'solo', 'paused', 'all']);
    assert.match(select(root, 'waiting').getAttribute('aria-label')!, /dev\/review/u);
    assert.doesNotMatch(root.textContent!, /State:|stopped|@all/u, 'state words live only in hover/ARIA');
    assert.equal(select(root, 'runner').getAttribute('aria-pressed'), 'true');
    assert.equal(select(root, 'all').getAttribute('aria-pressed'), 'false');
    assert.equal(select(root, 'waiting').getAttribute('aria-pressed'), 'false');
    assert.match(select(root, 'all').getAttribute('aria-label')!, /everyone/u);
    assert.equal(select(root, 'all').querySelector('.ava'), null, 'broadcast is not another agent avatar (#173)');
    assert.ok(select(root, 'all').querySelector('.broadcast-glyph svg'));
    assert.match(select(root, 'runner').getAttribute('aria-label')!, /unread/iu);
    assert.ok(card(root, 'runner').querySelector('.st.live-dot'));
    assert.ok(card(root, 'runner').querySelector('.unread'));
    assert.equal(card(root, 'runner').querySelector('.agent-marks')!.classList.contains('unmarked'), false);
    assert.equal(card(root, 'waiting').querySelector('.agent-marks')!.classList.contains('unmarked'), true);
    assert.match(card(root, 'waiting').querySelector('.st')!.getAttribute('style')!, /--status-warn/u);
    assert.equal(card(root, 'waiting').querySelector('.st.live-dot'), null);
    assert.equal(stop(root, 'runner')?.parentElement?.parentElement, card(root, 'runner'));
    assert.equal(stop(root, 'runner')?.getAttribute('aria-label'), 'Interrupt runner');
    assert.equal(stop(root, 'all')?.getAttribute('aria-label'), 'Interrupt everyone');
    assert.ok(stop(root, 'runner')?.classList.contains('warn'));
    assert.equal(stop(root, 'runner')?.classList.contains('secondary'), false);
    assert.equal(root.querySelector('.agent-watch'), null, '#180: Watch remains in the existing ContextMenu, not a reserved slot');
    assert.equal(card(root, 'all').querySelector('.agent-watch'), null);
    assert.equal(select(root, 'runner').querySelector('button'), null);
    assert.equal(root.querySelector('.a-menu, [role="menu"], [role="button"]'), null);
  });

  await ctx.test('Stop visibility follows busyNames, never a local state guess', () => {
    const root = view({ busyNames: ['solo'] });
    assert.equal(stop(root, 'runner'), null, 'working without parent membership has no Stop');
    assert.equal(stop(root, 'waiting'), null);
    assert.ok(stop(root, 'solo'), 'parent membership is authoritative even with a stale status label');
    assert.ok(stop(root, 'all'));
    assert.equal(view({ busyNames: [] }).querySelector('.agent-stop'), null);
  });

  await ctx.test('pending blocks its member and all, without dimming selection or peers', () => {
    const root = view({ interrupting: ['runner'] });
    for (const name of ['runner', 'all']) {
      assert.ok(stop(root, name)?.hasAttribute('disabled'));
      assert.equal(stop(root, name)?.getAttribute('aria-busy'), 'true');
    }
    assert.equal(stop(root, 'waiting')?.hasAttribute('disabled'), false);
    assert.equal(select(root, 'runner').hasAttribute('disabled'), false);
    const allPending = view({ interrupting: ['runner', 'waiting'] });
    for (const name of ['all', 'runner', 'waiting']) assert.ok(stop(allPending, name)?.hasAttribute('disabled'));
    const unrelated = view({ interrupting: ['removed'] });
    assert.equal(stop(unrelated, 'all')?.hasAttribute('disabled'), false);
  });

  await ctx.test('body extras mark only reached cards with @, without selecting them', () => {
    const marked = (root: DocumentFragment) => [...root.querySelectorAll('.agent-mention')].map((node) => {
      assert.equal(node.textContent, '@');
      return node.closest('.acard')?.getAttribute('data-agent');
    });
    const extra = view({ composerText: '@waiting, @runner @missing' });
    assert.deepEqual(marked(extra), ['waiting']);
    assert.equal(select(extra, 'runner').getAttribute('aria-pressed'), 'true');
    assert.equal(select(extra, 'waiting').getAttribute('aria-pressed'), 'false');
    assert.deepEqual(marked(view({ composerText: '@all' })), ['waiting', 'runner', 'solo', 'all']);
    assert.deepEqual(marked(view({ recipient: 'all', composerText: '@waiting' })), []);
    assert.equal(select(view({ recipient: 'all' }), 'all').getAttribute('aria-pressed'), 'true');
    const none = view({ recipient: '', composerText: '@waiting' });
    assert.equal(none.querySelector('[aria-pressed="true"]'), null);
    assert.deepEqual(marked(none), ['waiting']);
    assert.equal(view({ recipient: '' }).querySelector('.agent-mention'), null);
  });

  await ctx.test('context usage is a uniform avatar ring, including zero and saturated readings (#180)', () => {
    for (const pct of [0, 20, 60, 85, 100, 125]) {
      const root = view({ managedAgents: [{ ...agents[0], vitals: { context_pct: pct } }] });
      const meter = card(root, 'runner').querySelector<HTMLElement>('.ctx-ring');
      assert.ok(meter, `${pct}% has a visible meter`);
      assert.equal(meter.getAttribute('aria-valuenow'), String(Math.min(100, pct)));
      assert.equal(meter.style.getPropertyValue('--ctx-amount'), `${Math.min(100, pct)}%`);
      assert.ok(card(root, 'runner').querySelector('.avatar-slot .ava'));
    }
    assert.equal(view().querySelector('.ctx-ring'), null, 'unknown is absent, not a guessed zero');
    const expanded = view({ expanded: true, managedAgents: [{ ...agents[0], vitals: { context_pct: 125 } }] });
    assert.equal(expanded.querySelector('.ctx-value')?.textContent, '125%');
  });

  await ctx.test('reading filter and delivery selection have independent card states (#173)', () => {
    const root = view({ filterAgent: 'waiting' });
    assert.ok(card(root, 'waiting').classList.contains('filtered'));
    assert.equal(select(root, 'waiting').getAttribute('aria-pressed'), 'false');
    assert.equal(card(root, 'runner').classList.contains('filtered'), false);
    assert.equal(select(root, 'runner').getAttribute('aria-pressed'), 'true');
  });

  await ctx.test('one controlled disclosure targets the same single list in either mode', () => {
    for (const expanded of [false, true]) {
      const root = view({ expanded });
      const toggle = root.querySelector('.roster-toggle button')!;
      const list = root.querySelector('.cards')!;
      assert.ok(toggle);
      assert.equal(toggle.getAttribute('aria-expanded'), String(expanded));
      assert.equal(toggle.getAttribute('aria-controls'), list.id);
      assert.equal(list.classList.contains('expanded'), expanded);
      assert.equal(root.querySelectorAll('.cards').length, 1);
      assert.equal(root.querySelectorAll('[data-agent=runner]').length, 1);
    }
  });

  await ctx.test('stopped context targets retain backend identity but cannot receive or interrupt', () => {
    const root = view({ acting: true });
    assert.equal(card(root, 'paused').querySelector('img')?.getAttribute('src'), '/assets/codex.svg');
    assert.ok(select(root, 'paused').hasAttribute('disabled'));
    assert.equal(select(root, 'paused').getAttribute('aria-pressed'), null);
    assert.equal(stop(root, 'paused'), null);
    assert.equal(root.querySelector('.a-start'), null);
  });

  await ctx.test('empty/closed rooms retain add; loading and long names remain honest', () => {
    assert.ok(view({ managedAgents: [], stopped: [], managedNames: [], busyNames: [] }).querySelector('.roster-add button'));
    assert.equal(view({ selected: '' }).querySelector('.roster'), null);
    assert.ok(view({ roomReady: false }).querySelector('.sk-cards[aria-hidden="true"]'));
    assert.equal(view({ roomReady: false }).querySelector('[data-agent="all"]'), null, 'no empty-all verdict before first answer');
    const name = 'a-very-long-agent-name-with-identity-intact';
    const root = view({ managedAgents: [{ ...agents[0], name }], managedNames: [name], busyNames: [name] });
    assert.equal(card(root, name).querySelector('.a-name')?.textContent, name);
  });
});
