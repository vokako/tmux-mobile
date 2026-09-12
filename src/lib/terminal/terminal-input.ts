// The terminal's INPUT side, as testable logic: the keystroke send queue and
// the paste fallback. Terminal.svelte owns the DOM, xterm, resumeLiveTail and
// the failure feedback; this module owns ordering, merging, one-in-flight,
// drop-on-failure and — the reason it exists (board #190) — WHICH PANE each
// piece of input is for. Every queued item carries the pane it was typed
// into, captured at enqueue time, so nothing here ever reads a live target
// across an await.

export type SendKeys = (target: string, keys: string, literal: boolean) => Promise<unknown>;

interface QueueDeps {
  send: SendKeys;
  onSuccess: () => void;
  onFailure: () => void;
  dbg?: (message: string) => void;
  /** Saturation cap (long-press repeat on a dead-slow link); default 64. */
  max?: number;
}

interface Item { target: string; keys: string; literal: boolean }

/** One send_keys in flight at a time; keys pressed meanwhile queue up, and
 * consecutive LITERAL chars for the SAME pane merge into one string (tmux
 * send-keys -l applies it as one write). Special keys never merge but still
 * serialize, so their order against typed chars holds. A failure drops
 * everything queued behind it — replaying seconds-old keystrokes after a
 * reconnect is worse than losing them. `reset()` is the pane switch: queued
 * keys belonged to the previous pane. */
export function createKeyQueue(deps: QueueDeps) {
  const max = deps.max ?? 64;
  let items: Item[] = [];
  let sending = false;

  async function pump() {
    if (sending) return;
    sending = true;
    while (items.length > 0) {
      const item = items.shift()!;
      try {
        await deps.send(item.target, item.keys, item.literal);
        deps.onSuccess();
      } catch (e) {
        deps.dbg?.(`input: sendKeys FAILED: ${(e as Error)?.message ?? e}`);
        deps.onFailure();
        items = [];
      }
    }
    sending = false;
  }

  return {
    enqueue(target: string, keys: string, literal: boolean): void {
      const last = items[items.length - 1];
      if (literal && last?.literal && last.target === target) {
        last.keys += keys;
      } else if (items.length >= max) {
        // Saturated. Drop the newest — dropping anything earlier would
        // reorder the user's input.
        deps.dbg?.('input: key queue full — dropping key');
        return;
      } else {
        items.push({ target, keys, literal });
      }
      void pump();
    },
    reset(): void { items = []; },
    get length(): number { return items.length; },
    get sending(): boolean { return sending; },
  };
}

interface PasteDeps {
  paste: (target: string, data: string) => Promise<unknown>;
  enqueue: (target: string, keys: string, literal: boolean) => void;
  onSuccess: () => void;
  onFailure: (kind: 'paste') => void;
}

/** Paste through tmux's paste buffer; a pre-paste_text server (-32601,
 * method not found) gets the old keystroke path instead of a dropped paste.
 * The pane is the one the paste was made into — captured HERE, because the
 * answer arrives after a round trip and the user may be looking at another
 * pane by then (board #190). */
export async function pasteOrFallback(pane: string, data: string, deps: PasteDeps): Promise<void> {
  try {
    await deps.paste(pane, data);
    deps.onSuccess();
  } catch (e) {
    if ((e as { code?: number })?.code === -32601) { deps.enqueue(pane, data, true); return; }
    deps.onFailure('paste');
  }
}
