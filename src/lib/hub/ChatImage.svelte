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
  // The picture fades in once its bytes are there (motion.md: an image that
  // pops from blank to painted is a cut the eye notices). `onload` sets it; a
  // CACHED image can be complete before the handler is wired, so the effect
  // below reads `complete` too. Reset per src.
  let loaded = $state(false);
  let imgEl = $state(null);
  $effect(() => {
    if (imgEl?.complete && imgEl.naturalWidth > 0) loaded = true;
  });

  $effect(() => {
    const ref = src;
    failed = false;
    loaded = false;
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
  {#if onview}
    <!-- With an in-app viewer there is NO link at all: any anchor to a /dl
         URL leaves a path where a tap navigates or downloads (owner,
         2026-08-27: "注意图片查看是在我应用内的，不是说我点击图片去下载了一个
         图片"). The button can only open the Lightbox. -->
    <button class="ci-link" aria-label={alt || 'image'} onclick={() => onview(url)}>
      <img class="ci" class:loaded {alt} src={url} loading="lazy" bind:this={imgEl} onload={() => loaded = true} onerror={() => failed = true} />
    </button>
  {:else}
    <a class="ci-link" href={url} target="_blank" rel="noopener">
      <img class="ci" class:loaded {alt} src={url} loading="lazy" bind:this={imgEl} onload={() => loaded = true} onerror={() => failed = true} />
    </a>
  {/if}
{:else}
  <!-- Unresolvable: name what was referenced instead of showing nothing. -->
  <span class="ci-ref" class:failed>{src}</span>
{/if}

<style>
  .ci-link { display: block; padding: 0; margin: 0; border: 0; background: none; text-align: left; }
  .ci {
    /* A THUMBNAIL by default (owner, 2026-08-27: "消息框里显示的图片，默认使用
       较小尺寸的缩略图，我可以点击放大查看" — the old 42vh cap let one screenshot
       take half the conversation); the Lightbox is where it gets big. */
    display: block; max-width: min(100%, 300px); max-height: 180px; width: auto; height: auto;
    border-radius: var(--ui-radius-control); border: 1px solid var(--border2); background: var(--surface2);
    cursor: zoom-in;
    /* Fades in when loaded — opacity only, the box is already its size. */
    opacity: 0; transition: opacity var(--t-move) ease-out;
  }
  .ci.loaded { opacity: 1; }
  @media (prefers-reduced-motion: reduce) { .ci { transition: none; } }
  .ci-ref {
    display: inline-block; font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub);
    color: var(--text3); border: 1px dashed var(--border); border-radius: 7px; padding: 4px 8px;
    max-width: 100%; overflow-wrap: anywhere;
    transition: color var(--t-fast), border-color var(--t-fast);
  }
  .ci-ref.failed { color: var(--status-warn); border-color: var(--status-warn); }
</style>
