import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./AgentsPage.svelte', import.meta.url), [new URL('../core/ws.ts', import.meta.url)]);
let hosted: ReturnType<typeof compileMount> | undefined;
function hostFixture() {
  return hosted ??= compileMount(new URL('./AgentsPage.test.svelte', import.meta.url), [new URL('../core/ws.ts', import.meta.url)]);
}
const alpha = { name: 'alpha', backend: 'codex', model: '', effort: '', system: 'Original',
  skills: '["missing-skill"]', mcp: '["missing-server",{"name":"inline","expanded":true}]' };
function rpc(extra: Record<string, (...args: any[]) => unknown> = {}) {
  return {
    registryList: async () => ({ agents: [alpha, { ...alpha, name: 'beta' }] }),
    teamsList: async () => ({ teams: [] }),
    skillsList: async () => ({ skills: [] }),
    mcpList: async () => ({ mcp: [] }),
    modelsList: async () => ({ models: [] }),
    ...extra,
  };
}
type App = Awaited<ReturnType<Awaited<typeof compiled>['mount']>>;
function command(app: App, name: string) {
  const button = app.document.querySelector<HTMLButtonElement>(`[aria-label="${name}"]`);
  assert.ok(button, name);
  return button;
}
async function openAgent(app: App, name = 'alpha') {
  for (let i = 0; i < 12 && !app.document.querySelector('.r-name'); i++) await app.flush();
  const row = [...app.document.querySelectorAll<HTMLButtonElement>('.side-row')]
    .find(button => button.querySelector('.r-name')?.textContent === name);
  assert.ok(row, name); row.click(); await app.flush();
}
async function text(app: App, value: string) {
  const field = app.document.querySelector<HTMLTextAreaElement>('.editor textarea')!;
  field.value = value;
  field.dispatchEvent(new app.window.Event('input', { bubbles: true }));
  await app.flush();
}

test('Agent Save is disabled for an unchanged definition (#156)', async context => {
  const app = await (await compiled).mount(context, { props: { visible: true, section: 'agents' }, modules: [rpc()] });
  try {
    await openAgent(app);
    assert.equal(command(app, 'Save').disabled, true);
  } finally { await app.close(); }
});

test('Agent Cancel protects the draft, and Keep editing preserves its text (#156)', async context => {
  const app = await (await compiled).mount(context, { props: { visible: true, section: 'agents' }, modules: [rpc()] });
  try {
    await openAgent(app);
    await text(app, 'Unsaved draft');
    command(app, 'Cancel').click(); await app.flush();
    assert.ok(app.document.querySelector('[role=alertdialog]'), 'cancel cannot silently discard');
    command(app, 'Keep editing').click(); await app.flush();
    assert.equal(app.document.querySelector('textarea')?.value, 'Unsaved draft');
    command(app, 'Cancel').click(); await app.flush();
    command(app, 'Discard').click(); await app.flush();
    assert.equal(app.document.querySelector('.editor'), null);
  } finally { await app.close(); }
});

test('Agent Save freezes one payload, blocks duplicates and retains a failed draft for retry (#156)', async context => {
  const writes: any[] = [];
  let reject!: (reason: Error) => void;
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents' },
    modules: [rpc({ registrySave: value => { writes.push(value); return new Promise((_, no) => reject = no); } })],
  });
  try {
    await openAgent(app); await text(app, 'Edited');
    command(app, 'Save').click(); command(app, 'Save').click(); await app.flush();
    assert.equal(writes.length, 1, 'one in-flight write');
    assert.equal(command(app, 'Save').getAttribute('aria-busy'), 'true');
    assert.equal(app.document.querySelector('textarea')?.matches(':disabled'), true, 'submitted fields are locked');
    command(app, 'Cancel').click(); await app.flush();
    assert.ok(app.document.querySelector('.editor'), 'pending exit does not drop the submitted draft');
    assert.equal(writes[0].name, 'alpha');
    assert.equal(writes[0].skills, alpha.skills);
    assert.equal(writes[0].mcp, alpha.mcp);
    reject(new Error('Save failed')); await app.flush();
    assert.equal(app.document.querySelector('textarea')?.value, 'Edited');
    assert.equal(command(app, 'Save').disabled, false);
    assert.match(app.document.querySelector('.err')?.textContent ?? '', /Save failed/);
  } finally { await app.close(); }
});

