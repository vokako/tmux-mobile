import test from 'node:test';
import assert from 'node:assert/strict';
import { restoreViewportAfterPaneSwitch } from './terminal-viewport.ts';

function createRoot() {
  const properties = new Map<string, string>();
  const classes = new Set(['keyboard-open']);
  return {
    properties,
    classes,
    style: { setProperty: (name: string, value: string) => properties.set(name, value) },
    classList: { remove: (name: string) => classes.delete(name) },
  } as unknown as HTMLElement & { properties: Map<string, string>; classes: Set<string> };
}

test('desktop pane switch leaves the current viewport height untouched', () => {
  const root = createRoot();
  root.properties.set('--app-height', '891px');

  restoreViewportAfterPaneSwitch({ isMobile: false, fullHeight: 596, root });

  assert.equal(root.properties.get('--app-height'), '891px');
  assert.equal(root.classes.has('keyboard-open'), true);
});

test('mobile pane switch restores the full height and closes keyboard state', () => {
  const root = createRoot();

  restoreViewportAfterPaneSwitch({ isMobile: true, fullHeight: 891, root });

  assert.equal(root.properties.get('--app-height'), '891px');
  assert.equal(root.classes.has('keyboard-open'), false);
});
