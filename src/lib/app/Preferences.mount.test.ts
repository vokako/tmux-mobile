import assert from 'node:assert/strict';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { join } from 'node:path';
import { compileMount } from '../test/mount.ts';

// A reactive host lets external requests change without remounting Preferences.
// The temporary source uses the existing client compiler and is removed after it.
const compiled = (async () => {
  const temp = fileURLToPath(new URL('../../../temp/', import.meta.url));
  await mkdir(temp, { recursive: true });
  const dir = await mkdtemp(join(temp, 'preferences-mount-'));
  const entry = join(dir, 'Host.svelte');
  try {
    await writeFile(entry, `<script>
      import Preferences from ${JSON.stringify(fileURLToPath(new URL('./Preferences.svelte', import.meta.url)))};
      let { initial, register } = $props();
      let props = $state({ ...initial });
      register(next => Object.assign(props, next));
    </script>
    <Preferences {...props} />`);
    return await compileMount(pathToFileURL(entry), [
      new URL('../core/ws.ts', import.meta.url),
      new URL('../hub/AgentsPage.svelte', import.meta.url),
    ]);
  } finally { await rm(dir, { recursive: true, force: true }); }
})();

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

type AgentProps = {
  section: string;
  editRequest: { name: string; n: number } | null;
  onGuardExit?: (guard: (action: () => void) => void) => (() => void) | void;
  onGoBack: (back: () => boolean) => void;
  onDrilled: (drilled: boolean) => void;
};

async function mount(context: TestContext, options: {
  props?: Record<string, unknown>;
  tab?: string;
  rpc?: Record<string, (...args: any[]) => unknown>;
  agent?: (props: AgentProps) => void;
  setup?: (window: any) => void;
} = {}) {
  let update!: (props: Record<string, unknown>) => void;
  const app = await (await compiled).mount(context, {
    props: {
      initial: options.props ?? {},
      register: (fn: typeof update) => { update = fn; },
    },
    modules: [
      { agentHooksStatus: () => ({}), ...options.rpc },
      // This stub supplies only the agreed child navigation contract; it does
      // not simulate Agent draft logic, which the parent tests independently.
      { default: (_anchor: unknown, props: AgentProps) => { options.agent?.(props); } },
    ],
    setup(window) {
      if (options.tab) window.localStorage.setItem('tmux_settings_tab', options.tab);
      window.localStorage.setItem('tmux_locale', 'en');
      window.HTMLCanvasElement.prototype.getContext = () => null;
      options.setup?.(window);
    },
  });
  const button = (label: string) => {
    const found = [...app.document.querySelectorAll<HTMLButtonElement>('button')]
      .find(el => el.getAttribute('aria-label') === label || el.textContent?.trim() === label);
    assert.ok(found, `button: ${label}`);
    return found;
  };
  return {
    ...app, button,
    async click(label: string) { button(label).click(); await app.flush(); },
    async update(props: Record<string, unknown>) { update(props); await app.flush(); },
    async wait(predicate: () => boolean) {
      for (let i = 0; i < 12 && !predicate(); i++) await app.flush();
      assert.ok(predicate(), 'expected component state settled');
    },
  };
}

test('hook commands block same-tick repeats, preserve status on failure and allow retry', async context => {
  const install = deferred<object>();
  let calls = 0;
  const app = await mount(context, {
    tab: 'connection', props: { connected: true, serverInfo: { hostname: 'A', machineId: 'a' } },
    rpc: { agentHooksInstall: () => ++calls === 1 ? install.promise
      : { claude: { installed: true }, codex: { installed: true }, kiro: { installed: true } } },
  });
  try {
    await app.wait(() => !!app.document.querySelector('.hook-backends'));
    const button = app.button('Install');
    button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    assert.equal(calls, 1, 'the handler guards before Svelte updates disabled');
    await app.flush();
    assert.equal(button.getAttribute('aria-busy'), 'true');
    install.reject(new Error('permission denied'));
    await app.wait(() => app.document.body.textContent!.includes('permission denied'));
    assert.ok(app.document.querySelector('.hook-backends'), 'confirmed status survives');
    assert.equal(app.button('Install').disabled, false);
    await app.click('Install');
    await app.wait(() => app.document.body.textContent!.includes('Remove'));
    assert.equal(calls, 2, 'retry is a new request after failure, not a duplicate');
  } finally { install.resolve({}); await app.close(); }
});

test('late hook replies belong to the captured connection', async context => {
  const old = deferred<object>();
  let calls = 0;
  const app = await mount(context, {
    tab: 'connection', props: { connected: true, serverInfo: { hostname: 'A', machineId: 'a' } },
    rpc: { agentHooksStatus: () => ++calls === 1 ? old.promise : { claude: { installed: true }, codex: { installed: true }, kiro: { installed: true } } },
  });
  try {
    await app.update({ serverInfo: { hostname: 'B', machineId: 'b' } });
    await app.wait(() => calls === 2);
    await app.wait(() => app.document.body.textContent!.includes('Remove'));
    old.resolve({});
    await app.flush();
    assert.ok(app.button('Remove'), 'the old status cannot replace B');
  } finally { old.resolve({}); await app.close(); }
});

