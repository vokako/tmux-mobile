# Testing Conventions

Frontend tests run with `npm test` (`node --test`, no framework, no tmux
needed). Rust tests: `cd src-tauri && cargo test -- --test-threads=1`
(needs a running tmux). This file governs the frontend side.

## Two kinds of tests, two naming schemes

### 1. Unit tests — `<module>.test.js`

One test file per module, colocated, named EXACTLY after the module it
tests (`ws.test.js` tests `ws.ts`; `agent-notifications.test.js` tests
`agent-notifications.svelte.ts`). They import the module and test
behavior. If a module is worth extracting, its invariants are worth
pinning here.

Runes modules (`*.svelte.ts`) are testable in node with the shim:

```js
globalThis.$state = value => value;
```

### 2. Source-contract tests — `<Component>.source.test.js`

Svelte components can't execute under `node --test`, but some component
wiring is too important to leave unpinned (which notification query a
template calls, which navigation states exist, a CSS overflow contract).
These tests `readFile` the component source and assert with regexes.

- Named after the component, colocated (`Terminal.source.test.js` next
  to `Terminal.svelte`; `App.source.test.js` next to `src/App.svelte`).
- One component per file. A cross-component contract gets one file per
  component, each asserting that component's half.
- When one fails after an intentional change, update the assertion —
  the failure means the change must be deliberate, not that it's wrong.
- Prefer upgrading to a real unit test whenever the logic can move into
  an importable module — source regexes are a last resort, and each one
  should explain WHY the invariant matters.

## Rules

- New module → its `<module>.test.js` lands in the same commit.
- Tests may stay `.js` (node runs them without checking); convert to
  `.test.ts` opportunistically when touching one. The glob picks up both.
- No test file without a clear subject; no subject with two test files.
- Every regression fix starts with a failing test that reproduces it.

## Current source-contract inventory

| File | Pins |
|------|------|
| `src/App.source.test.js` | notification refresh on every connect path; terminal nav/page-layer structure |
| `sessions/Sessions.source.test.js` | notification-state import wiring |
| `sessions/PanePicker.source.test.js` | Team-dot suppression in the picker |
| `terminal/Terminal.source.test.js` | Terminal chrome uses only Team-filtered queries |
| `team/Team.source.test.js` | roster chip wrap/overflow CSS contract |
