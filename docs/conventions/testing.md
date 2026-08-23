# Testing Conventions

Frontend tests and repository development-script tests run with `npm test`
(`node --test`, no framework, no tmux needed). Rust tests: `cd src-tauri && cargo test -- --test-threads=1`
(needs a running tmux). This file governs the frontend side.

## Two kinds of tests, two naming schemes

### 1. Unit tests — `<module>.test.ts`

One test file per module, colocated, named EXACTLY after the module it
tests (`ws.test.ts` tests `ws.ts`; `agent-notifications.test.ts` tests
`agent-notifications.svelte.ts`). They import the module and test
behavior. If a module is worth extracting, its invariants are worth
pinning here.

Runes modules (`*.svelte.ts`) are testable in node with the shim:

```js
globalThis.$state = value => value;
```

### 2. Source-contract tests — `<Component>.source.test.ts`

Svelte components can't execute under `node --test`, but some component
wiring is too important to leave unpinned (which notification query a
template calls, which navigation states exist, a CSS overflow contract).
These tests `readFile` the component source and assert with regexes.

- Named after the component, colocated (`Terminal.source.test.ts` next
  to `Terminal.svelte`; `App.source.test.ts` next to `src/App.svelte`).
- One component per file. A cross-component contract gets one file per
  component, each asserting that component's half.
- When one fails after an intentional change, update the assertion —
  the failure means the change must be deliberate, not that it's wrong.
- Prefer upgrading to a real unit test whenever the logic can move into
  an importable module — source regexes are a last resort, and each one
  should explain WHY the invariant matters.

## Rules

- New module → its `<module>.test.ts` lands in the same commit.
- Tests are TypeScript (`.test.ts`) — node executes them natively via
  type stripping, and `npm run check` verifies test code against the
  typed modules it exercises. Deliberately-partial test doubles are cast
  once at the boundary (`as unknown as X`) with a comment; don't build
  full fakes just to satisfy the checker.
- No test file without a clear subject; no subject with two test files.
- Every regression fix starts with a failing test that reproduces it.

## Current source-contract inventory

| File | Pins |
|------|------|
| `src/App.source.test.ts` | notification refresh on every connect path; terminal nav/page-layer structure |
| `sessions/Sessions.source.test.ts` | notification-state import wiring |
| `sessions/PanePicker.source.test.ts` | Team-dot suppression in the picker |
| `terminal/Terminal.source.test.ts` | Terminal chrome uses only Team-filtered queries |
| `team/Team.source.test.ts` | roster chip wrap/overflow CSS contract |
| `ui/tokens.source.test.ts` | one type scale: no raw px font-size outside the listed exceptions |
| `ui/confirm.source.test.ts` | every destructive verb goes through the shared confirmation |
| `hub/Hub.source.test.ts` | a tool-lane row is one line (`nowrap`, never `pre`), the argument is never truncated (the lane pans instead), and the row cap stays expressed in rows |
| `ui/sidebar.source.test.ts` | one sidebar box: a section header takes its padding and type from `.side-h`, and `.side-h`/`.side-row` keep the same 10px inset |
