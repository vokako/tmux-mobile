// The last line of defence for path links (board #99, owner: "即使路径不对，
// 应该友好的提示，不要弹出新的窗口").
//
// A schemeless href — `/local/…/file.rs`, `temp/notes.md` — is a PATH, and a
// browser can only 404 on it (or worse: a webview escapes to a system browser
// window). The surfaces that know their context route these clicks into the
// file preview themselves (Hub bubbles, the Files markdown preview) and call
// preventDefault. This net catches every anchor NOBODY routed — new render
// surfaces, Team chat, a modifier-click — and swallows the navigation, so a
// path link can never take the app away or pop a window, anywhere, ever.
//
// Bubble phase at the window, checked AFTER `defaultPrevented`: a routed
// click passes through untouched; only the unrouted rest is stopped.
import { pathRef } from '../hub/hub.ts';

/** The decision, pure for tests: swallow when nobody routed it and the href
 * is a path. */
export function shouldSwallowPathClick(defaultPrevented: boolean, href: string | null): boolean {
  if (defaultPrevented) return false;
  return pathRef(href) !== '';
}

export function installPathLinkNet(win: Window): () => void {
  const onClick = (e: MouseEvent) => {
    const a = (e.target as Element | null)?.closest?.('a');
    if (!a) return;
    if (shouldSwallowPathClick(e.defaultPrevented, a.getAttribute('href'))) e.preventDefault();
  };
  // `auxclick` too: a middle-click would open the 404 in a NEW window.
  win.addEventListener('click', onClick);
  win.addEventListener('auxclick', onClick);
  return () => { win.removeEventListener('click', onClick); win.removeEventListener('auxclick', onClick); };
}
