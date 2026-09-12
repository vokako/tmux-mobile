<script>
  import ContextMenu from './ContextMenu.svelte';
  let at = $state(null);
</script>

<button onclick={(event) => at = { x: 10, y: 10, trigger: event.currentTarget }}>Open</button>
<ContextMenu {at} who="File" items={[{ label: 'Copy', onselect() {} }]} oncancel={() => at = null} />
<input aria-label="Next field" />
