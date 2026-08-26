<script lang="ts">
  // Fullscreen image viewer (owner, 2026-08-26: "看图片的支持"). One fixed
  // overlay above everything — like every fixed layer it assumes the VIEWPORT
  // as containing block (design-language.md §2). Gestures are the ones a photo
  // viewer owes you: pinch to zoom (two pointers), drag to pan once zoomed,
  // double-tap toggles 1x ↔ 2.5x at the tapped point, wheel zooms on desktop.
  // Dismissal is the standard set: backdrop tap (only when NOT zoomed — a pan
  // that ends on the backdrop must not close it), Escape, the ✕, and the
  // Android back gesture via the host page's onGoBack chain.
  let { src = '', alt = '', onclose = () => {} } = $props<{
    src?: string; alt?: string; onclose?: () => void;
  }>();

  let scale = $state(1);
  let tx = $state(0);
  let ty = $state(0);

  const MAX_SCALE = 6;
  let pointers = new Map<number, { x: number; y: number }>();
  let pinchDist = 0;
  let lastTap = 0;
  let moved = false;

  function clamp() {
    if (scale <= 1) { scale = 1; tx = 0; ty = 0; }
    else if (scale > MAX_SCALE) scale = MAX_SCALE;
  }
  function zoomAt(cx: number, cy: number, factor: number) {
    // Keep the point under the cursor/fingers stationary while scaling.
    const s2 = Math.min(Math.max(scale * factor, 1), MAX_SCALE);
    const k = s2 / scale;
    const ox = cx - innerWidth / 2, oy = cy - innerHeight / 2;
    tx = ox - (ox - tx) * k;
    ty = oy - (oy - ty) * k;
    scale = s2;
    clamp();
  }

  function onPointerDown(e: PointerEvent) {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    moved = false;
    if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      pinchDist = Math.hypot(a!.x - b!.x, a!.y - b!.y);
    }
  }
  function onPointerMove(e: PointerEvent) {
    const prev = pointers.get(e.pointerId);
    if (!prev) return;
    const dx = e.clientX - prev.x, dy = e.clientY - prev.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 2) {
      const [a, b] = [...pointers.values()];
      const d = Math.hypot(a!.x - b!.x, a!.y - b!.y);
      if (pinchDist > 0) zoomAt((a!.x + b!.x) / 2, (a!.y + b!.y) / 2, d / pinchDist);
      pinchDist = d;
    } else if (scale > 1) {
      tx += dx; ty += dy;
    }
  }
  function onPointerUp(e: PointerEvent) {
    pointers.delete(e.pointerId);
    pinchDist = 0;
    if (pointers.size === 0 && !moved) {
      const now = performance.now();
      if (now - lastTap < 300) {
        // Double-tap: zoom into the tapped point, or all the way back out.
        if (scale > 1) { scale = 1; tx = 0; ty = 0; }
        else zoomAt(e.clientX, e.clientY, 2.5);
        lastTap = 0;
      } else lastTap = now;
    }
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    zoomAt(e.clientX, e.clientY, e.deltaY < 0 ? 1.2 : 1 / 1.2);
  }
  function onBackdrop(e: MouseEvent) {
    // Only a clean tap on the backdrop itself, while unzoomed, closes.
    if (e.target === e.currentTarget && scale === 1 && !moved) onclose();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.stopPropagation(); onclose(); }
  }
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_click_events_have_key_events -->
<div class="lb" role="dialog" aria-label={alt || 'image'} tabindex="-1"
  onpointerdown={onPointerDown} onpointermove={onPointerMove}
  onpointerup={onPointerUp} onpointercancel={onPointerUp}
  onclick={onBackdrop} onwheel={onWheel}>
  <img class="lb-img" {src} {alt} draggable="false"
    style:transform={`translate(${tx}px, ${ty}px) scale(${scale})`} />
  <button class="lb-close" aria-label="close" onclick={onclose}>✕</button>
</div>

<style>
  .lb {
    position: fixed; inset: 0; z-index: 300;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px);
    display: flex; align-items: center; justify-content: center;
    touch-action: none; overscroll-behavior: contain;
    /* The APK's real status-bar inset arrives via --sat, never raw env(). */
    padding-top: var(--sat, 0px);
  }
  .lb-img {
    max-width: 100%; max-height: 100%;
    user-select: none; -webkit-user-select: none;
    /* The transform is the viewer's state, not a transition: the finger is
       the animation, exactly like the Files edge-drag. */
    will-change: transform;
    pointer-events: none;
  }
  .lb-close {
    position: absolute; top: calc(8px + var(--sat, 0px)); right: 8px;
    width: 34px; height: 34px; display: flex; align-items: center; justify-content: center;
    border-radius: var(--ui-radius-control); border: 1px solid var(--border2);
    background: var(--surface); color: var(--text2); font-size: var(--fs-body);
    cursor: pointer;
  }
  .lb-close::after { content: ''; position: absolute; inset: -5px; }
  .lb-close:hover { border-color: var(--accent); color: var(--text); }
</style>
