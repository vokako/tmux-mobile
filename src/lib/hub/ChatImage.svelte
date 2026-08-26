<script>
  // An image a chat message REFERENCED. The message carries a path or a URL,
  // never bytes — the room is a log, so resolving the reference is the client's
  // job and happens here, once per src:
  //   http(s)/data/blob → straight into <img>.
  //   anything else     → a path on the server's machine, fetched through the
  //                       same signed /dl endpoint the file browser uses, so a
  //                       50 MB screenshot streams instead of arriving base64'd
  //                       through the RPC channel.
  // The signature on that URL expires (60 s), but it only has to survive the
  // fetch the browser does immediately; a failed load falls back to showing the
  // reference itself, which is still information ("it sent /tmp/x.png").
  import { isDirectUrl } from './hub.ts';
  import { fsDownloadHttp } from '../core/ws.ts';

  let { src = '', alt = '', onview = null } = $props();

  let url = $state('');
  let failed = $state(false);

  $effect(() => {
    const ref = src;
    failed = false;
    if (!ref) { url = ''; return; }
    if (isDirectUrl(ref)) { url = ref; return; }
    url = '';
    let live = true;
    fsDownloadHttp(ref)
      .then((info) => { if (live) url = info.url; })
      .catch(() => { if (live) failed = true; });
    return () => { live = false; };
  });
</script>

{#if url && !failed}
  <!-- Tap opens the in-app viewer when the host provides one (owner,
       2026-08-26: "看图片的支持" — a new browser tab is not viewing);
       middle-click/long-press keep the plain link behaviors. -->
  <a class="ci-link" href={url} target="_blank" rel="noopener"
    onclick={(e) => { if (onview) { e.preventDefault(); onview(url); } }}>
    <img class="ci" {alt} src={url} loading="lazy" onerror={() => failed = true} />
  </a>
{:else}
  <!-- Unresolvable: name what was referenced instead of showing nothing. -->
  <span class="ci-ref" class:failed>{src}</span>
{/if}

<style>
  .ci-link { display: block; }
  .ci {
    display: block; max-width: 100%; max-height: calc(42vh / var(--ui-zoom, 1)); width: auto; height: auto;
    border-radius: var(--ui-radius-control); border: 1px solid var(--border2); background: var(--surface2);
  }
  .ci-ref {
    display: inline-block; font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub);
    color: var(--text3); border: 1px dashed var(--border); border-radius: 7px; padding: 4px 8px;
    max-width: 100%; overflow-wrap: anywhere;
  }
  .ci-ref.failed { color: var(--status-warn); border-color: var(--status-warn); }
</style>
