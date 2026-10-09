<script>
  // Host for the server-context mount test (board #335 ②a-3). It stands in
  // for what ②b's App will do: one provider per server-bound subtree, keyed
  // by the runtime HANDLE, plus one subtree with no provider at all — the
  // path every page takes today.
  import ServerContextProvider from './ServerContext.test.provider.svelte';
  import ServerContextConsumer from './ServerContext.test.consumer.svelte';

  let runtimes = $state([]);
  let bare = $state(true);
  globalThis.__ctx = {
    set runtimes(v) { runtimes = v; },
    get runtimes() { return runtimes; },
    set bare(v) { bare = v; },
  };
</script>

{#each runtimes as runtime (runtime.id)}
  <!-- Keyed by the HANDLE, not by the id: a replacement runtime for the same
       server must unmount the old owner before the new one mounts, while an
       ordinary reconnect (same object) must not remount anything. -->
  {#key runtime}
    <ServerContextProvider {runtime}>
      <ServerContextConsumer label={runtime.id} />
    </ServerContextProvider>
  {/key}
{/each}

{#if bare}
  <ServerContextConsumer label="no-provider" />
{/if}
