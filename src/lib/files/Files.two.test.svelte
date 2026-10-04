<script>
  // Board #305 (validator probe): the app mounts Files twice (the Files page and
  // the Hub drawer). Both live in ONE realm and share one native part dir.
  import Files from './Files.svelte';
  let { ...initial } = $props();
</script>

<div class="page"><Files {...initial} /></div>
<div class="drawer"><Files {...initial} singlePane /></div>
