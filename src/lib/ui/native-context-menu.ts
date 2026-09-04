export interface ContextMenuLike {
  pointerType?: string;
  sourceCapabilities?: { firesTouchEvents?: boolean } | null;
  preventDefault(): void;
}

/** A finger/stylus hold belongs to native text selection. Everything else is
 * desktop/keyboard contextmenu chrome, which the app replaces or suppresses. */
export function systemOwnsContextMenu(event: ContextMenuLike): boolean {
  return event.pointerType === 'touch'
    || event.pointerType === 'pen'
    || event.sourceCapabilities?.firesTouchEvents === true;
}

/** Returns whether browser chrome was suppressed. Pure apart from the event's
 * preventDefault call so the platform split is directly testable. */
export function handleNativeContextMenu(event: ContextMenuLike): boolean {
  if (systemOwnsContextMenu(event)) return false;
  event.preventDefault();
  return true;
}

/** Capture phase means component-owned app menus still open normally while a
 * surface with no app menu never falls through to browser chrome. */
export function installNativeContextMenuGuard(win: Window): () => void {
  const onContextMenu = (event: Event) => {
    handleNativeContextMenu(event as Event & ContextMenuLike);
  };
  win.addEventListener('contextmenu', onContextMenu, { capture: true });
  return () => win.removeEventListener('contextmenu', onContextMenu, true);
}
