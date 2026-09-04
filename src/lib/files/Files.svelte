<script module>
  // Browse positions parked per SESSION, shared by every Files instance (the
  // Files page and the Hub's drawer both mount this component) and outliving
  // any one instance. In-memory on purpose — a temporary reading position,
  // not a preference; the follow-the-real-cwd rule still outranks it.
  const browsed = new Map(); // session → { cwd, sourceDir }
</script>

<script>
  // Markdown goes through the ONE safe renderer (rule 13: `&`/`<` escaped, so a
  // README's raw <img onerror> is inert text). It also owns marked + KaTeX.
  import { renderMarkdown } from '../core/markdown.ts';
  import { isAndroid, isTauri, tauriReady } from '../core/platform.ts';
  import Icon from '../ui/Icon.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import GitPanel from './GitPanel.svelte';
  import { hoverInfo } from '../ui/hover.ts';
  import { createPersistedList } from './persisted-list.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { layout } from '../app/layout.svelte.ts';
  import { copyText } from '../core/clipboard.ts';
  import { untrack } from 'svelte';
  import { directoryLoadState, leaveDecision, cwdFollowStep } from './file-view-state.ts';
  import { installExternalLinkHandler } from '../core/external-links.ts';
  import { fsCwd, fsList, fsStat, fsRead, fsWrite, fsMkdir, fsDelete, fsRename, fsDownload, fsDownloadHttp, fsUpload, getBookmarks, saveBookmarks, gitCmd, getPrefs, setPref, fsConvert } from '../core/ws.ts';

  // Tauri plugin imports (tree-shaken in browser builds). The platform flags
  // come from the ONE module (rule 3); `tauriPlugins` is this file's own
  // "modules are in" promise, chained on the shared `tauriReady` gate.
  let tauriFs, tauriDialog, tauriOpener, tauriPath, tauriWebview;
  const tauriPlugins = isTauri ? tauriReady.then(() => Promise.all([
    import('@tauri-apps/plugin-fs').then(m => tauriFs = m),
    import('@tauri-apps/plugin-dialog').then(m => tauriDialog = m),
    import('@tauri-apps/plugin-opener').then(m => tauriOpener = m),
    import('@tauri-apps/api/path').then(m => tauriPath = m),
    import('@tauri-apps/api/webview').then(m => tauriWebview = m),
  ])) : Promise.resolve();

  // ── Heavy preview libraries load on FIRST USE, never at startup ──────────
  // highlight.js (+15 grammars), mermaid and pdf.js are 1.5 MB of the app that
  // most sessions never open; Files is statically imported by App and the Hub
  // drawer, so a static import here landed all of it in the entry chunk of the
  // primary (Android) target (2.30 MB before, review 2026-09-03). Each loader
  // is idempotent and memoizes its promise; the highlighter is a $state so the
  // preview it was loaded for re-renders highlighted once it arrives —
  // until then the same lines show escaped and unhighlighted.
  let hljs = $state(null);
  let hljsLoading = null;
  function loadHljs() {
    if (hljsLoading) return hljsLoading;
    hljsLoading = Promise.all([
      import('highlight.js/lib/core'),
      import('highlight.js/lib/languages/javascript'),
      import('highlight.js/lib/languages/typescript'),
      import('highlight.js/lib/languages/python'),
      import('highlight.js/lib/languages/rust'),
      import('highlight.js/lib/languages/css'),
      import('highlight.js/lib/languages/json'),
      import('highlight.js/lib/languages/bash'),
      import('highlight.js/lib/languages/xml'),
      import('highlight.js/lib/languages/yaml'),
      import('highlight.js/lib/languages/sql'),
      import('highlight.js/lib/languages/go'),
      import('highlight.js/lib/languages/java'),
      import('highlight.js/lib/languages/ruby'),
      import('highlight.js/lib/languages/markdown'),
      import('highlight.js/styles/github-dark.min.css'),
    ]).then(([core, javascript, typescript, python, rust, css, json, bash, xml, yaml, sql, go, java, ruby, markdown]) => {
      const h = core.default;
      h.registerLanguage('javascript', javascript.default);
      h.registerLanguage('js', javascript.default);
      h.registerLanguage('typescript', typescript.default);
      h.registerLanguage('ts', typescript.default);
      h.registerLanguage('python', python.default);
      h.registerLanguage('rust', rust.default);
      h.registerLanguage('css', css.default);
      h.registerLanguage('json', json.default);
      h.registerLanguage('bash', bash.default);
      h.registerLanguage('sh', bash.default);
      h.registerLanguage('html', xml.default);
      h.registerLanguage('xml', xml.default);
      h.registerLanguage('svg', xml.default);
      h.registerLanguage('yaml', yaml.default);
      h.registerLanguage('sql', sql.default);
      h.registerLanguage('go', go.default);
      h.registerLanguage('java', java.default);
      h.registerLanguage('ruby', ruby.default);
      h.registerLanguage('markdown', markdown.default);
      hljs = h;
      return h;
    }).catch(() => { hljsLoading = null; return null; }); // offline: retry next time
    return hljsLoading;
  }

  let mermaidLoading = null;
  function loadMermaid() {
    if (mermaidLoading) return mermaidLoading;
    mermaidLoading = import('mermaid').then(m => {
      m.default.initialize({ startOnLoad: false, theme: 'dark' });
      return m.default;
    }).catch(() => { mermaidLoading = null; return null; });
    return mermaidLoading;
  }

  let pdfjsLoading = null;
  function loadPdfjs() {
    if (pdfjsLoading) return pdfjsLoading;
    pdfjsLoading = Promise.all([
      import('pdfjs-dist'),
      import('pdfjs-dist/build/pdf.worker.min.mjs?url'),
    ]).then(([lib, worker]) => {
      lib.GlobalWorkerOptions.workerSrc = worker.default;
      return lib;
    }).catch(() => { pdfjsLoading = null; return null; });
    return pdfjsLoading;
  }

  let { session = '', onGoBack = null, visible = false, fontSize = 14, singlePane = false, navRequest = null, jumped = false, currentDir = $bindable('') } = $props();

  /* The confirmation becomes a bottom sheet on a phone-sized viewport, the same
     rule the Hub's dialogs use. */
  let narrowViewport = $state(typeof window !== 'undefined' && window.matchMedia('(max-width: 760px)').matches);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 760px)');
    const onChange = () => { narrowViewport = mq.matches; };
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  });

  function navPush() { history.pushState({ app: true }, ''); }

  // Register goBack for Android back gesture
  $effect(() => {
    if (onGoBack) onGoBack(() => {
      // navAnim('back') rides only the branches that CHANGE the view — the
      // git panel's internal peel and the unsaved-changes dialog move
      // nothing, so they must not slide the page.
      if (view === 'git') {
        if (gitPanelRef?.goBack()) return true;
        navAnim('back'); view = 'list'; return true;
      }
      if (view === 'edit') {
        leaveEditor(() => { navAnim('back'); view = 'preview'; });
        return true;
      }
      if (view === 'info') { navAnim('back'); view = fromGit ? (fromGit = false, 'git') : currentFile?.content != null ? 'preview' : 'list'; return true; }
      if (view === 'preview') { navAnim('back'); if (fromGit) { fromGit = false; view = 'git'; } else { view = 'list'; } currentFile = null; return true; }
      if (view === 'local') { navAnim('back'); view = 'list'; return true; }
      // The directory step retraces the user's OWN path (board #17); at the
      // entry point the stack is empty and the floor decides — App returns a
      // chat jump to the chat, a tab visit to the root.
      if (popDir()) return true;
      // Own path exhausted: a TAB visit CLIMBS to the parent instead of
      // leaving the page (board #47: "应该返回上级目录 不是去terminal") —
      // via loadDir, never navTo, or the climb would push DIR history for the
      // next back to bounce back down. This climb itself has no forward
      // navigation entry to consume (unlike popDir), so replenish the APP
      // history entry that this pop just spent; otherwise a deep path stalls
      // once the older entries run out. At / there is nothing above, so it
      // falls through (App re-pushes). A chat-jumped visit stands aside:
      // its floor is the conversation, App's return slot below.
      if (!jumped && cwd && cwd !== '/') {
        navAnim('back');
        navPush();
        loadDir(cwd.replace(/\/[^/]+\/?$/, '') || '/');
        return true;
      }
      return false;
    });
  });

  // State
  let cwd = $state('');
  let entries = $state([]);
  let showHidden = $state(false);
  let loading = $state(false);        // directory listing in flight (left list)
  let previewLoading = $state(false); // file content in flight (right preview)
  let error = $state('');

  // View modes: 'list', 'preview', 'edit', 'info', 'local'
  let view = $state('list');
  let previewZoom = $state(100);

  // ─── Desktop two-pane (folder browser | preview) ──────────────────────────
  // On a wide, non-touch screen Files becomes a desktop-style file manager:
  // the list is always on the left, the preview fills the right. On mobile /
  // narrow / touch we keep the single-pane `view` chain unchanged. Mirrors the
  // split the Team tab already ships (Team.svelte).
  const SPLIT_MIN_WIDTH = 900;
  let wideEnough = $state(typeof window !== 'undefined' && window.innerWidth >= SPLIT_MIN_WIDTH);
  // singlePane: the embedder (Hub's drawer) opts into the phone view chain —
  // its column is 320–900px of a WIDE window, so the window-width heuristic
  // lies there (owner, 2026-08-28: "文件的侧边栏可以是类似手机的单页模式").
  let splitEligible = $derived(!singlePane && !layout.isTouchDevice && (layout.forceDesktop || wideEnough));

  $effect(() => {
    const onResize = () => { wideEnough = window.innerWidth >= SPLIT_MIN_WIDTH; };
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  // Drag the splitter to adjust the browser/preview width ratio (desktop only).

  // Android local files
  let localFiles = $state([]);
  let localDir = $state('');

  async function getLocalDir() {
    if (!isTauri) return '';
    return '/storage/emulated/0/Download/TmuxMobile/';
  }

  async function openLocalFiles() {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const files = await invoke('list_downloads');
      localFiles = files.map(file => typeof file === 'string' ? { name: file, modified: 0 } : file);
      localDir = '/storage/emulated/0/Download/TmuxMobile/';
      view = 'local';
      navPush();
    } catch (e) { error = e.message; }
  }

  function getFileOpener() {
    try {
      const opener = window.AndroidFileOpener;
      return opener?.ping?.() === 'ok' ? opener : null;
    } catch {
      return null;
    }
  }

  async function waitForFileOpener(maxWait = 5000) {
    let opener = getFileOpener();
    if (opener) return opener;
    const start = Date.now();
    while (Date.now() - start < maxWait) {
      await new Promise(r => setTimeout(r, 100));
      opener = getFileOpener();
      if (opener) return opener;
    }
    return null;
  }

  async function openFileNative(path) {
    const opener = getFileOpener() || await waitForFileOpener();
    if (!opener) throw new Error('No file opener available after app resume');
    const result = opener.openFile(path);
    if (result !== 'ok') throw new Error(result);
  }

  async function openLocalFile(name) {
    try {
      await openFileNative(localDir + name);
    } catch (e) { error = t('openFailed') + (e.message || e); }
  }

  async function deleteLocalFile(name) {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('delete_download', { name });
      localFiles = localFiles.filter(f => f.name !== name);
    } catch (e) { error = e.message; }
  }
  let currentFile = $state(null); // { path, name, stat, content }
  let editContent = $state('');
  let editOriginal = $state('');
  let undoStack = $state([]);
  // Refs for the three stacked editor layers (line-number gutter, syntax
  // highlight, and the transparent textarea on top). The textarea is the only
  // scrollable layer; the gutter and highlight are kept in lockstep with it via
  // syncEditorScroll so line numbers always line up with their lines — even
  // when a long line scrolls horizontally instead of soft-wrapping.
  let taEl;     // textarea
  let numsEl;   // gutter
  let hlEl;     // highlight <pre>
  let layerEl;  // .editor-layer (sized to the highlight/textarea content box)
  let mirrorEl; // off-screen per-line mirror used to measure wrapped line heights
  // Per-logical-line pixel heights, measured from the mirror. The gutter renders
  // one number block per entry at that height, so numbers stay aligned with the
  // text whether or not lines wrap. Empty until first measure (the gutter then
  // falls back to natural line height via the `h ? … : ''` guard).
  let lineHeights = $state([]);
  // Soft-wrap toggle. Prose (markdown / plain text) defaults to wrapped — no
  // horizontal scrolling while writing, and line numbers are hidden since they
  // can't track wrapped rows. Code/config (yaml, json, rs, …) defaults to
  // no-wrap so the gutter stays exactly aligned (long lines scroll sideways).
  let wrapLines = $state(false);
  function defaultWrapForMime(mime) {
    return mime === 'text/markdown' || mime === 'text/plain';
  }
  /** The destructive action awaiting confirmation:
   *   { kind: 'file', path }     — delete on the server, no trash, no undo
   *   { kind: 'local', name }    — delete a downloaded copy (had NO confirm)
   *   { kind: 'leave', run }     — abandon unsaved edits; `run` is the parked
   *                                move (was a native confirm(), i.e. an OS
   *                                dialog in the middle of our UI)
   * Tap-to-confirm is gone: it re-labelled the button for 3s, said nothing about
   * what is lost, and differed from every other destructive verb in the app. */
  let pendingAct = $state(null);
  let acting = $state(false);
  /** EVERY way out of the editor goes through here — the back button/gesture,
   *  a session switch, the cwd follow, the drawer's "look here". No edits:
   *  `run` moves now. Unsaved edits: `run` waits behind the discard dialog and
   *  fires on confirm; cancel DROPS it — a move is never queued. Until
   *  2026-09-03 only the back button asked; the other three set view = 'list'
   *  outright and the text was gone. `untrack` because the callers are
   *  $effects that must not start re-running on every keystroke. */
  function leaveEditor(run) {
    if (untrack(() => leaveDecision({ view, edited: isEdited })) === 'go') { run(); return; }
    pendingAct = { kind: 'leave', run };
  }
  const ACT_COPY = {
    file:  { title: 'confirmDeleteFileTitle',  note: 'confirmDeleteFileNote',  go: 'delete' },
    local: { title: 'confirmDeleteFileTitle',  note: 'confirmDeleteFileNote',  go: 'delete' },
    leave: { title: 'confirmDiscardTitle',     note: 'confirmDiscardNote',     go: 'confirmDiscard' },
  };
  const actName = (a) => (a?.kind === 'file' ? (a.path.split('/').pop() ?? a.path) : a?.name ?? '');
  async function runPendingAct() {
    if (!pendingAct || acting) return;
    const act = pendingAct;
    acting = true;
    try {
      if (act.kind === 'file') await handleDelete(act.path);
      else if (act.kind === 'local') await deleteLocalFile(act.name);
      else act.run();
      pendingAct = null;
    } finally { acting = false; }
  }
  let newName = $state('');
  let newType = $state(''); // 'file' or 'dir'
  let renaming = $state(null); // path being renamed
  let renameValue = $state('');
  let bcPathEl = $state(null);
  let pdfContainer = $state(null);
  let filesEl = $state(null);
  let bookmarks = $state([]);
  let showBookmarks = $state(false);
  let recentFiles = $state([]);
  let showRecent = $state(false);

  // Both lists are server-persisted whole arrays with clobber/race guards
  // (single-flighted first load, generation-counter staleness checks) —
  // the shared discipline lives in persisted-list.ts; these are mirrors.
  const recentsList = createPersistedList({
    fetch: async () => (await getPrefs()).recentFiles || [],
    persist: (items) => setPref('recentFiles', items),
    onChange: (items) => { recentFiles = items; },
  });

  $effect(() => {
    // Depend on `visible` so this re-runs when the user opens the Files tab.
    // Files is always mounted now (even before the socket connects), and this
    // effect has no other reactive dep — without the `visible` gate it would
    // fire once at mount (often pre-connection), fail, and never retry, leaving
    // Recent empty forever.
    if (!visible) return;
    recentsList.load().catch(() => {});
  });

  function addRecent(path, name) {
    return recentsList.mutate(items => [{ path, name }, ...items.filter(f => f.path !== path)].slice(0, 20));
  }

  // Git: the panel itself (status/log/diff/commit/push) lives in
  // GitPanel.svelte and owns all git state. Files keeps only the
  // "is this a repo?" probe for the toolbar button, the view routing,
  // and the fromGit flag for back-navigation out of previews.
  let hasGit = $state(false);
  let fromGit = $state(false);
  let gitPanelRef = $state(null);

  $effect(() => {
    if (cwd) {
      gitCmd('rev-parse', ['--git-dir'], cwd).then(r => { hasGit = r.code === 0; }).catch(() => { hasGit = false; });
    }
  });

  function openGitView() {
    view = 'git';
    navPush();
  }

  const bookmarksList = createPersistedList({
    fetch: async () => (await getBookmarks()).bookmarks || [],
    persist: (items) => saveBookmarks(items),
    onChange: (items) => { bookmarks = items; },
  });

  $effect(() => {
    // Same visible-gate rationale as the recents effect above.
    if (!visible) return;
    bookmarksList.load().catch(() => {});
  });

  function isBookmarked(path) { return bookmarks.includes(path); }

  function toggleBookmark(path) {
    return bookmarksList.mutate(items => (
      items.includes(path) ? items.filter(b => b !== path) : [...items, path]
    ));
  }

  // Edge-swipe back, INTERACTIVE (owner, 2026-08-25: "文件浏览页面的滑动手势
  // …做的更丝滑一些"): the page follows the finger with damping instead of
  // firing blind at finger-lift, releases spring back, and a commit plays the
  // drill-back slide. Two guards the old two-liner lacked: an intent lock
  // (a diagonal scroll that drifted 60px right used to trigger goBack), and
  // cancelability (drag out, drag back, release = nothing). The transform
  // lives only while the finger does — a resting transform would make .files
  // a containing block and break fixed popovers (design-language.md §2).
  // (Pull-to-refresh stays removed — it conflicts with top-edge scrolling.)
  const SWIPE_EDGE = 40, SWIPE_COMMIT = 60, SWIPE_DAMP = 0.4, SWIPE_CAP = 96;
  let swipeStartX = 0, swipeStartY = 0;
  let swipeEdge = false;      // started at the left edge
  let swipeIntent = 0;        // 0 undecided, 1 horizontal, -1 vertical
  let swipeDX = $state(0);    // damped, drives the live translate
  let swipeSnap = $state(false); // animate the spring-back

  function onTouchStart(e) {
    swipeStartX = e.touches[0].clientX;
    swipeStartY = e.touches[0].clientY;
    swipeEdge = swipeStartX < SWIPE_EDGE && !splitEligible;
    swipeIntent = 0;
    swipeSnap = false;
  }
  function onTouchMove(e) {
    if (!swipeEdge) return;
    const dx = e.touches[0].clientX - swipeStartX;
    const dy = e.touches[0].clientY - swipeStartY;
    if (swipeIntent === 0 && (Math.abs(dx) > 8 || Math.abs(dy) > 8))
      swipeIntent = Math.abs(dx) > Math.abs(dy) * 1.2 ? 1 : -1;
    swipeDX = swipeIntent === 1 ? Math.min(Math.max(dx, 0) * SWIPE_DAMP, SWIPE_CAP) : 0;
  }
  function onTouchEnd(e) {
    if (!swipeEdge) return;
    const dx = e.changedTouches[0].clientX - swipeStartX;
    swipeEdge = false;
    if (swipeIntent === 1 && dx > SWIPE_COMMIT) { swipeDX = 0; goBack(); }
    else if (swipeDX > 0) { swipeSnap = true; swipeDX = 0; }
  }
  function onTouchCancel() {
    if (!swipeEdge) return;
    swipeEdge = false;
    if (swipeDX > 0) { swipeSnap = true; swipeDX = 0; }
  }

  // Navigation motion (the drill grammar in design-language.md §1): going
  // deeper enters from the right, back from the left — single-pane touch
  // layout only; the desktop split has no view chain to slide. The class is
  // dropped and re-added a frame later so a second hop in the SAME direction
  // replays the animation.
  let navAnimClass = $state('');
  function navAnim(dir) {
    if (splitEligible) return;
    navAnimClass = '';
    requestAnimationFrame(() => { navAnimClass = dir; });
  }

  // ── BACK is a HISTORY, not a parent walk (board #17) ─────────────────
  // "实现一个类似于'后退'的逻辑": every navigation the USER makes inside the
  // page (entering a directory, a crumb, a bookmark, the up/home buttons)
  // pushes where they WERE, and back pops exactly that path — so back retraces
  // the user's own steps. An EXTERNAL move (session switch, the cwd follow
  // rule, a drawer handoff) RESETS the stack instead: it is a new entry point.
  // Below the stack, a tab visit climbs parent directories to / (board #47);
  // only a chat-jumped visit leaves the page, via App's return slot.
  let dirHist = [];
  function navTo(path) {
    if (cwd && path !== cwd) dirHist.push(cwd);
    loadDir(path);
  }
  function popDir() {
    const prev = dirHist.pop();
    if (prev == null) return false;
    navAnim('back');
    loadDir(prev);
    return true;
  }

  function goBack() {
    navAnim('back');
    if (view === 'edit') { view = 'preview'; }
    else if (view === 'info') { view = fromGit ? (fromGit = false, 'git') : currentFile?.content != null ? 'preview' : 'list'; }
    else if (view === 'preview') { if (fromGit) { fromGit = false; view = 'git'; } else { view = 'list'; } currentFile = null; }
    else popDir();
  }

  async function renderPdf(data) {
    if (!pdfContainer) return;
    const pdfjsLib = await loadPdfjs();
    if (!pdfjsLib || !pdfContainer) return;
    pdfContainer.innerHTML = '';
    const bytes = Uint8Array.from(atob(data), c => c.charCodeAt(0));
    const pdf = await pdfjsLib.getDocument({ data: bytes, verbosity: 0 }).promise;
    for (let i = 1; i <= pdf.numPages; i++) {
      const page = await pdf.getPage(i);
      const scale = (pdfContainer.clientWidth || 360) / page.getViewport({ scale: 1 }).width;
      const viewport = page.getViewport({ scale });
      const canvas = document.createElement('canvas');
      canvas.width = viewport.width;
      canvas.height = viewport.height;
      canvas.style.width = '100%';
      canvas.style.marginBottom = '4px';
      pdfContainer.appendChild(canvas);
      await page.render({ canvasContext: canvas.getContext('2d'), viewport }).promise;
    }
  }

  // Breadcrumb parts
  let breadcrumbs = $derived.by(() => {
    if (!cwd) return [];
    const parts = cwd.split('/').filter(Boolean);
    return parts.map((name, i) => ({
      name,
      path: '/' + parts.slice(0, i + 1).join('/')
    }));
  });

  let isEdited = $derived(view === 'edit' && editContent !== editOriginal);
  // True when the preview is the per-line code/text view (where a wrap toggle
  // makes sense) — not markdown/csv/html/pdf/image/converted output.
  let isLinedPreview = $derived.by(() => {
    if (!currentFile?.stat || currentFile?.convertedHtml) return false;
    const cat = mimeCategory(currentFile.stat.mime_hint);
    return cat === 'code' || cat === 'other';
  });

  $effect(() => {
    cwd;
    setTimeout(() => { if (bcPathEl) bcPathEl.scrollLeft = bcPathEl.scrollWidth; }, 0);
  });

  // Sync Files to the terminal/team working directory — but only when that dir
  // CHANGES (you switched pane/team, or cd'd). If it's unchanged, your own
  // in-Files navigation is preserved across tab switches. Re-checked whenever
  // Files becomes visible or the session changes.
  //
  // Per-SESSION browse memory (owner, 2026-08-22: "一个project刷新路径后 这个
  // project当前又动过路径需要暂时记下来，在切到其他project再切回来能返回之前
  // 这个project的路径状态位置"): switching projects parks where you were
  // browsing under the project you leave and restores the other project's
  // parked position. In-memory on purpose — it is a temporary reading
  // position, not a preference — and the existing rule still outranks it:
  // when THAT project's tmux cwd moved while you were away (someone cd'd),
  // following the real cwd wins over the parked position.
  // svelte-ignore state_referenced_locally — the INITIAL session is exactly
  // what "previous" means before the first switch; the effect updates it.
  let prevSession = session;
  // A NEW instance starts from the shared parked position (the Hub's drawer
  // mounts a fresh Files on every open — without this it forgot its place;
  // owner, 2026-08-28: "每个 project 自己记录自己的 current路径").
  // svelte-ignore state_referenced_locally — the MOUNT-time session is the one
  // whose parked position a new instance should wake up in.
  const parked0 = browsed.get(session);
  let lastSourceDir = parked0?.sourceDir ?? '';
  if (parked0?.cwd) cwd = parked0.cwd;
  // Park on unmount too — the drawer instance dies with the drawer, and a
  // position recorded only at the next session switch would never be written.
  $effect(() => () => { browsed.set(prevSession, { cwd, sourceDir: lastSourceDir }); });
  $effect(() => {
    if (!visible) { void session; return; }
    if (session !== prevSession) {
      // cwd still holds the OLD session's position — nothing else resets it.
      browsed.set(prevSession, { cwd, sourceDir: lastSourceDir });
      prevSession = session;
      const parked = browsed.get(session);
      lastSourceDir = parked?.sourceDir ?? '';
      dirHist = []; // a session switch is a new entry point, not a step
      if (parked?.cwd) {
        // An unsaved editor holds the switch behind the discard dialog
        // (leaveEditor); the parked position is a hint, the text is not.
        leaveEditor(() => { cwd = parked.cwd; view = 'list'; loadDir(parked.cwd); });
      }
    }
    if (cwd && !entries.length && !loading) loadDir(cwd); // restored park: list it
    // session may be '' when Files is opened before any terminal pane exists —
    // the server then reports the user's home directory. Once a terminal/team
    // session appears, its cwd differs from home and we follow it.
    fsCwd(session).then(r => {
      // The follow is DISARMED before it asks (lastSourceDir moves first): a
      // cancelled follow is skipped for this event, not queued — the next
      // re-run sees the same cwd and stays quiet (file-view-state.test.ts).
      const step = cwdFollowStep(r.path, lastSourceDir, untrack(() => ({ view, edited: isEdited })));
      lastSourceDir = step.lastSourceDir;
      if (step.move === 'none') return;
      leaveEditor(() => {
        cwd = r.path;
        view = 'list';
        dirHist = []; // the follow rule moved us — a new entry point
        loadDir(r.path);
      });
    }).catch(() => {
      if (!lastSourceDir) { lastSourceDir = '/'; cwd = '/'; loadDir('/'); }
    });
  });

  // Refresh the preview content when returning to the tab on a preview.
  $effect(() => {
    if (visible && view === 'preview') reloadPreview();
  });

  // The embedder can read where this instance is (the drawer's maximize
  // button hands its cwd to the Files PAGE instance).
  $effect(() => { currentDir = cwd; });

  // An imperative "go there" from outside. Same shape as AgentsPage's
  // editRequest: a bumped `n` re-fires an identical path.
  let lastNav = 0;
  $effect(() => {
    if (!navRequest || navRequest.n === lastNav) return;
    lastNav = navRequest.n;
    if (navRequest.path) {
      const to = navRequest.path;
      leaveEditor(() => {
        view = 'list';
        dirHist = []; // a drawer/see-here handoff is a new entry point
        loadDir(to);
      });
      // Disarm the cwd-follow for the CURRENT real cwd: without this a
      // same-moment session switch re-follows the project root and stomps
      // the requested directory.
      fsCwd(session).then((r) => { if (r.path) lastSourceDir = r.path; }).catch(() => {});
    }
  });

  // A DIFFERENT directory's rows unfold (motion.md principle 15): the rows are
  // keyed by path, so a navigation remounts them all and `.reveal` staggers
  // them in under the dim that was already there; a refresh of the same
  // directory keeps its nodes and gets no animation at all.
  let revealDir = $state('');
  let loadSeq = 0;
  async function loadDir(path, purpose = 'navigate') {
    const my = ++loadSeq; // several callers can navigate concurrently around a
    loading = true;       // session switch — the NEWEST intent wins (DirPicker's rule)
    error = '';
    try {
      const r = await fsList(path, showHidden);
      if (my !== loadSeq) return;
      revealDir = path !== cwd ? path : '';
      entries = r.entries;
      cwd = path;
      ({ view, currentFile } = directoryLoadState({ view, currentFile }, purpose));
    } catch (e) {
      if (my !== loadSeq) return;
      error = e.message;
    }
    loading = false;
  }

  // After a reconnect, refresh the directory data without treating it as
  // navigation. The active preview/editor belongs to the user and must survive
  // an app resume even though the underlying file list may have changed.
  $effect(() => {
    const onReconn = () => { if (visible && cwd) loadDir(cwd, 'refresh'); };
    window.addEventListener('ws-reconnected', onReconn);
    return () => window.removeEventListener('ws-reconnected', onReconn);
  });

  function goUp() {
    navAnim('back');
    const parent = cwd.replace(/\/[^/]+\/?$/, '') || '/';
    navTo(parent);
  }

  function scrollEnd(el) { el.scrollLeft = el.scrollWidth; }

  // Back to the SESSION's working directory (the pane's cwd) — not the user's
  // home. The button wore the house icon until 2026-09-03 while DirPicker's
  // house meant `~`: one glyph, two destinations. Home now means `~`
  // everywhere; this control is the terminal glyph, labelled for what it is.
  function goSessionDir() {
    fsCwd(session).then(r => navTo(r.path)).catch(() => navTo('/'));
  }

  const PREVIEW_SIZE_LIMIT = 5 * 1024 * 1024;

  function isPreviewable(stat, name) {
    if (!stat) return false;
    const m = stat.mime_hint || '';
    if (m === 'application/pdf') return true;
    if (m.startsWith('image/')) return true;
    if (stat.is_text && stat.size <= 512 * 1024) return true;
    if (name && /\.pptx$/i.test(name)) return true;
    return false;
  }

  async function loadPreviewContent(file) {
    previewLoading = true;
    try {
      const { path, name, stat } = file;
      if (stat.mime_hint === 'application/pdf') {
        const r = await fsDownload(path);
        currentFile = { ...file, pdfData: r.data };
        view = 'preview';
      } else if (stat.mime_hint.startsWith('image/')) {
        const r = await fsDownload(path);
        currentFile = { ...file, dataUrl: `data:${stat.mime_hint};base64,${r.data}` };
        view = 'preview';
      } else if (stat.is_text && stat.size <= 512 * 1024) {
        const r = await fsRead(path);
        if (mimeCategory(stat.mime_hint || '') !== 'markdown') loadHljs(); // lined view: highlight when it lands
        showAllLines = false; // the cap is per file
        currentFile = { ...file, content: r.content };
        wrapLines = defaultWrapForMime(stat.mime_hint || '');
        view = 'preview';
      } else if (/\.pptx$/i.test(name)) {
        const r = await fsConvert(path);
        currentFile = { ...file, convertedHtml: r.html };
        view = 'preview';
      }
    } catch (e) {
      error = e.message;
    }
    previewLoading = false;
  }

  async function openEntry(entry) {
    navAnim('fwd');
    if (entry.type === 'dir') {
      navPush();
      navTo(entry.path);
      return;
    }
    if (entry.type === 'broken') {
      // Dangling symlink — nothing to preview, surface a clear error.
      const tgt = entry.link_target ? ` → ${entry.link_target}` : '';
      error = `Broken symlink: ${entry.name}${tgt}`;
      return;
    }
    // File open: use previewLoading (right pane), NOT loading — `loading` drives
    // the left directory list's spinner, and toggling it here would make the
    // whole list flash/re-render every time you click a file in the desktop
    // two-pane layout (on mobile the list was hidden so it went unnoticed).
    previewLoading = true;
    try {
      const stat = await fsStat(entry.path);
      currentFile = { path: entry.path, name: entry.name, stat };
      addRecent(entry.path, entry.name);
      navPush();
      if (stat.size > PREVIEW_SIZE_LIMIT || !isPreviewable(stat, entry.name)) {
        view = 'info';
        previewLoading = false;
        return;
      }
    } catch (e) {
      error = e.message;
      previewLoading = false;
      return;
    }
    previewLoading = false;
    await loadPreviewContent(currentFile);
  }

  async function reloadPreview() {
    if (!currentFile?.path) return;
    try {
      if (currentFile.stat?.is_text) {
        const r = await fsRead(currentFile.path);
        currentFile.content = r.content;
        currentFile = currentFile; // trigger reactivity
      } else if (currentFile.stat?.mime_hint?.startsWith('image/')) {
        const r = await fsDownload(currentFile.path);
        currentFile.dataUrl = `data:${currentFile.stat.mime_hint};base64,${r.data}`;
        currentFile = currentFile;
      }
    } catch {}
  }

  function startEdit() {
    loadHljs(); // the editor's highlight overlay follows the same lazy load
    editContent = currentFile.content;
    editOriginal = currentFile.content;
    undoStack = [];
    wrapLines = defaultWrapForMime(currentFile?.stat?.mime_hint || '');
    view = 'edit';
    navPush();
  }

  function undo() {
    if (undoStack.length) {
      editContent = undoStack.pop();
      undoStack = undoStack; // trigger reactivity
    } else {
      editContent = editOriginal;
    }
  }

  function onEditInput(e) {
    undoStack.push(editContent);
    if (undoStack.length > 50) undoStack.shift();
    undoStack = undoStack;
    editContent = e.target.value;
    // Content changed (line count / cursor may have moved) — realign the gutter
    // and highlight layer on the next frame, after the DOM reflows.
    requestAnimationFrame(syncEditorScroll);
  }

  // Keep the line-number gutter and highlight layer scrolled in lockstep with
  // the textarea. The textarea owns the scroll (vertical + horizontal, since we
  // no longer soft-wrap); the gutter follows vertically only (numbers stay
  // pinned left) and the highlight follows both axes so the coloured text sits
  // exactly under the caret.
  function syncEditorScroll() {
    if (!taEl) return;
    if (numsEl) numsEl.scrollTop = taEl.scrollTop;
    if (hlEl) {
      hlEl.scrollTop = taEl.scrollTop;
      hlEl.scrollLeft = wrapLines ? 0 : taEl.scrollLeft;
    }
  }

  // Measure each logical line's rendered height via the off-screen mirror, then
  // feed the gutter so line numbers line up with (possibly wrapped) lines.
  function measureLineHeights() {
    if (view !== 'edit' || !mirrorEl || !layerEl) return;
    const cw = layerEl.clientWidth - 24; // minus the 12px left+right padding
    if (cw <= 0) return;
    mirrorEl.style.width = cw + 'px';
    lineHeights = Array.from(mirrorEl.children).map(c => c.offsetHeight);
  }
  // Re-measure when content, wrap mode, or font size changes (after the DOM
  // reflows). The reads below register the reactive dependencies.
  $effect(() => {
    editContent; wrapLines; fontSize; view;
    requestAnimationFrame(measureLineHeights);
  });
  // Re-measure on width changes (rotation, split-pane drag, keyboard show/hide).
  $effect(() => {
    if (!layerEl) return;
    const ro = new ResizeObserver(() => measureLineHeights());
    ro.observe(layerEl);
    return () => ro.disconnect();
  });

  async function saveFile() {
    try {
      await fsWrite(currentFile.path, editContent);
      editOriginal = editContent;
      currentFile.content = editContent;
      undoStack = [];
    } catch (e) {
      error = e.message;
    }
  }

  function backToList() {
    navAnim('back');
    if (fromGit) { fromGit = false; view = 'git'; } else {
      // Stay in the file's parent directory, not session cwd
      const dir = currentFile?.path?.replace(/\/[^/]+$/, '') || cwd;
      view = 'list';
      if (dir !== cwd) loadDir(dir);
    }
    currentFile = null;
  }

  function backToPreview() {
    // navAnim rides INSIDE the move: the dialog itself moves nothing.
    leaveEditor(() => { navAnim('back'); view = 'preview'; });
  }

  async function handleDelete(path) {
    try {
      await fsDelete(path);
      if (view !== 'list') backToList();
      loadDir(cwd);
    } catch (e) { error = e.message; }
  }

  async function handleNewItem() {
    if (!newName.trim()) return;
    const path = cwd.replace(/\/$/, '') + '/' + newName.trim();
    try {
      if (newType === 'dir') {
        await fsMkdir(path);
      } else {
        await fsWrite(path, '');
      }
      newName = '';
      newType = '';
      loadDir(cwd);
    } catch (e) { error = e.message; }
  }

  async function handleRename() {
    if (!renameValue.trim() || !renaming) return;
    const dir = renaming.replace(/\/[^/]+$/, '');
    const newPath = dir + '/' + renameValue.trim();
    try {
      await fsRename(renaming, newPath);
      renaming = null;
      renameValue = '';
      loadDir(cwd);
    } catch (e) { error = e.message; }
  }

  let downloadToast = $state('');
  let downloadedPath = $state('');
  let downloading = $state('');
  // Real progress 0–100, driven by the byte counter in fetchBytes (0–95)
  // and the final write step (95–100). No more synthetic timer that always
  // hovered at 80% — that was the user-visible "stuck at 80%" bug.
  let dlProgress = $state(0);
  let displayedDlProgress = $derived(Math.max(0, Math.min(100, Math.round(dlProgress))));

  async function openDownloaded() {
    if (!downloadedPath) return;
    try {
      if (isAndroid) {
        await openFileNative(downloadedPath);
      } else if (isTauri) {
        await tauriPlugins;
        await tauriOpener.openPath(downloadedPath);
      }
      dismissDownload();
    } catch (e) { error = t('openFailed') + (e.message || e); }
  }

  function dismissDownload() {
    downloadToast = ''; downloadedPath = ''; downloading = ''; dlProgress = 0;
  }

  // Stream one HTTP response body into `chunks`, counting bytes. A stall
  // watchdog aborts the fetch if no bytes arrive for STALL_TIMEOUT_MS —
  // reverse proxies on the public internet love to silently kill long
  // responses, and without the watchdog reader.read() hangs forever.
  const DL_STALL_TIMEOUT_MS = 20000;
  async function fetchRangeInto(url, startByte, chunks, onBytes) {
    const ctrl = new AbortController();
    let stallTimer = null;
    const armStall = () => {
      clearTimeout(stallTimer);
      stallTimer = setTimeout(() => ctrl.abort(new Error('download stalled')), DL_STALL_TIMEOUT_MS);
    };
    armStall();
    try {
      const headers = startByte > 0 ? { Range: `bytes=${startByte}-` } : {};
      const resp = await fetch(url, { headers, signal: ctrl.signal });
      if (!resp.ok && resp.status !== 206) throw new Error(`HTTP ${resp.status}`);
      // Asked to resume but got a full 200 (server/proxy ignored Range):
      // the bytes we already have would be duplicated. Restart cleanly.
      if (startByte > 0 && resp.status !== 206) {
        chunks.length = 0;
        onBytes?.(-startByte, 0);
        startByte = 0;
      }
      // total size of the WHOLE file (for progress), regardless of range
      let total = 0;
      const cr = resp.headers.get('content-range'); // "bytes N-M/SIZE"
      if (cr) total = Number(cr.split('/')[1]) || 0;
      else total = Number(resp.headers.get('content-length')) || 0;
      if (!resp.body || !resp.body.getReader) {
        const buf = await resp.arrayBuffer();
        clearTimeout(stallTimer);
        chunks.push(new Uint8Array(buf));
        onBytes?.(buf.byteLength, total);
        return;
      }
      const reader = resp.body.getReader();
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        armStall();
        chunks.push(value);
        onBytes?.(value.length, total);
      }
    } finally {
      clearTimeout(stallTimer);
    }
  }

  // Fetch a download into a Uint8Array with REAL progress + automatic
  // resume. On a mid-transfer failure (proxy idle timeout, network blip,
  // stall) we retry with `Range: bytes=<received>-` so completed bytes are
  // never re-downloaded. Each retry calls `freshUrl()` because the signed
  // /dl URL expires after 60 s — a retry minutes into a big transfer would
  // otherwise 403. The retry budget refills whenever a retry makes real
  // progress, so a flaky-but-moving link can take many small hits without
  // dying; only consecutive no-progress failures give up.
  const DL_MAX_RETRIES = 4;
  const DL_RETRY_DELAY_MS = 1500;
  // Zero bytes after this many attempts ⇒ the HTTP path is unreachable
  // (typical: a reverse proxy that forwards WebSocket upgrades but not
  // plain GETs — WS works fine, every fetch dies instantly). Retrying is
  // pointless; fail fast with a marker so the caller can fall back to the
  // WS RPC download path.
  const DL_UNREACHABLE_ATTEMPTS = 2;
  async function fetchWithResume(url, freshUrl, onProgress) {
    const chunks = [];
    let received = 0;
    let totalSize = 0;
    let retriesLeft = DL_MAX_RETRIES;
    let attempts = 0;
    const onBytes = (n, total) => {
      received += n;
      if (total) totalSize = total;
      if (totalSize) onProgress?.(received / totalSize);
      else onProgress?.(null); // size unknown → indeterminate
    };
    let curUrl = url;
    while (true) {
      const receivedBefore = received;
      try {
        attempts++;
        await fetchRangeInto(curUrl, received, chunks, onBytes);
        break; // complete
      } catch (e) {
        // If the whole-file size is known and we already have every byte,
        // treat the error as EOF noise (some proxies cut the connection
        // instead of finishing cleanly).
        if (totalSize && received >= totalSize) break;
        // Never received a single byte across multiple attempts: the HTTP
        // endpoint is unreachable, not flaky. Surface a typed error so the
        // caller can switch transports instead of burning the full retry
        // budget on a path that will never work.
        if (received === 0 && attempts >= DL_UNREACHABLE_ATTEMPTS) {
          const err = new Error(`HTTP download unreachable: ${e.message}`);
          err.code = 'DL_HTTP_UNREACHABLE';
          throw err;
        }
        if (received > receivedBefore) retriesLeft = DL_MAX_RETRIES; // made progress
        if (retriesLeft <= 0) throw e;
        retriesLeft--;
        window.__dbg?.(`dl: retry at byte ${received} (${retriesLeft} left): ${e.message}`);
        await new Promise(r => setTimeout(r, DL_RETRY_DELAY_MS));
        // Signed URL may have expired (60 s TTL) — get a fresh one.
        try { curUrl = await freshUrl(); } catch { /* keep old URL */ }
      }
    }
    const out = new Uint8Array(received);
    let off = 0;
    for (const c of chunks) { out.set(c, off); off += c.length; }
    return out;
  }

  async function handleDownload(path) {
    const name = path.split('/').pop();
    try {
      downloading = name;
      dlProgress = 0;
      window.__dbg?.(`dl: start ${name}`);
      const t0 = Date.now();
      const dlInfo = await fsDownloadHttp(path);
      window.__dbg?.(`dl: got ${dlInfo.url ? 'HTTP URL' : 'base64'} in ${Date.now()-t0}ms`);

      // Pull the bytes. Same shape regardless of platform; downstream code
      // either writes via Tauri fs/invoke or triggers a browser download.
      // Progress 0..0.95 is reserved for fetch; 0.95..1.00 for write.
      let bytes;
      if (dlInfo.url) {
        // freshUrl re-signs on each retry: the /dl signature has a 60 s TTL,
        // so resuming a long transfer needs a new URL, not the original.
        const freshUrl = () => fsDownloadHttp(path).then(info => info.url);
        try {
          bytes = await fetchWithResume(dlInfo.url, freshUrl, (frac) => {
            if (frac == null) {
              // Indeterminate: tick a slow ramp so the bar isn't motionless.
              if (dlProgress < 90) dlProgress = Math.min(90, dlProgress + 1);
            } else {
              dlProgress = Math.round(frac * 95);
            }
          });
        } catch (e) {
          if (e.code !== 'DL_HTTP_UNREACHABLE') throw e;
          // The WS connection demonstrably works (we just got the signed
          // URL over it) but plain HTTP to the same host doesn't — typical
          // when a reverse proxy only forwards WebSocket upgrades. Fall
          // back to the WS RPC download (base64, 50 MB server-side cap).
          window.__dbg?.('dl: HTTP unreachable → falling back to WS RPC');
          const r = await fsDownload(path);
          bytes = Uint8Array.from(atob(r.data), c => c.charCodeAt(0));
        }
        dlProgress = 95;
      } else {
        // wss:// fallback path: we got base64 over WS RPC. No progress to
        // report mid-decode; jump straight to "fetched".
        bytes = Uint8Array.from(atob(dlInfo.base64), c => c.charCodeAt(0));
        dlProgress = 95;
      }
      window.__dbg?.(`dl: fetched ${(bytes.length/1024|0)}KB in ${Date.now()-t0}ms`);

      // Write phase. dlProgress runs 95→100 as the write completes.
      if (isTauri && tauriFs) {
        await tauriPlugins;
        if (isAndroid) {
          // Tauri 2's invoke supports Vec<u8> natively over its binary IPC
          // channel. Earlier we transcoded bytes → base64 → JSON IPC →
          // Rust base64-decode, which dominated download time on Android
          // (FileReader.readAsDataURL is a main-thread allocation of 4n/3
          // bytes in addition to the n raw bytes the response already used).
          dlProgress = 96;
          const { invoke } = await import('@tauri-apps/api/core');
          const filePath = await invoke('save_to_downloads', { name, data: bytes });
          window.__dbg?.(`dl: saved → ${filePath}`);
          dlProgress = 100;
          await new Promise(r => setTimeout(r, 300));
          downloading = '';
          downloadedPath = filePath;
          downloadToast = filePath;
          setTimeout(() => { if (downloadToast === filePath) dismissDownload(); }, 10000);
          return;
        }
        // macOS / desktop: prompt for save location.
        const savePath = await tauriDialog.save({ defaultPath: name });
        if (!savePath) { downloading = ''; dlProgress = 0; return; }
        dlProgress = 96;
        await tauriFs.writeFile(savePath, bytes);
        window.__dbg?.(`dl: saved → ${savePath}`);
        dlProgress = 100;
        await new Promise(r => setTimeout(r, 300));
        downloading = '';
        downloadedPath = String(savePath);
        downloadToast = downloadedPath;
        setTimeout(() => { if (downloadToast === downloadedPath) dismissDownload(); }, 10000);
        return;
      }
      // Plain browser: trigger a tag-based download.
      const blob = new Blob([bytes]);
      const blobUrl = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = blobUrl; a.download = name;
      document.body.appendChild(a);
      a.click();
      setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(blobUrl); }, 100);
      dlProgress = 100;
      await new Promise(r => setTimeout(r, 300));
      downloading = '';
      downloadToast = 'Downloaded';
      setTimeout(() => downloadToast = '', 2000);
    } catch (e) {
      downloading = ''; dlProgress = 0;
      window.__dbg?.(`dl: FAILED ${e.message}`);
      error = e.message;
    }
  }

  // ONE destination rule and ONE byte→b64 encoder for every upload entry point
  // (toolbar picker and drag-drop), so they cannot disagree about where a file
  // lands or how it travels. The DIR is a parameter, never a live read: a
  // batch SNAPSHOTS its target at the gesture (drop / picker confirm), so
  // navigating away mid-upload cannot re-route the rest of the files.
  const uploadDest = (dir, name) => dir.replace(/\/$/, '') + '/' + name;

  // After a batch: refresh ONLY if the user is still looking at the target.
  // Reloading the snapshot dir after they navigated away would hijack their
  // view back; refreshing their NEW dir would announce files that landed
  // elsewhere. Still there → show the arrivals; moved on → touch nothing.
  const refreshAfterBatch = (dir) => { if (cwd === dir) loadDir(dir); };

  function bytesToB64(bytes) {
    let binary = '';
    for (let i = 0; i < bytes.length; i += 8192) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
    }
    return btoa(binary);
  }

  // Browser File objects (the picker's input, a drop's DataTransfer). A
  // per-file try/catch: one unreadable item (a dropped DIRECTORY reads as a
  // File whose FileReader errors) must not abandon the rest of the batch.
  async function uploadBlobFiles(files) {
    const dir = cwd; // the batch's target, fixed at the gesture
    for (const file of files) {
      try {
        const b64 = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result).split(',')[1]);
          reader.onerror = () => reject(new Error(`cannot read: ${file.name}`));
          reader.readAsDataURL(file);
        });
        await fsUpload(uploadDest(dir, file.name), b64);
      } catch (e) { error = e.message; }
    }
    refreshAfterBatch(dir);
  }

  // Tauri filesystem paths (the native picker, the webview's drag-drop event).
  async function uploadTauriPaths(paths) {
    const dir = cwd; // the batch's target, fixed at the gesture
    await tauriPlugins;
    for (const filePath of paths) {
      try {
        const name = String(filePath).split('/').pop().split('\\').pop();
        const bytes = new Uint8Array(await tauriFs.readFile(filePath));
        await fsUpload(uploadDest(dir, name), bytesToB64(bytes));
      } catch (e) { error = e.message; }
    }
    refreshAfterBatch(dir);
  }

  async function handleUpload() {
    if (isTauri) {
      await tauriPlugins;
      const selected = await tauriDialog.open({ multiple: true });
      if (!selected) return;
      await uploadTauriPaths(Array.isArray(selected) ? selected : [selected]);
      return;
    }
    // Browser fallback
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = true;
    document.body.appendChild(input);
    input.onchange = async () => {
      await uploadBlobFiles(Array.from(input.files || []));
      document.body.removeChild(input);
    };
    input.click();
  }

  // ── Drag a file in from OUTSIDE, drop it on the listing, it uploads to the
  //    current directory (board #22). Two transports for one gesture:
  //    · Browser: the HTML5 events on the listing itself.
  //    · Compiled app: the webview INTERCEPTS native drags (Tauri's default
  //      dragDropEnabled), so DataTransfer never carries files there — the
  //      drop arrives as the webview's own drag-drop event with fs PATHS, and
  //      a hit-test against the listing's rect stands in for event targeting.
  let dragOver = $state(false);
  let fileListEl = $state(null);

  const dragHasFiles = (e) => Array.from(e.dataTransfer?.types || []).includes('Files');
  function onListDragOver(e) {
    if (!dragHasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    dragOver = true;
  }
  function onListDragLeave(e) {
    // dragleave also fires when the cursor enters a CHILD row; only a real
    // exit (relatedTarget outside the listing) parks the highlight.
    if (e.currentTarget.contains(e.relatedTarget)) return;
    dragOver = false;
  }
  async function onListDrop(e) {
    if (!dragHasFiles(e)) return;
    e.preventDefault();
    dragOver = false;
    const files = Array.from(e.dataTransfer.files || []);
    if (files.length) await uploadBlobFiles(files);
  }

  // A missed drop must not NAVIGATE the tab to the file (which tears down the
  // whole app, socket included). While Files is visible in a browser, stray
  // drags over the window are neutralized; the listing's own handlers still
  // run first in the bubble.
  $effect(() => {
    if (!visible || isTauri) return;
    const block = (e) => { if (dragHasFiles(e)) e.preventDefault(); };
    window.addEventListener('dragover', block);
    window.addEventListener('drop', block);
    return () => {
      window.removeEventListener('dragover', block);
      window.removeEventListener('drop', block);
    };
  });

  // The compiled app's half: the listener EXISTS only while this instance is
  // visible — that is the real gate against the parked page-layer twin (App's
  // hidden layer keeps LAYOUT under visibility:hidden, and bare
  // checkVisibility() does NOT check the visibility property — its
  // visibilityProperty option defaults false — so a rect test alone, or a
  // bare checkVisibility, would still let the hidden instance double-claim a
  // drop). The in-test visibility check stays as defense in depth, with the
  // option set and a computed-style fallback for engines without the API.
  function listHit(pos) {
    const el = fileListEl;
    if (!el) return false;
    const shown = el.checkVisibility
      ? el.checkVisibility({ visibilityProperty: true, checkVisibilityCSS: true })
      : getComputedStyle(el).visibility !== 'hidden';
    if (!shown) return false;
    // The webview reports PHYSICAL pixels; client rects are CSS pixels.
    const dpr = window.devicePixelRatio || 1;
    const x = pos.x / dpr, y = pos.y / dpr;
    const r = el.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }
  $effect(() => {
    if (!isTauri || !visible) return;
    let unlisten = null, dead = false;
    (async () => {
      await tauriPlugins;
      const un = await tauriWebview.getCurrentWebview().onDragDropEvent((ev) => {
        const t = ev.payload.type;
        if (t === 'leave') { dragOver = false; return; }
        const hit = listHit(ev.payload.position);
        if (t === 'enter' || t === 'over') dragOver = hit;
        else if (t === 'drop') {
          dragOver = false;
          if (hit && ev.payload.paths?.length) uploadTauriPaths(ev.payload.paths);
        }
      });
      if (dead) un(); else unlisten = un;
    })();
    return () => { dead = true; dragOver = false; unlisten?.(); };
  });

  let copyToast = $state(false);
  let copyTimer;
  async function copyPath(path) {
    await copyText(path);
    clearTimeout(copyTimer);
    copyToast = true;
    copyTimer = setTimeout(() => copyToast = false, 1200);
  }

  function formatSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB';
  }

  function formatDate(ts) {
    if (!ts) return '';
    return new Date(ts * 1000).toLocaleString();
  }

  // The row's facts for the hover card (motion.md principle 16), through the
  // SAME formatters the info view uses — never a second one.
  function entryKind(entry) {
    if (entry.type === 'broken') return t('kindBroken');
    const target = entry.type === 'dir' ? t('kindDir') : t('kindFile');
    return entry.is_symlink ? `${t('kindSymlink')} → ${entry.link_target || target}` : target;
  }
  function entryInfo(entry) {
    const lines = [{ label: t('type'), value: entryKind(entry) }];
    if (entry.type !== 'dir') lines.push({ label: t('size'), value: formatSize(entry.size) });
    if (entry.modified) lines.push({ label: t('modified'), value: formatDate(entry.modified) });
    return { title: entry.name, lines };
  }

  function fileIcon(entry) {
    if (entry.type === 'dir') return 'folder';
    return 'file';
  }

  function mimeCategory(mime) {
    if (!mime) return 'other';
    if (mime.startsWith('image/')) return 'image';
    if (mime === 'text/markdown') return 'markdown';
    if (mime === 'text/csv') return 'csv';
    if (mime === 'text/html') return 'html';
    if (mime === 'application/pdf') return 'pdf';
    if (mime.startsWith('text/') || mime === 'application/json' || mime === 'application/toml' || mime === 'application/yaml') return 'code';
    return 'other';
  }

  function hljsLang(mime) {
    const map = {
      'text/javascript': 'js', 'text/typescript': 'ts', 'text/python': 'python',
      'text/rust': 'rust', 'text/css': 'css', 'text/shell': 'bash', 'text/sql': 'sql',
      'text/go': 'go', 'text/java': 'java', 'text/ruby': 'ruby', 'text/c': 'c',
      'text/cpp': 'cpp', 'text/svelte': 'html', 'text/vue': 'html',
      'application/json': 'json', 'application/toml': 'yaml', 'application/yaml': 'yaml',
    };
    return map[mime] || null;
  }

  // `hljs` is null until loadHljs() resolves; every highlighter degrades to
  // the escaped source in the meantime and the $state flip re-renders it.
  function highlightCode(text, mime) {
    if (text == null) return '';
    if (!hljs) return text.replace(/&/g, '&amp;').replace(/</g, '&lt;');
    const lang = hljsLang(mime);
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(text, { language: lang }).value;
    }
    try { return hljs.highlightAuto(text).value; } catch { return text.replace(/&/g, '&amp;').replace(/</g, '&lt;'); }
  }

  // Highlight a SINGLE line for the per-line code views (preview + the gutter
  // alignment fix). Per-line so each logical line is its own DOM row — the line
  // number then sits at the top of that row and stays aligned even when the
  // line soft-wraps. We skip highlightAuto here (too slow per-line and it would
  // guess a different language each line); without a known language we just
  // escape. Cross-line constructs (block comments, multiline strings) lose
  // their context, an acceptable trade for reliable alignment + wrapping.
  function highlightLine(line, mime) {
    if (!line) return '';
    const lang = hljs ? hljsLang(mime) : null;
    if (lang && hljs.getLanguage(lang)) {
      try { return hljs.highlight(line, { language: lang }).value; } catch {}
    }
    return line.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  }

  // The lined code preview renders one DOM row per line with a per-line hljs
  // call; a 512 KB log is 20k rows and a multi-second main-thread freeze on a
  // phone. Show the head and let the reader ask for the rest.
  const CODE_PREVIEW_MAX_LINES = 3000;
  let showAllLines = $state(false);
  let previewLines = $derived((currentFile?.content ?? '').split('\n'));
  let shownLines = $derived(showAllLines || previewLines.length <= CODE_PREVIEW_MAX_LINES
    ? previewLines
    : previewLines.slice(0, CODE_PREVIEW_MAX_LINES));

  let mermaidId = 0;
  async function renderMermaidBlocks(container) {
    if (!container) return;
    const blocks = container.querySelectorAll('code.language-mermaid');
    if (!blocks.length) return; // most markdown has no diagram: never load mermaid for it
    const mermaid = await loadMermaid();
    if (!mermaid) return;
    for (const block of blocks) {
      const pre = block.parentElement;
      const id = `mermaid-${++mermaidId}`;
      const div = document.createElement('div');
      div.className = 'mermaid-block';
      try {
        const { svg } = await mermaid.render(id, block.textContent);
        div.innerHTML = svg;
      } catch { div.textContent = block.textContent; }
      pre.replaceWith(div);
    }
  }

  let previewEl = $state(null);
  let htmlPreviewEl = $state(null);
  let removeHtmlPreviewLinks = () => {};

  function attachHtmlPreviewLinks() {
    removeHtmlPreviewLinks();
    removeHtmlPreviewLinks = installExternalLinkHandler(htmlPreviewEl?.contentDocument);
  }

  $effect(() => () => removeHtmlPreviewLinks());

  function mimeFromName(name) {
    const ext = name.split('.').pop()?.toLowerCase();
    const map = { png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', svg: 'image/svg+xml', webp: 'image/webp', bmp: 'image/bmp', ico: 'image/x-icon', avif: 'image/avif' };
    return map[ext] || 'image/png';
  }

  async function resolveImages(container) {
    if (!container) return;
    const dir = currentFile?.path?.replace(/\/[^/]+$/, '') || '';
    const imgs = container.querySelectorAll('img[src]');
    for (const img of imgs) {
      const src = img.getAttribute('src');
      if (!src || src.startsWith('data:') || src.startsWith('http')) continue;
      // Resolve relative path
      const fullPath = src.startsWith('/') ? src : dir + '/' + src;
      try {
        const r = await fsDownload(fullPath);
        const mime = mimeFromName(r.name || src);
        img.src = `data:${mime};base64,${r.data}`;
      } catch { img.alt = `[${src}]`; }
    }
  }

  $effect(() => {
    if (view === 'preview' && mimeCategory(currentFile?.stat?.mime_hint) === 'markdown' && previewEl) {
      setTimeout(() => { renderMermaidBlocks(previewEl); resolveImages(previewEl); }, 50);
    }
    if (view === 'preview' && currentFile?.pdfData && pdfContainer) {
      setTimeout(() => renderPdf(currentFile.pdfData), 50);
    }
  });

  function renderCsv(text) {
    if (text == null) return '';
    const lines = text.trim().split('\n');
    if (!lines.length) return '';
    const rows = lines.map(l => l.split(',').map(c => c.trim().replace(/^"|"$/g, '')));
    let html = '<table><thead><tr>';
    rows[0].forEach(h => html += `<th>${h.replace(/</g,'&lt;')}</th>`);
    html += '</tr></thead><tbody>';
    rows.slice(1).forEach(r => {
      html += '<tr>';
      r.forEach(c => html += `<td>${c.replace(/</g,'&lt;')}</td>`);
      html += '</tr>';
    });
    return html + '</tbody></table>';
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- View blocks live in snippets so both layouts (mobile single-pane + desktop
     two-pane) render the same markup. -->
{#snippet listPanel()}
    <!-- Toolbar: all buttons in one row -->
    <div class="toolbar">
      <button class="tool-btn" onclick={goSessionDir} title={t('filesSessionDir')} aria-label={t('filesSessionDir')}><Icon name="terminal" size={13} /></button>
      <button class="tool-btn" onclick={() => loadDir(cwd)} aria-label="Refresh"><Icon name="refresh" size={13} /></button>
      <button class="tool-btn" onclick={() => { newType = newType ? '' : 'file'; newName = ''; }}><Icon name="plus" size={13} /></button>
      <button class="tool-btn" onclick={handleUpload}><Icon name="upload" size={13} /></button>
      <button class="tool-btn" class:tool-active={showHidden} onclick={() => { showHidden = !showHidden; loadDir(cwd); }}>
        <Icon name="eye" size={13} />
      </button>
      <button class="tool-btn" class:starred={isBookmarked(cwd)} onclick={() => toggleBookmark(cwd)} title="Bookmark">
        {#key isBookmarked(cwd)}<span class="tool-glyph appear-pop"><Icon name={isBookmarked(cwd) ? 'star-filled' : 'star'} size={13} /></span>{/key}
      </button>
      <button class="tool-btn" class:tool-active={showBookmarks} onclick={() => { showBookmarks = !showBookmarks; showRecent = false; }} title="Bookmarks">
        <Icon name="folder-star" size={13} />
      </button>
      <button class="tool-btn" class:tool-active={showRecent} onclick={() => { showRecent = !showRecent; showBookmarks = false; }} title="Recent">
        <Icon name="clock" size={13} />
      </button>
      {#if hasGit}
        <button class="tool-btn" onclick={openGitView} title="Git">
          <Icon name="git-branch" size={13} />
        </button>
      {/if}
      <div style="flex:1"></div>
      {#if isTauri}
        <button class="tool-btn" onclick={openLocalFiles} title="Local files"><Icon name="download" size={13} /></button>
      {/if}
    </div>

    <!-- Path -->
    <div class="bc-path-row" bind:this={bcPathEl}>
      <button class="bc-seg" onclick={() => navTo('/')}>/</button>
      {#each breadcrumbs as bc, i (bc.path)}
        <button class="bc-seg" class:appear={i === breadcrumbs.length - 1} onclick={() => navTo(bc.path)}
          use:hoverInfo={() => ({ title: bc.name, text: bc.path })}>{bc.name}</button>
        <span class="bc-sep">/</span>
      {/each}
    </div>

    {#if showBookmarks && bookmarks.length}
      <div class="bookmarks-panel appear-rise">
        {#each bookmarks as bm}
          <div class="bm-row">
            <span class="bm-icon"><Icon name="star-filled" size={13} /></span>
            <button class="bm-path" onclick={() => { navTo(bm); showBookmarks = false; }} use:scrollEnd use:hoverInfo={() => ({ text: bm })}>
              {bm}
            </button>
            <button class="bm-del" onclick={() => toggleBookmark(bm)}><Icon name="x" size={12} /></button>
          </div>
        {/each}
      </div>
    {/if}

    {#if showRecent && recentFiles.length}
      <div class="bookmarks-panel appear-rise">
        {#each recentFiles as rf}
          <div class="bm-row">
            <span class="bm-icon"><Icon name="clock" size={13} /></span>
            <button class="bm-path" onclick={() => { showRecent = false; openEntry({ type: 'file', path: rf.path, name: rf.name }); }} use:scrollEnd
              use:hoverInfo={() => ({ title: rf.name, text: rf.path })}>
              <span style="color:var(--text3);font-size:var(--fs-meta)">{rf.path.replace(/\/[^/]+$/, '')}/</span>{rf.name}
            </button>
            <button class="bm-del" onclick={() => { recentFiles = recentFiles.filter(f => f.path !== rf.path); setPref('recentFiles', recentFiles).catch(() => {}); }}><Icon name="x" size={12} /></button>
          </div>
        {/each}
      </div>
    {/if}

    <!-- New item input -->
    {#if newType}
      <div class="new-item appear-rise">
        <button class="new-type-btn" onclick={() => newType = newType === 'file' ? 'dir' : 'file'}>
          <Icon name={newType === 'dir' ? 'folder' : 'file'} size={13} />
        </button>
        <input
          type="text"
          bind:value={newName}
          placeholder={newType === 'dir' ? t('folderName') : t('fileName')}
          onkeydown={(e) => e.key === 'Enter' && !e.isComposing && e.keyCode !== 229 && handleNewItem()}
          autocapitalize="off"
          autocomplete="off"
        />
        <button onclick={handleNewItem}><Icon name="plus" size={12} /></button>
        <button onclick={() => newType = ''}><Icon name="x" size={12} /></button>
      </div>
    {/if}

    <!-- Rename input -->
    {#if renaming}
      <div class="new-item appear-rise">
        <input
          type="text"
          bind:value={renameValue}
          placeholder={t('newName')}
          onkeydown={(e) => e.key === 'Enter' && !e.isComposing && e.keyCode !== 229 && handleRename()}
          autocapitalize="off"
        />
        <button onclick={handleRename}><Icon name="edit" size={12} /></button>
        <button onclick={() => renaming = null}><Icon name="x" size={12} /></button>
      </div>
    {/if}

    {#if error}
      <div class="error appear">{error}</div>
    {/if}

    <!-- File list. Also the drop target for OS files (board #22): the browser
         path via the HTML5 events here, the compiled app via the webview's
         drag-drop event hit-testing this element's rect. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="file-list" class:panel-open={showBookmarks || showRecent} class:drop-hot={dragOver} class:busy={loading} class:reveal={!!revealDir}
      bind:this={fileListEl}
      ondragover={onListDragOver} ondragleave={onListDragLeave} ondrop={onListDrop}>
      {#if dragOver}
        <div class="drop-hint appear"><Icon name="upload" size={16} />{t('dropToUpload')}</div>
      {/if}
      <!-- A directory load KEEPS the rows on screen and dims them after a beat
           (DirPicker's rule, motion.md): swapping them for "Loading…" made
           every tap blank-then-repaint. The placeholder is for the very first
           answer only, when there is nothing to keep. -->
      {#if loading && !entries.length}
        <div class="loading">{t('loading')}</div>
      {:else}
        {#each entries as entry (entry.path)}
          <div class="file-row" class:broken={entry.type === 'broken'}>
            <button class="file-main" onclick={() => openEntry(entry)} use:hoverInfo={() => entryInfo(entry)}>
              <span class="file-icon" class:is-link={entry.is_symlink}>
                <Icon name={fileIcon(entry)} size={16} />
              </span>
              <span
                class="file-name"
                class:dir-name={entry.type === 'dir'}
                class:link-name={entry.is_symlink}
              >{entry.name}</span>
              {#if entry.type !== 'dir'}
                <span class="file-size">{formatSize(entry.size)}</span>
              {/if}
            </button>
            <div class="file-actions">
              {#if entry.type !== 'dir' && entry.type !== 'broken'}
                <button class="act-btn" onclick={() => handleDownload(entry.path)} title="Download"><Icon name="download" size={12} /></button>
              {/if}
              <button class="act-btn" onclick={() => { renaming = entry.path; renameValue = entry.name; }} title="Rename"><Icon name="edit" size={12} /></button>
              <button class="act-btn del" onclick={() => (pendingAct = { kind: 'file', path: entry.path })} title="Delete">
                <Icon name="trash" size={12} />
              </button>
            </div>
          </div>
        {/each}
        {#if !entries.length && !loading}
          <div class="empty">{t('emptyDir')}</div>
        {/if}
      {/if}
    </div>
{/snippet}

{#snippet previewPanel()}
    <!-- File preview -->
    <div class="preview-header">
      <button class="back-btn" onclick={backToList}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile.name}</span>
      <div class="preview-actions">
        {#if isLinedPreview}
          <button class="act-btn" class:on={wrapLines} onclick={() => wrapLines = !wrapLines} title={wrapLines ? t('editorNoWrap') : t('editorWrap')}><Icon name={wrapLines ? 'wrap-text' : 'no-wrap'} size={14} /></button>
        {/if}
        {#if currentFile.stat?.is_text && currentFile.stat?.writable}
          <button class="act-btn" onclick={startEdit}><Icon name="edit" size={14} /></button>
        {/if}
        <button class="act-btn" onclick={() => handleDownload(currentFile.path)}><Icon name="download" size={14} /></button>
        <button class="act-btn" onclick={reloadPreview}><Icon name="refresh" size={14} /></button>
        <button class="act-btn" onclick={() => { view = 'info'; navPush(); }}><Icon name="info" size={14} /></button>
      </div>
    </div>
    <div class="preview-body" style="--file-font-size:{fontSize}px">
      {#if mimeCategory(currentFile.stat?.mime_hint) === 'markdown'}
        <div class="md-render" bind:this={previewEl}>{@html renderMarkdown(currentFile.content)}</div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'csv'}
        <div class="csv-render">{@html renderCsv(currentFile.content)}</div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'html'}
        <iframe
          class="html-preview"
          bind:this={htmlPreviewEl}
          srcdoc={currentFile.content}
          sandbox="allow-same-origin"
          title="HTML Preview"
          onload={attachHtmlPreviewLinks}
        ></iframe>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'pdf'}
        <div class="pdf-container" bind:this={pdfContainer} style="margin: -12px; padding: 0;"></div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'image'}
        <div class="image-preview"><img src={currentFile.dataUrl} alt={currentFile.name} /></div>
      {:else if currentFile.convertedHtml}
        <div class="md-render">{@html currentFile.convertedHtml}</div>
      {:else}
        <!-- 'code' and every other text: one lined view, capped (see shownLines) -->
        <div class="code-lined" class:wrap={wrapLines}>
          {#each shownLines as line, i}
            <div class="cl-row"><span class="cl-num">{i + 1}</span><code class="cl-code">{@html highlightLine(line, currentFile.stat?.mime_hint) || '\u200b'}</code></div>
          {/each}
          {#if shownLines.length < previewLines.length}
            <button class="cl-more" onclick={() => { showAllLines = true; }}>
              {t('previewShowAllLines').replace('{n}', String(previewLines.length))}
            </button>
          {/if}
        </div>
      {/if}
    </div>
{/snippet}

{#snippet editPanel()}
    <!-- File editor -->
    <div class="preview-header">
      <button class="back-btn" onclick={backToPreview}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile.name}{isEdited ? ' *' : ''}</span>
      <div class="preview-actions">
        <button class="act-btn" class:on={wrapLines} onclick={() => { wrapLines = !wrapLines; requestAnimationFrame(syncEditorScroll); }} title={wrapLines ? t('editorNoWrap') : t('editorWrap')}><Icon name={wrapLines ? 'wrap-text' : 'no-wrap'} size={14} /></button>
        <button class="act-btn" onclick={undo} disabled={!undoStack.length && editContent === editOriginal}><Icon name="undo" size={14} /></button>
        <button class="act-btn save" onclick={saveFile} disabled={!isEdited}><Icon name="save" size={14} /></button>
      </div>
    </div>
    <div class="editor-wrap" class:wrap={wrapLines} style="--file-font-size:{fontSize}px">
      <div class="editor-nums" bind:this={numsEl}>
        {#each lineHeights as h, i}
          <div class="eln" style={h ? `height:${h}px` : ''}>{i + 1}</div>
        {/each}
      </div>
      <div class="editor-layer" bind:this={layerEl}>
        <pre class="editor-highlight" bind:this={hlEl} aria-hidden="true"><code>{@html highlightCode(editContent, currentFile?.stat?.mime_hint)}</code>{'\n'}</pre>
        <!-- Off-screen mirror: one block per logical line, same width/font/wrap
             as the highlight layer, so each block's measured height tells the
             gutter how tall to make that line number — keeping numbers aligned
             even when a line soft-wraps. -->
        <div class="editor-mirror" bind:this={mirrorEl} aria-hidden="true">
          {#each editContent.split('\n') as line}
            <div class="emir">{line || '\u200b'}</div>
          {/each}
        </div>
        <textarea
          class="editor"
          bind:this={taEl}
          value={editContent}
          oninput={onEditInput}
          onscroll={syncEditorScroll}
          spellcheck="false"
          autocapitalize="off"
          autocomplete="off"
        ></textarea>
      </div>
    </div>
{/snippet}

{#snippet localPanel()}
    <!-- Local downloaded files -->
    <div class="preview-header">
      <button class="back-btn" onclick={() => { navAnim('back'); view = 'list'; }}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{t('downloads')}</span>
      <div class="preview-actions">
        <button class="act-btn" onclick={openLocalFiles}><Icon name="refresh" size={14} /></button>
      </div>
    </div>
    <div class="file-list">
      {#each localFiles as f}
        <div class="file-row">
          <button class="file-main" onclick={() => openLocalFile(f.name)}>
            <Icon name="file" size={16} />
            <span class="file-name">{f.name}</span>
          </button>
          <button class="act-btn del" onclick={() => (pendingAct = { kind: 'local', name: f.name })}><Icon name="trash" size={12} /></button>
        </div>
      {/each}
      {#if !localFiles.length}
        <div class="empty">{t('noDownloads')}</div>
      {/if}
    </div>
{/snippet}

{#snippet infoPanel()}
    <!-- File info -->
    <div class="preview-header">
      <button class="back-btn" onclick={() => { navAnim('back'); view = currentFile?.content != null ? 'preview' : 'list'; }}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile?.name}</span>
      <div class="preview-actions">
        {#if isPreviewable(currentFile?.stat, currentFile?.name)}
          <button class="act-btn" onclick={() => loadPreviewContent(currentFile)} title={t('preview')}><Icon name="eye" size={14} /></button>
        {/if}
        <button class="act-btn" onclick={() => handleDownload(currentFile.path)}><Icon name="download" size={14} /></button>
        <button class="act-btn" onclick={() => copyPath(currentFile.path)}><Icon name="copy" size={14} /></button>
      </div>
    </div>
    <div class="info-body">
      {#if currentFile?.stat}
        <div class="info-row"><span class="info-label">{t('path')}</span><button class="info-path" onclick={() => copyPath(currentFile.stat.path)}>{currentFile.stat.path}</button></div>
        <div class="info-row"><span class="info-label">{t('type')}</span><span class="info-val">{currentFile.stat.mime_hint}</span></div>
        <div class="info-row"><span class="info-label">{t('size')}</span><span class="info-val">{formatSize(currentFile.stat.size)}</span></div>
        <div class="info-row"><span class="info-label">{t('modified')}</span><span class="info-val">{formatDate(currentFile.stat.modified)}</span></div>
        <div class="info-row"><span class="info-label">{t('permissions')}</span><span class="info-val mono">{currentFile.stat.permissions}</span></div>
        <div class="info-row"><span class="info-label">{t('readable')}</span><span class="info-val">{currentFile.stat.readable ? t('yes') : t('no')}</span></div>
        <div class="info-row"><span class="info-label">{t('writable')}</span><span class="info-val">{currentFile.stat.writable ? t('yes') : t('no')}</span></div>
        <div class="info-row"><span class="info-label">{t('textFile')}</span><span class="info-val">{currentFile.stat.is_text ? t('yes') : t('no')}</span></div>
      {/if}
    </div>
{/snippet}


<!-- Touch handlers implement the edge-swipe back gesture; not an interactive element. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="files" bind:this={filesEl}
  class:snap={swipeSnap} class:drill-fwd={navAnimClass === 'fwd'} class:drill-back={navAnimClass === 'back'}
  style:transform={swipeDX > 0 ? `translateX(${swipeDX}px)` : ''}
  ontouchstart={onTouchStart} ontouchmove={onTouchMove} ontouchend={onTouchEnd} ontouchcancel={onTouchCancel}>
  {#if splitEligible}
    <!-- Desktop: folder browser (left) | draggable splitter | preview (right). -->
    <div class="files-split">
      <!-- The directory browser lives in THE shared sidebar
           (ui-unification.md): same width var, same handle as Hub/Agents. -->
      <div class="files-left">
        <SideHandle />
        {@render listPanel()}
      </div>
      <div class="files-right">
        {#if view === 'preview'}{@render previewPanel()}
        {:else if view === 'edit'}{@render editPanel()}
        {:else if view === 'info'}{@render infoPanel()}
        {:else if view === 'git'}<GitPanel bind:this={gitPanelRef} {cwd} {fontSize} onOpenFile={(entry) => { fromGit = true; openEntry(entry); }} onClose={() => { view = 'list'; }} />
        {:else if view === 'local'}{@render localPanel()}
        {:else}
          <div class="files-placeholder"><Icon name="file" size={40} /><p>{t('selectFile')}</p></div>
        {/if}
      </div>
    </div>
  {:else}
    <!-- Mobile / narrow / touch: single-pane view chain (unchanged). -->
    {#if view === 'list'}{@render listPanel()}
    {:else if view === 'preview'}{@render previewPanel()}
    {:else if view === 'edit'}{@render editPanel()}
    {:else if view === 'local'}{@render localPanel()}
    {:else if view === 'info'}{@render infoPanel()}
    {:else if view === 'git'}<GitPanel bind:this={gitPanelRef} {cwd} {fontSize} onOpenFile={(entry) => { fromGit = true; openEntry(entry); }} onClose={() => { view = 'list'; }} />
    {/if}
  {/if}
  {#if copyToast}
    <div class="copy-toast flash">{t('copied')}</div>
  {/if}
  {#if downloading}
    <div class="copy-toast download-toast appear-rise">
      <svg class="dl-ring" width="28" height="28" viewBox="0 0 28 28">
        <circle cx="14" cy="14" r="11" fill="none" stroke="var(--border)" stroke-width="2.5" />
        <circle cx="14" cy="14" r="11" fill="none" stroke="var(--accent)" stroke-width="2.5"
          transform="rotate(-90 14 14)"
          stroke-dasharray={2 * Math.PI * 11}
          stroke-dashoffset={2 * Math.PI * 11 * (1 - displayedDlProgress / 100)} />
      </svg>
      <span class="dl-pct">{displayedDlProgress}%</span>
      <span class="dl-name">{downloading}</span>
    </div>
  {:else if downloadToast}
    <div class="copy-toast download-toast appear-rise">
      {t('saved')} <span class="dl-path">{downloadToast}</span>
      {#if downloadedPath}
        <button class="toast-open" onclick={openDownloaded}>{t('open')}</button>
      {/if}
      <button class="toast-close" onclick={dismissDownload}><Icon name="x" size={12} /></button>
    </div>
  {/if}
</div>

<ConfirmDialog open={!!pendingAct} busy={acting} compact={narrowViewport}
  title={pendingAct ? t(ACT_COPY[pendingAct.kind].title).replace('{name}', actName(pendingAct)) : ''}
  note={pendingAct ? t(ACT_COPY[pendingAct.kind].note) : ''}
  confirmLabel={pendingAct ? t(ACT_COPY[pendingAct.kind].go) : ''}
  onconfirm={runPendingAct} oncancel={() => (pendingAct = null)} />

<style>
  .files { display: flex; flex-direction: column; flex: 1; min-height: 0; background: var(--bg); }
  /* Interactive edge-swipe: the drag itself sets an inline translate (no
     transition — the finger is the animation); releasing under the commit
     threshold springs back on --t-fast; a committed back plays the shared
     drill grammar (120ms, from the left; deeper from the right). Transforms
     exist only during these beats, so .files is never a resting containing
     block (design-language.md §2). */
  .files.snap { transition: transform var(--t-fast); }
  .files.drill-fwd  { animation: drill-in-right 0.12s linear; }
  .files.drill-back { animation: drill-in-left 0.12s linear; }
  @media (prefers-reduced-motion: reduce) {
    .files.drill-fwd, .files.drill-back { animation: none; }
    .files.snap { transition: none; }
  }

  /* Desktop two-pane: folder browser | splitter | preview. The columns must be
     flex-column themselves so each panel's sticky header (flex-shrink:0) +
     scrollable body (flex:1; overflow) constrain correctly — `.files` gave them
     that in single-pane mode; here the columns must. */
  .files-split { display: flex; flex: 1; min-height: 0; width: 100%; }
  .files-left, .files-right {
    display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden;
  }
  /* The directory browser is THE shared sidebar: same width var, same bg2,
     same handle as Hub/Agents (ui-unification.md). The old per-page flex
     fraction (tmux_files_frac) is gone with its private splitter. */
  .files-left { position: relative; flex: none; width: var(--sidebar-w); background: var(--bg2); }
  .files-right { flex: 1; border-left: 1px solid var(--border); }
  .files-placeholder {
    flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 10px; color: var(--text3); font-size: var(--fs-body); padding: 24px; text-align: center;
  }

  /* Toolbar — same vertical rhythm as the Terminal window-switcher bar
     and the Sessions top-row (24 px buttons + 3 px padding = ~31 px total). */
  /* Page-skeleton alignment (ui-unification.md): same bar geometry and
     border as the Hub page header; the dense controls inside stay. */
  .toolbar {
    display: flex; align-items: center; gap: var(--ui-gap); min-height: 42px; padding: 6px 10px; box-sizing: border-box;
    border-bottom: 1px solid var(--border); background: transparent; flex-shrink: 0;
  }
  .tool-btn {
    /* Borderless icon action — the .icon-btn/rail grammar (owner, 2026-08-28),
       at this toolbar's touch size. */
    width: var(--ui-control-height); height: var(--ui-control-height);
    padding: 0; border: none; border-radius: var(--ui-radius-pill);
    background: none; color: var(--text2); cursor: pointer;
    font-size: var(--ui-font-control); display: flex; align-items: center; justify-content: center;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
    transition: background var(--t-fast), color var(--t-fast);
  }
  .tool-glyph { display: flex; }
  .tool-btn:active { background: var(--accent-bg); color: var(--accent); }
  .tool-btn.tool-active { background: var(--accent-bg); color: var(--accent); }
  .tool-btn.starred { color: var(--accent); }

  /* Path row */
  .bc-path-row {
    display: flex; align-items: center; gap: 1px; padding: 4px 10px;
    overflow-x: auto; font-size: var(--fs-ui); font-family: var(--font-mono);
    scrollbar-width: none; border-bottom: 1px solid var(--border2); flex-shrink: 0;
  }
  .bc-path-row::-webkit-scrollbar { display: none; }
  .bc-seg {
    padding: 2px 4px; border: none; background: none; color: var(--text2);
    cursor: pointer; white-space: nowrap; font-size: var(--fs-ui); font-family: inherit;
    transition: color var(--t-fast);
  }
  .bc-seg:last-of-type { color: var(--accent); }
  .bc-sep { color: var(--text3); font-size: var(--fs-sub); }

  /* Bookmarks / Recent panel */
  .bookmarks-panel {
    border-bottom: 1px solid var(--border2); flex-shrink: 0;
    max-height: calc(40vh / var(--ui-zoom, 1)); overflow-y: auto;
    -webkit-overflow-scrolling: touch; overscroll-behavior: contain;
  }
  .bm-row {
    display: flex; align-items: center; gap: 6px; padding: 0 10px;
    border-bottom: 1px solid var(--border2);
  }
  .bm-icon { color: var(--accent); display: flex; flex-shrink: 0; }
  .bm-path {
    flex: 1; display: block;
    padding: 8px 0; border: none; background: none; color: var(--text);
    font-size: var(--fs-ui); font-family: var(--font-mono);
    cursor: pointer; text-align: left; overflow-x: auto;
    white-space: nowrap; scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
    transition: color var(--t-fast);
  }
  .bm-path::-webkit-scrollbar { display: none; }
  .bm-path:active { color: var(--accent); }
  .bm-del {
    padding: 4px; border: none; border-radius: var(--ui-radius-control); background: none;
    color: var(--text3); cursor: pointer; display: flex;
    transition: color var(--t-fast);
  }
  .bm-del:active { color: var(--danger); }

  /* New item / rename */
  .new-item {
    display: flex; gap: 6px; padding: 6px 10px;
    border-bottom: 1px solid var(--border2); min-width: 0;
  }
  .new-item input {
    flex: 1; min-width: 0; padding: 6px 10px; border: 1px solid var(--input-border); border-radius: var(--ui-radius-control);
    background: var(--input-bg); color: var(--text); font-size: var(--fs-body);
    font-family: var(--font-mono);
  }
  .new-item button {
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: var(--ui-radius-control);
    background: var(--surface2); color: var(--text2); cursor: pointer;
  }
  .new-type-btn { display: flex; align-items: center; color: var(--accent); }

  .error {
    padding: 8px 12px; background: var(--bg2); color: var(--danger);
    font-size: var(--fs-ui); border-bottom: 1px solid var(--danger);
  }

  /* File list */
  .file-list {
    flex: 1; overflow-y: auto; -webkit-overflow-scrolling: touch; position: relative;
    transition: opacity var(--t-fast) ease;
  }
  /* In-flight cue that never flashes (DirPicker's rule): the dim starts only
     after 150ms, so a fast listing — the normal case — navigates with no
     visible blink. The delay is a threshold, not a tempo. */
  .file-list.busy { opacity: 0.55; transition-delay: 0.15s; }
  /* The drop target speaks the attach-affordance dialect (dashed accent, like
     the composer's +): a frame over the listing while an OS drag hovers it,
     pointer-events none so dragleave/drop still land on the list itself. */
  .drop-hint {
    position: absolute; inset: 6px; z-index: 5;
    display: flex; align-items: center; justify-content: center; gap: 8px;
    border: 2px dashed var(--accent-line); border-radius: var(--ui-radius-panel);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    color: var(--accent); font-size: var(--fs-ui); font-weight: 600;
    pointer-events: none;
  }
  /* When the bookmarks/recent panel is open, lock the file list so touch
     gestures on the panel can't bleed through and drag the list too. */
  .file-list.panel-open { overflow: hidden; touch-action: none; }
  .file-row {
    display: flex; align-items: center; border-bottom: 1px solid var(--border2);
  }
  .file-main {
    flex: 1; display: flex; align-items: center; gap: 10px; padding: 14px 12px;
    border: none; background: none; color: var(--text); cursor: pointer; text-align: left;
    font-size: var(--fs-body); min-width: 0; -webkit-tap-highlight-color: transparent;
    font-family: var(--font-ui); /* file names are data, not chrome */
    transition: background var(--t-fast);
  }
  .file-main:active { background: var(--input-bg); }
  /* Symlink badge — small ↗ arrow overlaid on the bottom-right of the
     file/folder icon. Renders for both symlink-to-dir and symlink-to-file
     so the link nature is visible without changing the base icon. */
  .file-icon {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    line-height: 0;
  }
  .file-icon.is-link::after {
    content: '↗';
    position: absolute;
    right: -4px;
    bottom: -4px;
    font-size: var(--fs-meta);
    line-height: 1;
    color: var(--accent);
    background: var(--bg);
    border-radius: 50%;
    padding: 1px 2px;
    pointer-events: none;
    font-weight: 700;
  }
  .file-row.broken { opacity: 0.55; }
  .file-row.broken .file-icon.is-link::after { color: var(--danger, #f87171); }
  .link-name { font-style: italic; }
  .file-name {
    flex: 1; min-width: 0; white-space: nowrap;
    overflow-x: auto; overflow-y: hidden;
    scrollbar-width: none; -ms-overflow-style: none;
    overscroll-behavior-x: contain;
  }
  .file-name::-webkit-scrollbar { display: none; }
  .dir-name { color: var(--accent); }
  .file-size { color: var(--text3); font-size: var(--fs-sub); font-family: var(--font-mono); white-space: nowrap; }
  .file-actions { display: flex; gap: 2px; padding-right: 8px; }
  .act-btn {
    padding: 6px; border: none; border-radius: var(--ui-radius-control); background: none;
    color: var(--text3); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
    transition: opacity var(--t-fast), color var(--t-fast);
  }
  .act-btn:active { color: var(--accent); }
  .act-btn.on { color: var(--accent); }
  .act-btn.save { color: var(--accent); }
  .act-btn.save:disabled { color: var(--text3); opacity: 0.5; }
  .act-btn:disabled { color: var(--text3); opacity: 0.5; }
  .empty, .loading { padding: 40px; text-align: center; color: var(--text3); font-size: var(--fs-body); }

  /* Preview header */
  .preview-header {
    display: flex; align-items: center; gap: 8px; padding: 8px 10px;
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .back-btn {
    padding: 6px; border: none; border-radius: var(--ui-radius-control); background: var(--surface2);
    color: var(--text2); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
    transition: color var(--t-fast), background var(--t-fast);
  }
  .preview-name {
    flex: 1; font-size: var(--fs-body); font-weight: 500; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap;
  }
  .preview-actions { display: flex; gap: 4px; }

  /* Preview body */
  .preview-body { flex: 1; overflow: auto; -webkit-overflow-scrolling: touch; padding: 12px; display: flex; flex-direction: column; min-height: 0; }
  /* Per-line code view: each logical line is its own flex row (number + code),
     so the line number sits at the top of its row and stays aligned even when
     the code soft-wraps. No-wrap mode scrolls horizontally with the number
     column pinned (sticky) on the left. */
  .code-lined {
    flex: 1; overflow: auto; -webkit-overflow-scrolling: touch;
    font-family: var(--font-mono); font-size: var(--file-font-size, 13px); line-height: 1.5;
    padding: 12px 0;
  }
  .cl-row { display: flex; align-items: flex-start; }
  .cl-num {
    position: sticky; left: 0; z-index: 1; flex-shrink: 0; min-width: 2.5em; padding: 0 8px;
    text-align: right; color: var(--text3); user-select: none; white-space: pre;
    background: var(--bg); border-right: 1px solid var(--border);
  }
  .cl-code {
    margin: 0 0 0 10px; flex: 1; min-width: 0; color: var(--text);
    white-space: pre; font-family: inherit;
  }
  .cl-code :global(code) { font-family: inherit; background: none; padding: 0; }
  .code-lined.wrap .cl-code { white-space: pre-wrap; word-break: break-word; }
  /* The "show all N lines" tail of a capped preview: a text button in the
     gutter's row grid, sticky like the numbers so it is reachable at any
     horizontal scroll. */
  .cl-more {
    display: block; position: sticky; left: 0; margin: 8px 0 0 10px; padding: 6px 10px;
    min-height: 44px; border: 1px solid var(--border); border-radius: var(--ui-radius-control);
    background: var(--surface); color: var(--accent); font: inherit; cursor: pointer;
    transition: background var(--t-fast);
  }
  .cl-more:hover { background: var(--surface2); }
  .html-preview {
    flex: 1; width: 100%; border: none; background: #fff; border-radius: 4px;
  }
  .pdf-container {
    flex: 1; overflow: auto; -webkit-overflow-scrolling: touch; padding: 4px;
    background: var(--surface);
  }
  .image-preview {
    flex: 1; display: flex; align-items: center; justify-content: center; overflow: auto; padding: 12px;
  }
  .image-preview img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 4px; }
  .md-render { font-size: var(--file-font-size, 14px); line-height: 1.6; color: var(--text); overflow-wrap: break-word; }
  .md-render :global(h1) { font-size: 1.55em; margin: 16px 0 8px; color: var(--accent); border-bottom: 1px solid var(--border); padding-bottom: 6px; }
  .md-render :global(h2) { font-size: 1.28em; margin: 14px 0 6px; color: var(--accent); }
  .md-render :global(h3) { font-size: 1.15em; margin: 10px 0 4px; color: var(--accent); }
  .md-render :global(h4), .md-render :global(h5), .md-render :global(h6) { font-size: 1em; margin: 8px 0 4px; color: var(--accent); }
  .md-render :global(p) { margin: 8px 0; }
  .md-render :global(code) { background: var(--surface2); padding: 2px 5px; border-radius: 3px; font-size: 0.86em; font-family: var(--font-mono); }
  .md-render :global(pre) { background: var(--code-bg); border-radius: 12px; padding: 12px; overflow-x: auto; margin: 8px 0; }
  .md-render :global(pre code) { background: none; padding: 0; font-size: var(--fs-ui); line-height: 1.5; }
  .md-render :global(strong) { color: var(--text); }
  .md-render :global(em) { color: var(--text2); }
  .md-render :global(a) { color: var(--accent); text-decoration: none; }
  .md-render :global(a:hover) { text-decoration: underline; }
  .md-render :global(ul), .md-render :global(ol) { padding-left: 20px; margin: 6px 0; }
  .md-render :global(li) { margin: 3px 0; }
  .md-render :global(blockquote) { border-left: 3px solid var(--accent); margin: 8px 0; padding: 4px 12px; color: var(--text2); }
  .md-render :global(hr) { border: none; border-top: 1px solid var(--border); margin: 12px 0; }
  .md-render :global(img) { max-width: 100%; border-radius: 6px; }
  .md-render :global(table) { border-collapse: collapse; width: 100%; margin: 8px 0; font-size: var(--fs-body); }
  .md-render :global(th), .md-render :global(td) { padding: 8px 12px; border: 1px solid var(--input-border); text-align: left; }
  .md-render :global(th) { background: var(--surface2); color: var(--accent); font-weight: 600; }
  .md-render :global(input[type="checkbox"]) { margin-right: 6px; }
  .md-render :global(.katex-display) { overflow-x: auto; margin: 8px 0; }
  .md-render :global(.mermaid-block) { background: var(--surface); border-radius: 12px; padding: 12px; margin: 8px 0; overflow-x: auto; }
  .md-render :global(.mermaid-block svg) { max-width: 100%; }
  .csv-render { overflow: auto; }
  .csv-render :global(table) { border-collapse: collapse; font-size: var(--fs-ui); width: 100%; }
  .csv-render :global(th), .csv-render :global(td) {
    padding: 6px 10px; border: 1px solid var(--input-border); text-align: left;
  }
  .csv-render :global(th) { background: var(--surface2); color: var(--accent); font-weight: 600; }
  .csv-render :global(td) { color: var(--text); }

  /* Editor */
  .editor-wrap {
    flex: 1; display: flex; overflow: hidden; -webkit-overflow-scrolling: touch; min-height: 0;
  }
  .editor-nums {
    padding: 12px 8px 12px 0; text-align: right; color: var(--text3); font-family: var(--font-mono);
    font-size: var(--file-font-size, 13px); line-height: 1.5; white-space: pre; user-select: none; flex-shrink: 0;
    border-right: 1px solid var(--border); overflow: hidden;
  }
  /* One block per logical line; its height is set inline from the measured
     mirror so the number aligns with the (possibly wrapped) line. The number
     sits at the top of its block. */
  .eln { padding: 0 8px; box-sizing: border-box; overflow: hidden; }
  .editor-layer { position: relative; flex: 1; min-width: 0; overflow: hidden; }
  .editor-highlight {
    margin: 0; padding: 12px; font-family: var(--font-mono); font-size: var(--file-font-size, 13px);
    line-height: 1.5; white-space: pre; color: var(--text);
    pointer-events: none; position: absolute; inset: 0; overflow: hidden;
  }
  .editor-highlight :global(code) { font-family: inherit; background: none; padding: 0; }
  /* Off-screen line-height probe: same font/line-height/wrap as the highlight
     layer; width is set in JS to the layer's content width before measuring. */
  .editor-mirror {
    position: absolute; top: 0; left: -99999px; visibility: hidden; pointer-events: none;
    font-family: var(--font-mono); font-size: var(--file-font-size, 13px); line-height: 1.5;
    white-space: pre; padding: 0;
  }
  .editor-mirror .emir { white-space: pre; }
  .editor {
    position: absolute; inset: 0; width: 100%; height: 100%; padding: 12px; border: none; resize: none;
    background: transparent; color: transparent; caret-color: var(--text);
    font-family: var(--font-mono); font-size: var(--file-font-size, 13px); line-height: 1.5; outline: none;
    white-space: pre; overflow: auto; -webkit-overflow-scrolling: touch; touch-action: pan-x pan-y;
  }
  /* Wrapped mode: soft-wrap the text + mirror (so measured heights match), drop
     horizontal scroll. The gutter stays visible — line numbers are aligned via
     the measured per-line heights. */
  .editor-wrap.wrap .editor-highlight,
  .editor-wrap.wrap .editor,
  .editor-wrap.wrap .editor-mirror .emir {
    white-space: pre-wrap; word-break: break-word; overflow-x: hidden;
  }
  .editor-wrap.wrap .editor { touch-action: pan-y; }
  .info-body { flex: 1; overflow: auto; padding: 12px; }
  .info-row {
    display: flex; padding: 10px 0; border-bottom: 1px solid var(--border2);
  }
  .info-label { width: 100px; flex-shrink: 0; color: var(--text3); font-size: var(--fs-ui); }
  .info-val { flex: 1; font-size: var(--fs-body); word-break: break-all; }
  .info-val.mono { font-family: var(--font-mono); }
  .info-path {
    flex: 1; font-size: var(--fs-body); word-break: break-all; text-align: left;
    background: none; border: none; color: var(--text); cursor: pointer; padding: 0;
    display: flex; align-items: center; gap: 4px; -webkit-tap-highlight-color: transparent;
    transition: color var(--t-fast);
  }
  .info-path:active { color: var(--accent); }

  /* Copy / download toasts. Centred with auto margins, NOT a translateX(-50%):
     the rise-in intro owns `transform`, and a resting transform here would
     be overridden for the 200ms it plays. */
  .copy-toast {
    position: absolute; bottom: 80px; left: 0; right: 0; margin: 0 auto; width: max-content; max-width: 90%;
    box-sizing: border-box;
    background: var(--bg); border: 1px solid var(--border); color: var(--accent); padding: 8px 20px;
    border-radius: var(--ui-radius-row); font-size: var(--fs-body); font-weight: 500;
    box-shadow: 0 4px 16px rgba(0,0,0,0.3); pointer-events: none;
  }
  /* The one-shot "Copied" flash fades in AND out — a local keyframe because it
     is an in+out one-shot, not an intro atom. */
  .copy-toast.flash { animation: toast-fade 1.2s ease forwards; }
  .download-toast {
    pointer-events: auto; display: flex; align-items: center; gap: 8px;
    font-size: var(--fs-ui);
  }
  .dl-path {
    flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    direction: rtl; text-align: left; min-width: 0;
    font-family: var(--font-mono); font-size: var(--fs-sub); color: var(--text2);
  }
  .dl-ring { flex-shrink: 0; }
  /* No transition on the progress arc — it must track the displayed integer
     percentage exactly. Square arc ends avoid the extra visual length that
     rounded caps add at both ends, especially below 10%.
     A `transition: stroke-dashoffset` made the arc lag the % number on fast
     (LAN) downloads: the number would read 94% while the arc was still easing
     through ~1/3. The two are now always in sync. */
  .dl-pct {
    font-family: var(--font-mono); font-size: var(--fs-sub);
    font-weight: 600; color: var(--accent); min-width: 30px;
  }
  .dl-name {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
  }
  .toast-open {
    padding: 4px 12px; border: 1px solid var(--accent); border-radius: var(--ui-radius-control);
    background: var(--accent-bg); color: var(--accent); font-size: var(--fs-ui);
    font-weight: 600; cursor: pointer; -webkit-tap-highlight-color: transparent;
    flex-shrink: 0;
  }
  .toast-close {
    padding: 2px; border: none; background: none; color: var(--text3);
    cursor: pointer; display: flex; flex-shrink: 0;
  }
  @keyframes toast-fade {
    0% { opacity: 0; }
    10%, 60% { opacity: 1; }
    100% { opacity: 0; }
  }
  @media (prefers-reduced-motion: reduce) {
    .file-list { transition: none; }
  }

</style>
