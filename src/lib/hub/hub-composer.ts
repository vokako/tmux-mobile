import type { AnchorRect } from '../ui/placement.ts';

export const ALL_TARGET = 'all';

/** Boxes are measured by the browser in textarea-local CSS pixels. A tall
 * touch target can hit the preceding line even when the final line is short. */
export function signatureLayout({
  width, naturalHeight, maxHeight, controlsWidth, controlsHeight, gap, textRects, empty = false,
}: {
  width: number; naturalHeight: number; maxHeight: number;
  controlsWidth: number; controlsHeight: number; gap: number;
  textRects: readonly AnchorRect[]; empty?: boolean;
}): { inputHeight: number; reserved: number; overflow: boolean; collision: boolean } {
  if (empty) return { inputHeight: controlsHeight, reserved: 0, overflow: false, collision: false };
  const natural = Math.max(controlsHeight, naturalHeight);
  const limit = Math.max(controlsHeight, maxHeight);
  const left = width - controlsWidth;
  const top = natural - controlsHeight;
  const collision = textRects.some(rect => rect.right > rect.left && rect.bottom > rect.top
    && rect.right + gap > left && rect.left < width && rect.bottom > top && rect.top < natural);
  const reserved = collision || natural > limit + 1 ? controlsHeight : 0;
  const inputHeight = Math.min(natural, Math.max(controlsHeight, limit - reserved));
  return { inputHeight, reserved, overflow: natural > inputHeight + 1, collision };
}

interface InterruptAgent {
  name: string;
  managed: boolean;
  state?: string;
}

const INTERRUPTIBLE_STATES = new Set(['running', 'working', 'waiting', 'blocked']);

/** A card owns only its busy managed members; callers capture before awaiting. */
export function busyTargetsFor(target: string, agents: readonly InterruptAgent[]): string[] {
  if (!target) return [];
  return [...new Set(agents
    .filter((a) => a.managed && (target === ALL_TARGET || a.name === target)
      && INTERRUPTIBLE_STATES.has(a.state ?? ''))
    .map((a) => a.name))];
}

interface PaletteAgent {
  name: string;
  managed: boolean;
  agent?: string | null;
}

interface AttachmentToken {
  kind: 'image' | 'file';
  n: number;
}

interface StagedAttachment extends AttachmentToken {
  path: string;
}

/** A leading addressee wins; a room/all choice needs one managed dialect. */
export function paletteBackendFor(
  text: string | null | undefined,
  recipient: string,
  agents: readonly PaletteAgent[],
): string {
  const m = /^\s*@([\w][\w.-]*)\s/u.exec(text ?? '');
  const name = m ? m[1] : (recipient === ALL_TARGET ? null : recipient);
  if (name) return agents.find((a) => a.managed && a.name === name)?.agent ?? '';
  const backends = [...new Set(agents.filter((a) => a.managed).map((a) => a.agent ?? ''))];
  return backends.length === 1 ? backends[0]! : 'mixed';
}

export const attachToken = (a: AttachmentToken): string =>
  `[${a.kind === 'image' ? 'img' : 'file'}:${a.n}]`;

/** Replace each attachment's first token in place; append a deleted token's ref. */
export function attachmentBody(raw: string, attachments: readonly StagedAttachment[]): string {
  let body = raw;
  const stragglers = [];
  for (const a of attachments) {
    const ref = a.kind === 'image' ? `![](${a.path})` : a.path;
    const tok = attachToken(a);
    if (body.includes(tok)) body = body.replace(tok, ref);
    else stragglers.push(ref);
  }
  return [body, ...stragglers].filter(Boolean).join('\n');
}
