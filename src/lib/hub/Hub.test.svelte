<script lang="ts">
  import { untrack } from 'svelte';
  import Hub from './Hub.svelte';
  import { setServedBackends, type BackendInfo } from '../core/agents.ts';
  import { centre } from './notify-centre.svelte.ts';
  // The notification centre is App-owned UI; a test drives its jump requests here (#322).
  (globalThis as unknown as { __centre: typeof centre }).__centre = centre;
  // What App.svelte does on connect: the server's backends_list, before the
  // Hub reads it (board #271 — the live queue/steer switch is server truth).
  let { backends = null, ...rest }: { backends?: BackendInfo[] | null; [key: string]: unknown } = $props();
  untrack(() => setServedBackends(backends));
</script>

<Hub {...rest} />