test('the embedding host uses the same discard guard, including a request held during Save (#156)', async context => {
  let leave!: (action: () => void) => void;
  let finish!: () => void;
  let exits = 0;
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents', onGuardExit: (guard: typeof leave) => { leave = guard; } },
    modules: [rpc({ registrySave: () => new Promise<void>(yes => finish = yes) })],
  });
  try {
    await openAgent(app); await text(app, 'Changed');
    leave(() => exits++); await app.flush();
    assert.equal(exits, 0);
    command(app, 'Keep editing').click(); await app.flush();
    command(app, 'Save').click(); await app.flush();
    leave(() => exits++); await app.flush();
    assert.equal(exits, 0);
    finish();
    for (let i = 0; i < 6; i++) await app.flush();
    assert.equal(exits, 1, 'a held host request is not silently lost');
    assert.equal(app.document.querySelector('.editor'), null);
  } finally { await app.close(); }
});

test('Back covers global instructions and a failed read cannot enable Save (#156)', async context => {
  let back!: () => boolean;
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents', onGoBack: (fn: typeof back) => { back = fn; } },
    modules: [rpc({ globalPromptGet: async () => { throw new Error('Read failed'); } })],
  });
  try {
    app.document.querySelector<HTMLElement>('.ava.global')!.closest('button')!.click();
    for (let i = 0; i < 4; i++) await app.flush();
    assert.equal(command(app, 'Save').disabled, true);
    assert.match(app.document.querySelector('.err')?.textContent ?? '', /Read failed/);
    assert.equal(back(), true); await app.flush();
    assert.equal(app.document.querySelector('.editor'), null);
  } finally { await app.close(); }
});

test('a late global read cannot overwrite a newer opening of the same document (#156)', async context => {
  const reads: ((value: unknown) => void)[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents' },
    modules: [rpc({ globalPromptGet: () => new Promise(yes => reads.push(yes)) })],
  });
  try {
    const open = () => app.document.querySelector<HTMLElement>('.ava.global')!.closest('button')!.click();
    open(); await app.flush();
    command(app, 'Cancel').click(); await app.flush();
    open(); await app.flush();
    reads[1]!({ text: 'New snapshot', path: '/config/AGENTS.md', max_bytes: 4096 });
    await app.flush();
    reads[0]!({ text: 'Old snapshot', path: '/config/AGENTS.md', max_bytes: 4096 });
    await app.flush();
    assert.equal(app.document.querySelector('textarea')?.value, 'New snapshot');
    assert.equal(command(app, 'Save').disabled, true);
  } finally { await app.close(); }
});

test('Ctrl/Cmd Enter shares Save, ignores IME and blocks repeated submission (#156)', async context => {
  let writes = 0;
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents' },
    modules: [rpc({ registrySave: () => { writes++; return new Promise(() => {}); } })],
  });
  try {
    await openAgent(app); await text(app, 'Changed');
    const field = app.document.querySelector('textarea')!;
    for (const ime of [{ isComposing: true }, { keyCode: 229 }]) {
      field.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, ...ime }));
    }
    assert.equal(writes, 0);
    for (const modifier of [{ ctrlKey: true }, { metaKey: true }]) {
      field.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true, ...modifier }));
    }
    await app.flush();
    assert.equal(writes, 1);
  } finally { await app.close(); }
});

test('an invalid MCP draft disables Save without discarding its JSON (#156)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'mcp' },
    modules: [rpc({ mcpList: async () => ({ mcp: [{ name: 'files', def: '{"command":"test"}' }] }) })],
  });
  try {
    await openAgent(app, 'files');
    assert.equal(command(app, 'Save').disabled, true);
    await text(app, '{');
    assert.equal(command(app, 'Save').disabled, true);
    command(app, 'Cancel').click(); await app.flush();
    assert.ok(app.document.querySelector('[role=alertdialog]'));
    command(app, 'Keep editing').click(); await app.flush();
    assert.equal(app.document.querySelector('textarea')?.value, '{');
  } finally { await app.close(); }
});

