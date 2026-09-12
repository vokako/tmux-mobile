/** An unseated cursor enters from the requested edge, not from index -1 modulo length. */
export function nextMenuIndex(
  current: number,
  direction: 1 | -1,
  count: number,
  enabled: (index: number) => boolean = () => true,
): number {
  if (!count) return -1;
  let next = current >= 0 && current < count ? current : direction === 1 ? -1 : 0;
  for (let visited = 0; visited < count; visited++) {
    next = (next + direction + count) % count;
    if (enabled(next)) return next;
  }
  return -1;
}
