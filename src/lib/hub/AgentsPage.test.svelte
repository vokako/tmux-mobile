<script lang="ts">
  import AgentsPage from './AgentsPage.svelte';
  let { ready }: { ready: (controls: { show: (value: boolean) => void; edit: (name: string) => void }) => void } = $props();
  let visible = $state(true);
  let editRequest = $state<{ name: string; n: number } | null>(null);
  let sequence = 0;
  $effect(() => ready({
    show: value => visible = value,
    edit: name => editRequest = { name, n: ++sequence },
  }));
</script>

<div hidden={!visible}>
  <AgentsPage {visible} {editRequest} section="agents" />
</div>
