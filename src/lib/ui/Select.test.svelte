<script>
  import Select from './Select.svelte';
  let value = $state('a'), disabled = $state(false), changes = $state(0);
</script>
<Select {value} {disabled} options={['a', 'b']} ariaLabel="Backend"
  onchange={next => { value = next; changes++; }} />
<button type="button" onclick={() => disabled = true}>Lock</button>
<output>{value}/{changes}</output>