test('connection commands expose pending/error and reject duplicate activation', async context => {
  const share = deferred<void>();
  let calls = 0;
  const app = await mount(context, {
    tab: 'connection',
    props: { connected: true, onShare: () => ++calls === 1 ? share.promise : Promise.resolve() },
  });
  try {
    const button = app.button('Share connection link');
    button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    assert.equal(calls, 1);
    await app.flush();
    assert.equal(button.getAttribute('aria-busy'), 'true');
    share.reject(new app.window.Error('Copy failed'));
    await app.wait(() => app.document.querySelector('[role="alert"]')?.textContent === 'Copy failed');
    assert.equal(button.disabled, false);
    await app.advance(3000);
    assert.equal(app.document.querySelector('[role="alert"]')?.textContent, 'Copy failed', 'copy errors do not expire');
    await app.click('Share connection link');
    await app.wait(() => !button.disabled);
    assert.equal(calls, 2, 'the same share command retries after failure');
    assert.equal(app.document.querySelector('[role="alert"]'), null);
  } finally { share.resolve(); await app.close(); }
});

test('notification switch remains immediate while permission/test requests are serialized', async context => {
  const permission = deferred<string>();
  let prompts = 0, sent = 0;
  const app = await mount(context, {
    tab: 'notifications',
    setup(window) {
      window.localStorage.setItem('tmux_notify', 'off');
      window.Audio = class { play() { return Promise.resolve(); } };
      window.Notification = class {
        static permission = 'default';
        static requestPermission() { prompts++; return permission.promise; }
        constructor() { sent++; }
      };
    },
  });
  try {
    const toggle = app.document.querySelector<HTMLButtonElement>('[role="switch"]');
    assert.ok(toggle, 'the boolean uses the shared Switch');
    assert.equal(prompts, 0, 'mount never prompts');
    toggle.click();
    await app.flush();
    assert.equal(app.window.localStorage.getItem('tmux_notify'), 'on');
    assert.equal(prompts, 1);
    app.button('Send a test notification').dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    assert.equal(prompts, 1, 'test cannot race a pending permission request');
    app.window.Notification.permission = 'granted';
    permission.resolve('granted');
    await app.flush();
    await app.click('Send a test notification');
    await app.wait(() => sent === 1);
  } finally { permission.resolve('granted'); await app.close(); }
});

test('font validation locks its row and restores the confirmed family on failure', async context => {
  const probe = deferred<void>();
  let calls = 0;
  const app = await mount(context, {
    setup(window) {
      window.localStorage.setItem('tmux_font_ui', 'Inter');
      window.FontFace = class { load() { calls++; return probe.promise; } };
    },
  });
  try {
    const input = app.document.querySelector<HTMLInputElement>('.sel-combo input')!;
    input.value = 'Unavailable Face';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    input.dispatchEvent(new app.window.FocusEvent('blur'));
    await app.flush();
    assert.equal(calls, 1);
    assert.equal(input.disabled, true);
    assert.equal(app.window.localStorage.getItem('tmux_font_ui'), 'Inter');
    probe.reject(new Error('not installed'));
    await app.wait(() => input.disabled === false);
    assert.equal(input.value, 'Inter');
    assert.ok(app.document.querySelector('[role="alert"].font-error'));
    input.value = '';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    input.dispatchEvent(new app.window.FocusEvent('blur'));
    await app.wait(() => app.window.localStorage.getItem('tmux_font_ui') === null);
    assert.equal(input.value, '');
    assert.equal(app.document.querySelector('[role="alert"].font-error'), null);
  } finally { probe.resolve(); await app.close(); }
});

test('numeric preferences use shared bounds and apply without a page Save', async context => {
  const sizes: number[] = [];
  const app = await mount(context, {
    tab: 'terminal', props: { fontSize: 6, onFontSize: (size: number) => { sizes.push(size); } },
  });
  try {
    const stepper = app.document.querySelector('[role="group"][aria-label="Font"]')!;
    const minus = stepper.querySelector<HTMLButtonElement>('button:first-child')!;
    const plus = stepper.querySelector<HTMLButtonElement>('button:last-child')!;
    assert.equal(minus.disabled, true);
    plus.click();
    assert.deepEqual(sizes, [7]);
    await app.update({ fontSize: 40 });
    assert.equal(plus.disabled, true);
    const slider = app.document.querySelector<HTMLInputElement>('input[type="range"]')!;
    slider.value = '1.25';
    slider.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush();
    assert.equal(app.window.localStorage.getItem('tmux_line_height'), '1.25');
    app.document.querySelector<HTMLButtonElement>('.control-slider button')!.click();
    await app.flush();
    assert.equal(app.window.localStorage.getItem('tmux_line_height'), '1');
    assert.equal(app.document.querySelector('[aria-label="Save"]'), null);
  } finally { await app.close(); }
});