test('a successful unnamed Skill import returns to its list, not another editor (#156)', async context => {
  const imports: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'skills' },
    modules: [rpc({ skillsImport: async (source: string) => { imports.push(source); return { imported: ['imported'], skipped: [] }; } })],
  });
  try {
    app.document.querySelector<HTMLButtonElement>('.side-row.add')!.click(); await app.flush();
    const source = app.document.querySelectorAll<HTMLInputElement>('.editor input')[1]!;
    source.value = '/fixture';
    source.dispatchEvent(new app.window.Event('input', { bubbles: true })); await app.flush();
    command(app, 'Import').click();
    for (let i = 0; i < 6; i++) await app.flush();
    assert.deepEqual(imports, ['/fixture']);
    assert.equal(app.document.querySelector('.editor'), null);
  } finally { await app.close(); }
});

test('a hidden page retains its draft and a held deep link opens only after the captured save (#156)', async context => {
  const fixture = await hostFixture();
  let controls!: { show: (value: boolean) => void; edit: (name: string) => void };
  let finish!: () => void;
  let finishReload!: (value: unknown) => void;
  let reads = 0;
  const writes: any[] = [];
  const app = await fixture.mount(context, {
    props: { ready: (value: typeof controls) => controls = value },
    modules: [rpc({
      registryList: () => ++reads === 1 ? Promise.resolve({ agents: [alpha, { ...alpha, name: 'beta' }] })
        : new Promise(yes => finishReload = yes),
      registrySave: value => { writes.push(value); return new Promise<void>(yes => finish = yes); },
    })],
  });
  try {
    await openAgent(app); await text(app, 'Persist across tabs');
    controls.show(false); await app.flush();
    controls.show(true); await app.flush();
    assert.equal(app.document.querySelector('textarea')?.value, 'Persist across tabs');
    command(app, 'Save').click(); await app.flush();
    controls.edit('beta'); await app.flush();
    assert.equal(app.document.querySelector('.mid h1')?.textContent, 'alpha');
    finish();
    for (let i = 0; i < 5; i++) await app.flush();
    assert.equal(writes[0].name, 'alpha', 'target captured at activation');
    assert.equal(app.document.querySelector('.mid h1')?.textContent, 'beta');
    finishReload({ agents: [alpha, { ...alpha, name: 'beta' }] });
    for (let i = 0; i < 5; i++) await app.flush();
    assert.equal(app.document.querySelector('.mid h1')?.textContent, 'beta', 'late catalog refresh does not close the newer editor');
    assert.equal(app.document.querySelector('textarea')?.value, 'Original');
  } finally { await app.close(); }
});

test('Team disclosure is not an edit; its actual role uses the guarded Save (#156)', async context => {
  const writes: any[] = [];
  const team = { name: 'squad', description: 'Rules', members: JSON.stringify([
    { name: 'dev', base: 'alpha', role: 'Implement' }, { name: 'reviewer', base: 'beta', role: 'Review' },
  ]) };
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'teams' },
    modules: [rpc({ teamsList: async () => ({ teams: [team] }), teamsSave: async value => { writes.push(value); return {}; } })],
  });
  try {
    await openAgent(app, 'squad');
    assert.equal(command(app, 'Save').disabled, true);
    app.document.querySelectorAll<HTMLButtonElement>('.member-summary')[1]!.click(); await app.flush();
    assert.equal(command(app, 'Save').disabled, true, 'folding does not mutate the payload');
    const role = app.document.querySelector<HTMLTextAreaElement>('.member-body textarea')!;
    role.value = 'Review carefully';
    role.dispatchEvent(new app.window.Event('input', { bubbles: true })); await app.flush();
    command(app, 'Save').click(); command(app, 'Save').click();
    for (let i = 0; i < 5; i++) await app.flush();
    assert.equal(writes.length, 1);
    assert.equal(JSON.parse(writes[0].members)[1].role, 'Review carefully');
    assert.equal(app.document.querySelector('.editor'), null);
  } finally { await app.close(); }
});

test('Escape enters the same dirty-exit guard after the Select has dismissed itself (#156)', async context => {
  const app = await (await compiled).mount(context, { props: { visible: true, section: 'agents' }, modules: [rpc()] });
  try {
    await openAgent(app); await text(app, 'Unsaved');
    const select = app.document.querySelector<HTMLButtonElement>('.sel-trigger')!;
    select.click(); await app.flush();
    const escape = () => select.dispatchEvent(new app.window.KeyboardEvent('keydown', {
      key: 'Escape', bubbles: true, cancelable: true,
    }));
    escape(); await app.flush();
    assert.equal(app.document.querySelector('[role=listbox]'), null);
    assert.equal(app.document.querySelector('[role=alertdialog]'), null, 'first Escape belongs to the Select');
    escape(); await app.flush();
    assert.ok(app.document.querySelector('[role=alertdialog]'), 'second Escape protects the draft');
  } finally { await app.close(); }
});

