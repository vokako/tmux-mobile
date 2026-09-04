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

export type SelectionClickKey = string | number;

export interface SelectionClickGuard {
  /** Mark a touch/pen-owned contextmenu without suppressing native selection. */
  mark(event: ContextMenuLike, key: SelectionClickKey, now?: number): boolean;
  /** Consume only the immediately following click for that same selectable item. */
  consume(key: SelectionClickKey, now?: number): boolean;
}

/** Android/WebKit may emit a compatibility click after native long-press text
 * selection. By click time the DOM selection can be temporarily collapsed, so
 * an `isCollapsed` check alone may open the app action row. Remember the item
 * whose touch-owned contextmenu just fired and consume one matching click.
 *
 * This guard never calls preventDefault: the contextmenu remains the system's
 * selection gesture. Any click — matching or not — spends the mark, and the
 * short expiry prevents a missing compatibility click from eating a later tap. */
export function selectionClickGuard(graceMs = 800): SelectionClickGuard {
  let markedKey: SelectionClickKey | null = null;
  let expiresAt = 0;

  return {
    mark(event, key, now = Date.now()) {
      if (!systemOwnsContextMenu(event)) return false;
      markedKey = key;
      expiresAt = now + graceMs;
      return true;
    },
    consume(key, now = Date.now()) {
      const suppress = markedKey === key && now <= expiresAt;
      markedKey = null;
      expiresAt = 0;
      return suppress;
    },
  };
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