test('the native shortcut recorder keeps capture, conflict, clear, cancel and reset', async context => {
  const app = await mount(context, { tab: 'shortcuts', props: { showShortcuts: true } });
  try {
    const recorder = app.document.querySelector<HTMLButtonElement>('[data-shortcut-recorder]')!;
    const key = async (key: string, code = key, metaKey = false) => {
      recorder.dispatchEvent(new app.window.KeyboardEvent('keydown', { key, code, metaKey, bubbles: true, cancelable: true }));
      await app.flush();
    };
    recorder.click();
    await key('i', 'KeyI', true);
    assert.ok(app.document.querySelector('[role="alert"]'), 'another action owns Meta+I');
    await key('Escape');
    assert.equal(app.document.querySelector('[role="alert"]'), null);
    recorder.click();
    await key('Backspace');
    assert.equal(JSON.parse(app.window.localStorage.getItem('tmux_shortcuts')!).previousPage, '');
    recorder.click();
    await key('x', 'KeyX', true);
    assert.equal(JSON.parse(app.window.localStorage.getItem('tmux_shortcuts')!).previousPage, 'Meta+KeyX');
    await app.click('Restore defaults');
    assert.equal(JSON.parse(app.window.localStorage.getItem('tmux_shortcuts')!).previousPage, 'Meta+KeyU');
    assert.equal(recorder.classList.contains('recording'), false);
  } finally { await app.close(); }
});

test('category exits use the embedded guard, with Back still delegated first', async context => {
  let apply: (() => void) | undefined;
  let registered: AgentProps | undefined;
  let back!: () => boolean;
  let backs = 0;
  const app = await mount(context, {
    tab: 'agents', props: { showAgents: true, onGoBack: (fn: () => boolean) => { back = fn; } },
    agent(props) {
      registered = props;
      props.onGuardExit?.(action => { apply = action; });
      props.onGoBack(() => { backs++; return true; });
    },
  });
  try {
    await app.click('Appearance');
    assert.equal(app.window.localStorage.getItem('tmux_settings_tab'), 'agents');
    assert.equal(app.document.querySelector('.preferences')?.classList.contains('config-compact'), false,
      '#162: a pending category exit cannot apply compact metrics to the still-mounted Agent editor');
    assert.ok(apply, 'Preferences sends the intent to the child');
    assert.equal(back(), true);
    assert.equal(backs, 1);
    assert.equal(registered!.section, 'agents');
    apply();
    await app.flush();
    assert.equal(app.window.localStorage.getItem('tmux_settings_tab'), 'appearance');
    assert.equal(app.document.querySelector('.preferences')?.classList.contains('config-compact'), true,
      '#162: the accepted Settings category opts in without remounting its host');
  } finally { await app.close(); }
});

test('a pending child may replay an external exit later without another request', async context => {
  let replay!: () => void;
  let child!: AgentProps;
  let calls = 0;
  const app = await mount(context, {
    tab: 'teams', props: { showAgents: true },
    agent(props) {
      child = props;
      props.onGuardExit?.(action => { calls++; replay = action; });
    },
  });
  try {
    await app.update({ openRequest: { tab: 'agents', n: 1 }, agentsEditRequest: { name: 'alice', n: 7 } });
    await app.flush();
    assert.equal(calls, 1, 'host does not poll the guard');
    assert.equal(child.section, 'teams');
    replay();
    await app.flush();
    assert.equal(child.section, 'agents');
    assert.equal(child.editRequest?.n, 7);
    await app.update({ openRequest: { tab: 'agents', n: 1 } });
    assert.equal(calls, 1, 'only an applied request is consumed');
  } finally { await app.close(); }
});

test('external category requests and Agent edits wait for approval; old exits cannot win', async context => {
  const exits: Array<() => void> = [];
  let child!: AgentProps;
  const edit = () => child.editRequest;
  const app = await mount(context, {
    tab: 'teams', props: { showAgents: true },
    agent(props) { child = props; props.onGuardExit?.(action => { exits.push(action); }); },
  });
  try {
    await app.update({ openRequest: { tab: 'agents', n: 1 }, agentsEditRequest: { name: 'alice', n: 1 } });
    assert.equal(child.section, 'teams');
    assert.equal(edit(), null, 'the Agent request must not leak into Teams');
    assert.equal(exits.length, 1);
    // A rejected request was not consumed; presenting that same request again
    // must still offer the guard. A later category click invalidates old actions.
    await app.update({ openRequest: { tab: 'agents', n: 1 } });
    assert.equal(exits.length, 2);
    await app.click('Skills');
    assert.equal(exits.length, 3);
    exits[0]!();
    await app.flush();
    assert.equal(child.section, 'teams');
    exits[2]!();
    await app.flush();
    assert.equal(child.section, 'skills');
    await app.update({ openRequest: { tab: 'agents', n: 1 } });
    assert.equal(exits.length, 4, 'consumption waits until the jump applies');
    exits[3]!();
    await app.flush();
    assert.equal(child.section, 'agents');
    assert.equal(edit()?.name, 'alice');
  } finally { await app.close(); }
});
