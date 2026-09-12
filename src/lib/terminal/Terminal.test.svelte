<script>
  import Terminal from './Terminal.svelte';
  let { register = () => {} } = $props();
  let target = $state('feedback:0.0');
  let session = $state('feedback');
  let visible = $state(true);
  let present = $state(true);
  $effect(() => register({
    target: value => { target = value; },
    session: value => { session = value; },
    visible: value => { visible = value; },
    present: value => { present = value; },
  }));
</script>

<main class:hidden={!visible}>
  {#if present}
    <Terminal {target} {session} {visible} command="zsh" onOpenSessions={() => {}} />
  {/if}
</main>

<style>
  main { height: 100dvh; display: flex; }
  main.hidden { visibility: hidden; }
</style>
