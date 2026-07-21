type RuntimeOptions = {
  windowRef?: Window | null;
  loadTauriOpener?: () => Promise<{ openUrl: (url: string) => Promise<void> }>;
};

function browserWindow(): Window | null {
  return typeof window === 'undefined' ? null : window;
}

export function isExternalWebUrl(value: unknown): value is string {
  return typeof value === 'string' && /^https?:\/\//i.test(value);
}

export function isTauriWindow(windowRef: Window | null = browserWindow()): boolean {
  const w = windowRef as (Window & { __TAURI__?: unknown; __TAURI_INTERNALS__?: unknown }) | null;
  return !!(w?.__TAURI__ || w?.__TAURI_INTERNALS__);
}

export function externalWebUrlFromAnchor(anchor: HTMLAnchorElement | null | undefined): string | null {
  if (!anchor) return null;

  const rawHref = anchor.getAttribute?.('href');
  if (isExternalWebUrl(rawHref)) return rawHref;

  // `href` is the browser-resolved value. Markdown commonly contains relative
  // or protocol-relative destinations, and checking only the source attribute
  // lets those navigate the embedded WebView.
  return isExternalWebUrl(anchor.href) ? anchor.href : null;
}

export async function openExternalUrl(url: string, {
  windowRef = browserWindow(),
  loadTauriOpener = () => import('@tauri-apps/plugin-opener'),
}: RuntimeOptions = {}) {
  if (!isExternalWebUrl(url)) return false;

  if (isTauriWindow(windowRef)) {
    const { openUrl } = await loadTauriOpener();
    await openUrl(url);
    return true;
  }

  const opened = windowRef?.open(url, '_blank', 'noopener,noreferrer');
  if (opened) opened.opener = null;
  return true;
}

export async function handleExternalLinkClick(event: MouseEvent, runtime?: RuntimeOptions): Promise<boolean> {
  if (event.type === 'auxclick' && event.button !== 1) return false;

  const anchor = (event.target as Element | null)?.closest?.('a[href]') as HTMLAnchorElement | null;
  const url = externalWebUrlFromAnchor(anchor);
  if (!url) return false;

  event.preventDefault();
  await openExternalUrl(url, runtime);
  return true;
}

export function installExternalLinkHandler(root: Document | HTMLElement | null | undefined, runtime?: RuntimeOptions): () => void {
  if (!root?.addEventListener) return () => {};

  const onActivate = (event: Event) => {
    void handleExternalLinkClick(event as MouseEvent, runtime).catch((error) => {
      console.error('Failed to open external link', error);
    });
  };
  root.addEventListener('click', onActivate, true);
  root.addEventListener('auxclick', onActivate, true);
  return () => {
    root.removeEventListener('click', onActivate, true);
    root.removeEventListener('auxclick', onActivate, true);
  };
}
