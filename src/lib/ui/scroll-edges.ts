type ScrollBox = Pick<HTMLElement, 'scrollLeft' | 'scrollWidth' | 'clientWidth'>;

/** Physical left/right edges of the app's LTR strips; tolerate fractional scroll rounding. */
export function horizontalEdges({ scrollLeft, scrollWidth, clientWidth }: ScrollBox) {
  const end = Math.max(0, scrollWidth - clientWidth);
  return {
    before: clientWidth > 0 && end > 1 && scrollLeft > 1,
    after: clientWidth > 0 && end > 1 && scrollLeft < end - 1,
  };
}

/** Observe the scrollport AND its items: fonts, labels and live membership can change overflow. */
export function scrollEdges(node: HTMLElement, active = true) {
  const paint = () => {
    const edges = active ? horizontalEdges(node) : { before: false, after: false };
    node.classList.toggle('edge-before', edges.before);
    node.classList.toggle('edge-after', edges.after);
  };
  const view = node.ownerDocument.defaultView!;
  const resize = new view.ResizeObserver(paint);
  const observe = () => {
    resize.disconnect();
    resize.observe(node);
    for (const child of node.children) resize.observe(child);
    paint();
  };
  const children = new view.MutationObserver(observe);
  children.observe(node, { childList: true });
  node.addEventListener('scroll', paint, { passive: true });
  observe();
  return {
    update(next: boolean) { active = next; paint(); },
    destroy() {
      resize.disconnect();
      children.disconnect();
      node.removeEventListener('scroll', paint);
      node.classList.remove('edge-before', 'edge-after');
    },
  };
}
