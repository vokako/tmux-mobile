<script>
  // Host for the scratch panel mount test (board #324): the frame state lives
  // here like App's, and `epoch` stands in for App's {#key serverEpoch}.
  import ScratchPanel from './ScratchPanel.svelte';
  let open = $state(false);
  let live = $state(true);
  let edge = $state('bottom');
  let epoch = $state(0);
  globalThis.__scratch = {
    set open(v) { open = v; }, get open() { return open; },
    set live(v) { live = v; }, bump() { epoch++; },
    get edge() { return edge; },
  };
</script>

<button class="opener">opener</button>
{#key epoch}
  <ScratchPanel {open} {live} {edge} onclose={() => (open = false)} onedge={(e) => (edge = e)} />
{/key}
