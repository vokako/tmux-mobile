// Source-contract test for the sidebar's shared box (see docs/conventions/testing.md).
//
// Every sidebar in this app is built from two shared classes in app.css:
// `.side-h` for a section header and `.side-row` for a row. They agree on ONE
// left inset — 10px — which is what makes the Chat sidebar and the Terminal
// sidebar read as the same component instead of two lists that happen to look
// similar.
//
// Twice now a component has re-declared part of that box locally and drifted:
// first the type dialect (10.5px/1.4px tracking became 11px/0.66px), then the
// padding (`8px 6px 4px`), which put the "PROJECTS" header 4px left of the rows
// it labels and 4px left of Chat's identical header — owner, 2026-08-20: "chat
// 页面和 terminal 页面这里的 projects 文字的位置不一样，样式好像不统一".
//
// A scoped rule outranks a shared class (0,2,0 vs 0,1,0), so the drift is silent:
// nothing breaks, the two pages just stop matching. Hence this guard.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const SRC = new URL('../../', import.meta.url); // src/

/** The components whose section headers ARE `.side-h` (class:side-h={…}). */
const HEADERS = ['lib/sessions/Sessions.svelte', 'lib/projects/Projects.svelte'];

test('a sidebar header takes its box from .side-h, not from a local override', async () => {
  for (const file of HEADERS) {
    const raw = await readFile(new URL(file, SRC), 'utf8');
    const style = /<style>([\s\S]*)<\/style>/.exec(raw)?.[1] ?? '';
    const css = style.replace(/\/\*[\s\S]*?\*\//g, ''); // comments mention px values
    // EVERY rule that can reach the header in SIDEBAR mode. The third
    // recurrence of this drift was an UNQUALIFIED `.group-label` rule (the
    // first two were `.projects.dense`-scoped), so the test now inverts:
    // only a rule pinned to PAGE mode (`:not(.dense)` / `:not(.sidebar-mode)`)
    // may declare the header's box or type.
    const rules = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
      .map(([, sel, body]) => [sel!.trim(), body!] as const)
      .filter(([sel]) => /\.group-label/.test(sel) && !/:not\(\.(?:dense|sidebar-mode)\)/.test(sel));
    for (const [sel, body] of rules) {
      assert.ok(
        !/(^|;|\s)padding[^:]*:/.test(body),
        `${file}: \`${sel}\` re-declares the header's padding, which .side-h owns in sidebar mode — qualify it to page mode (:not(.dense) / :not(.sidebar-mode))`,
      );
      assert.ok(
        !/font-size\s*:|letter-spacing\s*:|text-transform\s*:/.test(body),
        `${file}: \`${sel}\` re-declares the header's type, which .side-h owns in sidebar mode — qualify it to page mode (:not(.dense) / :not(.sidebar-mode))`,
      );
    }
  }
});

test('app.css still defines the one sidebar inset both classes share', async () => {
  const css = await readFile(new URL('app.css', SRC), 'utf8');
  const head = /\.side-h\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
  const row = /\.side-row\s*\{([^}]*)\}/.exec(css)?.[1] ?? '';
  const insetOf = (body: string) => {
    const p = /padding:\s*([^;]+);/.exec(body)?.[1]?.trim().split(/\s+/) ?? [];
    // `a b c` → left is b; `a b` → left is b; `a` → a.
    return p.length >= 2 ? p[1] : p[0];
  };
  assert.equal(insetOf(head), '10px', '.side-h must keep the shared inset');
  assert.equal(insetOf(row), '10px', '.side-row must keep the shared inset');
  assert.equal(insetOf(head), insetOf(row), 'a header and a row line up, or the sidebar looks broken');
});
