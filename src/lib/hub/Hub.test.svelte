<script lang="ts">
  import { untrack } from 'svelte';
  import Hub from './Hub.svelte';
  import { setServedBackends, type BackendInfo } from '../core/agents.ts';
  // What App.svelte does on connect: the server's backends_list, before the
  // Hub reads it (board #271 — the live queue/steer switch is server truth).
  let { backends = null, ...rest }: { backends?: BackendInfo[] | null; [key: string]: unknown } = $props();
  untrack(() => setServedBackends(backends));
</script>

<Hub {...rest} />
