<script>
  import ContextMenu from './ContextMenu.svelte';
  let { rect = false } = $props();
  let at = $state(null);
</script>

<section class="source-scroll">
  <button onclick={(event) => at = rect
    ? { anchor: { left: 10, right: 30, top: 10, bottom: 30 }, trigger: event.currentTarget }
    : { x: 10, y: 10, trigger: event.currentTarget }}>Open</button>
</section>
<section class="sibling-scroll"></section>
<ContextMenu {at} who="File" items={[{ label: 'Copy', onselect() {} }]} oncancel={() => at = null} />
<input aria-label="Next field" />
