/** The last visible modal owns keyboard input, independent of listener order.
 * Display/visibility hide an owner; an entry fade at opacity zero does not. */
export function activeModal(document: Document): HTMLElement | null {
  const modals = [...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')];
  const view = document.defaultView;
  for (const modal of modals.reverse()) {
    let visible = true;
    for (let node: HTMLElement | null = modal; node; node = node.parentElement) {
      const style = view?.getComputedStyle(node);
      if (node.hidden || node.hasAttribute('inert') || node.getAttribute('aria-hidden') === 'true'
        || style?.display === 'none' || style?.visibility === 'hidden') {
        visible = false;
        break;
      }
    }
    if (visible) return modal;
  }
  return null;
}
