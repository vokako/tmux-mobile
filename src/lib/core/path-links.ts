/** A file reference, not an external URL or an in-document fragment. */
export function pathRef(href: string | null | undefined): string {
  const raw = href?.trim() ?? '';
  if (!raw || raw.startsWith('#') || raw.startsWith('//')) return '';
  const path = (raw.split('#')[0] ?? '').replace(/:\d+(?::\d+)?$/u, '');
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/u.test(path)) return '';
  try { return decodeURIComponent(path); } catch { return path; }
}

/** Resolve filesystem segments, not against the browser's HTTP origin. */
export function resolvePathRef(base: string, ref: string): string {
  if (ref.startsWith('/') || ref.startsWith('~')) return ref;
  const parts: string[] = [];
  for (const part of `${base}/${ref}`.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') parts.pop(); else parts.push(part);
  }
  return '/' + parts.join('/');
}

export function handlePathLinkClick(event: MouseEvent, open: (path: string) => void): boolean {
  if (event.defaultPrevented || (event.type === 'auxclick' && event.button !== 1)) return false;
  const anchor = (event.target as Element | null)?.closest?.('a[href]');
  const path = pathRef(anchor?.getAttribute('href'));
  if (!path) return false;
  event.preventDefault();
  event.stopPropagation();
  open(path);
  return true;
}

/** iframe events stay in their document; use the same path policy there. */
export function installPathLinkHandler(root: Document | HTMLElement | null, open: (path: string) => void): () => void {
  if (!root) return () => {};
  const activate = (event: Event) => { handlePathLinkClick(event as MouseEvent, open); };
  root.addEventListener('click', activate, true);
  root.addEventListener('auxclick', activate, true);
  return () => {
    root.removeEventListener('click', activate, true);
    root.removeEventListener('auxclick', activate, true);
  };
}