test('Skill refresh invalidates older same-path file and file-list replies (#156)', async context => {
  const files: ((value: unknown) => void)[] = [], lists: ((value: unknown) => void)[] = [];
  const skill = { name: 'docs', source: '/fixture/docs', description: 'Documentation' };
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'skills' },
    modules: [rpc({
      skillsList: async () => ({ skills: [skill] }),
      skillsRefresh: async () => ({}),
      skillsFiles: () => new Promise(yes => lists.push(yes)),
      skillsFile: () => new Promise(yes => files.push(yes)),
    })],
  });
  try {
    await openAgent(app, 'docs');
    command(app, 'Refresh').click();
    for (let i = 0; i < 5; i++) await app.flush();
    lists[1]!({ files: [{ path: 'SKILL.md', size: 200 }, { path: 'new.txt', size: 10 }] });
    files[1]!({ content: 'New documentation' });
    await app.flush();
    files[0]!({ content: 'Old documentation' });
    lists[0]!({ files: [{ path: 'SKILL.md', size: 100 }] });
    await app.flush();
    assert.match(app.document.querySelector('.md-doc')?.textContent ?? '', /New documentation/);
    assert.equal(app.document.querySelectorAll('.file-row').length, 2);
  } finally { await app.close(); }
});

test('compact configuration confirmations use the existing sheet form (#156)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents' }, modules: [rpc()],
    setup(window) {
      const match = window.matchMedia;
      window.matchMedia = query => Object.assign(match(query), { matches: query === '(max-width: 760px)' });
    },
  });
  try {
    await openAgent(app); await text(app, 'Unsaved');
    command(app, 'Cancel').click(); await app.flush();
    assert.ok(app.document.querySelector('[role=alertdialog]')?.classList.contains('sheet'));
  } finally { await app.close(); }
});

test('a built-in Skill description is readable without an edit affordance (#156)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'skills' },
    modules: [rpc({
      skillsList: async () => ({ skills: [{ name: 'builtin', source: 'builtin', description: 'Read-only description' }] }),
      skillsFiles: async () => ({ files: [] }), skillsFile: async () => ({ content: '' }),
    })],
  });
  try {
    await openAgent(app, 'builtin');
    assert.equal(app.document.querySelector('.desc-view'), null);
    assert.equal(app.document.querySelector('.desc-readonly')?.textContent, 'Read-only description');
    assert.equal(command(app, 'Save').disabled, true);
  } finally { await app.close(); }
});

// #167, 2026-09-12: baseline errors were already visible in the modal note.
// Separate alert/error ownership while preserving busy/Back and retry behavior.
for (const kind of ['agent', 'team', 'skill', 'mcp'] as const) {
  test(`${kind} deletion preserves busy Back and retry with a separate modal alert (#167)`, async context => {
    const names = { agent: 'alpha', team: 'squad', skill: 'docs', mcp: 'files' };
    const section = { agent: 'agents', team: 'teams', skill: 'skills', mcp: 'mcp' }[kind];
    const deletes = { agent: 'registryDelete', team: 'teamsDelete', skill: 'skillsDelete', mcp: 'mcpDelete' };
    const calls: string[] = [];
    let back!: () => boolean, reject!: (error: Error) => void;
    const app = await (await compiled).mount(context, {
      props: { visible: true, section, onGoBack: (value: typeof back) => back = value },
      modules: [rpc({
        teamsList: async () => ({ teams: [{ name: 'squad', description: '', members: '[{"name":"dev","base":"alpha"}]' }] }),
        skillsList: async () => ({ skills: [{ name: 'docs', source: '/fixture', description: '' }] }),
        skillsFiles: async () => ({ files: [] }), skillsFile: async () => ({ content: '' }),
        mcpList: async () => ({ mcp: [{ name: 'files', def: '{"command":"test"}' }] }),
        [deletes[kind]]: (name: string) => { calls.push(name); return new Promise((_, no) => reject = no); },
      })],
    });
    try {
      await openAgent(app, names[kind]);
      command(app, 'Delete').click(); await app.flush();
      const confirm = () => app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!;
      confirm().click(); confirm().click(); await app.flush();
      assert.equal(back(), true, 'busy Back is consumed');
      app.document.querySelector<HTMLElement>('.dlg-backdrop')!.click();
      const escape = new app.window.KeyboardEvent('keydown', { key: 'Escape', cancelable: true });
      app.window.dispatchEvent(escape); await app.flush();
      assert.equal(escape.defaultPrevented, true);
      assert.equal(app.document.querySelector('[role=alertdialog]')?.getAttribute('aria-busy'), 'true');
      assert.deepEqual(calls, [names[kind]]);
      reject(new Error('Deletion refused')); await app.flush();
      assert.ok(app.document.querySelector('[role=alertdialog]'), 'existing retry guard stays intact');
      assert.match(app.document.querySelector('[role=alertdialog] [role=alert]')?.textContent ?? '', /Deletion refused/);
      assert.doesNotMatch(app.document.querySelector('.dlg-note')?.textContent ?? '', /Deletion refused/);
      assert.equal(confirm().disabled, false);
      confirm().click(); await app.flush();
      assert.deepEqual(calls, [names[kind], names[kind]]);
      assert.equal(app.document.querySelector('[role=alertdialog] [role=alert]'), null);
    } finally { await app.close(); }
  });
}

