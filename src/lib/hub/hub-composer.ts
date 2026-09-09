export const ALL_TARGET = 'all';

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
