<script>
  // The one job of a provider: bind its subtree to one runtime at init.
  import { setServerRuntime } from './server-context.ts';
  let { runtime, children } = $props();
  // svelte-ignore state_referenced_locally
  // One runtime for this subtree's whole life — the subtree is keyed by the
  // handle, so a replacement server means a new provider, not a new value.
  setServerRuntime(runtime);
</script>

{@render children?.()}
