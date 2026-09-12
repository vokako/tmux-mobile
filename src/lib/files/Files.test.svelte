<script>
  import Files from './Files.svelte';
  let { register = () => {}, ...initial } = $props();
  let changes = $state({});
  $effect(() => register(next => { changes = next; }));
</script>

<Files {...initial} {...changes} />
