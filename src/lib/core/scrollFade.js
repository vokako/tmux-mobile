// Svelte action: toggle a `.scrolling` class on a scrollable element while
// the user is actively scrolling, removing it after a short idle window.
//
// Pairs with the global `.subtle-scroll` styles (App.svelte) so the
// scrollbar fades in only when the user is interacting and fades back out
// when idle. Works for both mouse-wheel/keyboard scrolls (which already get
// `:hover` / `:focus-within`) and pure touch scrolls (which don't).
//
// Usage:
//   <div class="subtle-scroll" use:scrollFade>...</div>
//
// Idle delay defaults to 700ms — long enough to feel continuous through a
// touch flick, short enough to fade out promptly when the user stops.

const IDLE_MS = 700;

export function scrollFade(node, opts = {}) {
  const idle = typeof opts.idle === 'number' ? opts.idle : IDLE_MS;
  let timer = null;
  function onScroll() {
    node.classList.add('scrolling');
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      node.classList.remove('scrolling');
      timer = null;
    }, idle);
  }
  node.addEventListener('scroll', onScroll, { passive: true });
  return {
    destroy() {
      node.removeEventListener('scroll', onScroll);
      if (timer) clearTimeout(timer);
    },
  };
}
