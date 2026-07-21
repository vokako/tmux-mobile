function browserWindow() {
  return typeof window === 'undefined' ? null : window;
}

export function isExternalWebUrl(value) {
  return typeof value === 'string' && /^https?:\/\//i.test(value);
}

export function isTauriWindow(windowRef = browserWindow()) {
  return !!(windowRef?.__TAURI__ || windowRef?.__TAURI_INTERNALS__);
}

export function externalWebUrlFromAnchor(anchor) {
  if (!anchor) return null;

  const rawHref = anchor.getAttribute?.('href');
  if (isExternalWebUrl(rawHref)) return rawHref;

  // `href` is the browser-resolved value. Markdown commonly contains relative
  // or protocol-relative destinations, and checking only the source attribute
  // lets those navigate the embedded WebView.
  return isExternalWebUrl(anchor.href) ? anchor.href : null;
}

export async function openExternalUrl(url, {
  windowRef = browserWindow(),
  loadTauriOpener = () => import('@tauri-apps/plugin-opener'),
} = {}) {
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

export async function handleExternalLinkClick(event, runtime) {
  if (event.type === 'auxclick' && event.button !== 1) return false;

  const anchor = event.target?.closest?.('a[href]');
  const url = externalWebUrlFromAnchor(anchor);
  if (!url) return false;

  event.preventDefault();
  await openExternalUrl(url, runtime);
  return true;
}

export function installExternalLinkHandler(root, runtime) {
  if (!root?.addEventListener) return () => {};

  const onActivate = (event) => {
    void handleExternalLinkClick(event, runtime).catch((error) => {
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
