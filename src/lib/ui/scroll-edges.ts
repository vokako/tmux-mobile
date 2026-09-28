type ScrollBox = Pick<HTMLElement, 'scrollLeft' | 'scrollWidth' | 'clientWidth'>;

/** Physical left/right edges of the app's LTR strips; tolerate fractional scroll rounding. */
export function horizontalEdges({ scrollLeft, scrollWidth, clientWidth }: ScrollBox) {
  const end = Math.max(0, scrollWidth - clientWidth);
  return {
    before: clientWidth > 0 && end > 1 && scrollLeft > 1,
    after: clientWidth > 0 && end > 1 && scrollLeft < end - 1,
  };
}

type RowBox = Pick<HTMLElement, 'scrollWidth' | 'clientWidth'> & { classList: Pick<DOMTokenList, 'contains' | 'add' | 'remove'> };

/**
 * Would the strip's SINGLE ROW overflow? Measured in the single-row form even
 * while the strip is wrapped: the `wrapped` class comes off, the row is read
 * and the class goes back in one synchronous block, so no frame ever paints
 * the probe (#266: a wrapped list that fits on one row has nothing to expand).
 */
export function rowOverflows(node: RowBox, wrapped = '') {
  const on = !!wrapped && node.classList.contains(wrapped);
  if (on) node.classList.remove(wrapped);
  const over = node.clientWidth > 0 && node.scrollWidth - node.clientWidth > 1;
  if (on) node.classList.add(wrapped);
  return over;
}

export interface EdgeOptions {
  /** Paint the edge cues: the strip is in its single-row, scrolling form. */
  active?: boolean;
  /** The class that puts the strip in its wrapped form (lifted for the probe). */
  wrapped?: string;
  /** Told on every observation whether the single row overflows. */
  onoverflow?: (overflows: boolean) => void;
  /** Any value that changes with the strip's content: re-measure on change. */
  key?: unknown;
}

/** Observe the scrollport AND its items: fonts, labels and live membership can change overflow. */
export function scrollEdges(node: HTMLElement, options: EdgeOptions = {}) {
  let { active = true, wrapped = '', onoverflow } = options;
  const paint = () => {
    const edges = active ? horizontalEdges(node) : { before: false, after: false };
    node.classList.toggle('edge-before', edges.before);
    node.classList.toggle('edge-after', edges.after);
    onoverflow?.(rowOverflows(node, wrapped));
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
    update(next: EdgeOptions = {}) { ({ active = true, wrapped = '', onoverflow } = next); paint(); },
    destroy() {
      resize.disconnect();
      children.disconnect();
      node.removeEventListener('scroll', paint);
      node.classList.remove('edge-before', 'edge-after');
    },
  };
}