test('a completed deletion releases the editor before a failed refresh and cannot clear a newer deletion (#167)', async context => {
  const reads: ((error: Error) => void)[] = [];
  const writes: string[] = [];
  let initial = true, finishBeta!: () => void;
  const app = await (await compiled).mount(context, {
    props: { visible: true, section: 'agents' },
    modules: [rpc({
      registryList: () => {
        if (initial) { initial = false; return Promise.resolve({ agents: [alpha, { ...alpha, name: 'beta' }] }); }
        return new Promise((_, no) => reads.push(no));
      },
      registryDelete: name => {
        writes.push(name);
        return name === 'alpha' ? Promise.resolve() : new Promise<void>(yes => finishBeta = yes);
      },
    })],
  });
  try {
    await openAgent(app);
    command(app, 'Delete').click(); await app.flush();
    app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!.click();
    for (let i = 0; i < 4; i++) await app.flush();
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
    await openAgent(app, 'beta');
    assert.equal(app.document.querySelector('.mid h1')?.textContent, 'beta', 'catalog refresh is not a pending mutation');
    command(app, 'Delete').click(); await app.flush();
    app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!.click(); await app.flush();
    reads[0]!(new Error('Catalog unavailable'));
    for (let i = 0; i < 6; i++) await app.flush();
    assert.equal(app.document.querySelector('[role=alertdialog]')?.getAttribute('aria-busy'), 'true',
      'old refresh cleanup cannot clear the new mutation busy state');
    assert.deepEqual(writes, ['alpha', 'beta']);
    finishBeta();
    for (let i = 0; i < 4; i++) await app.flush();
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
  } finally { await app.close(); }
});

test('a committed host section change invalidates old delete completion and cleanup (#167)', async context => {
  const fixture = await hostFixture();
  let controls!: { section: (value: string) => void };
  let rejectOld!: (error: Error) => void, rejectNew!: (error: Error) => void;
  const app = await fixture.mount(context, {
    props: { ready: (value: typeof controls) => controls = value },
    modules: [rpc({
      teamsList: async () => ({ teams: [{ name: 'squad', description: '', members: '[{"name":"dev","base":"alpha"}]' }] }),
      registryDelete: () => new Promise((_, no) => rejectOld = no),
      teamsDelete: () => new Promise((_, no) => rejectNew = no),
    })],
  });
  try {
    await openAgent(app);
    command(app, 'Delete').click(); await app.flush();
    app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!.click(); await app.flush();
    controls.section('teams'); await app.flush();
    assert.equal(app.document.querySelector('[role=alertdialog]'), null, 'the host committed a different view');
    await openAgent(app, 'squad');
    command(app, 'Delete').click(); await app.flush();
    app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!.click(); await app.flush();
    rejectOld(new Error('Old rejection')); await app.flush();
    assert.equal(app.document.querySelector('[role=alertdialog]')?.getAttribute('aria-busy'), 'true');
    assert.equal(app.document.querySelector('[role=alertdialog] [role=alert]'), null);
    rejectNew(new Error('Current rejection')); await app.flush();
    assert.match(app.document.querySelector('[role=alertdialog] [role=alert]')?.textContent ?? '', /Current rejection/);
  } finally { await app.close(); }
});
