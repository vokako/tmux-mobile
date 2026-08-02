<script>
  import * as pdfjsLib from 'pdfjs-dist';
  import pdfjsWorker from 'pdfjs-dist/build/pdf.worker.min.mjs?url';
  import { marked } from 'marked';
  import '../core/markedSafeUrl.ts';
  import katex from 'katex';
  import 'katex/dist/katex.min.css';
  import hljs from 'highlight.js/lib/core';
  import javascript from 'highlight.js/lib/languages/javascript';
  import typescript from 'highlight.js/lib/languages/typescript';
  import python from 'highlight.js/lib/languages/python';
  import rust from 'highlight.js/lib/languages/rust';
  import css from 'highlight.js/lib/languages/css';
  import json from 'highlight.js/lib/languages/json';
  import bash from 'highlight.js/lib/languages/bash';
  import xml from 'highlight.js/lib/languages/xml';
  import yaml from 'highlight.js/lib/languages/yaml';
  import sql from 'highlight.js/lib/languages/sql';
  import go from 'highlight.js/lib/languages/go';
  import java from 'highlight.js/lib/languages/java';
  import ruby from 'highlight.js/lib/languages/ruby';
  import markdown from 'highlight.js/lib/languages/markdown';
  import 'highlight.js/styles/github-dark.min.css';
  import mermaid from 'mermaid';
  import Icon from '../ui/Icon.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import GitPanel from './GitPanel.svelte';
  import { createPersistedList } from './persisted-list.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { layout } from '../app/layout.svelte.ts';
  import { copyText } from '../core/clipboard.ts';
  import { directoryLoadState } from './file-view-state.ts';
  import { installExternalLinkHandler } from '../core/external-links.ts';
  import { fsCwd, fsList, fsStat, fsRead, fsWrite, fsMkdir, fsDelete, fsRename, fsDownload, fsDownloadHttp, fsUpload, getBookmarks, saveBookmarks, gitCmd, getPrefs, setPref, fsConvert } from '../core/ws.ts';

  // Tauri plugin imports (tree-shaken in browser builds)
  let tauriFs, tauriDialog, tauriOpener, tauriPath;
  const isTauri = typeof window !== 'undefined' && !!(window.__TAURI__ || window.__TAURI_INTERNALS__);
  const tauriReady = isTauri ? Promise.all([
    import('@tauri-apps/plugin-fs').then(m => tauriFs = m),
    import('@tauri-apps/plugin-dialog').then(m => tauriDialog = m),
    import('@tauri-apps/plugin-opener').then(m => tauriOpener = m),
    import('@tauri-apps/api/path').then(m => tauriPath = m),
  ]) : Promise.resolve();

  hljs.registerLanguage('javascript', javascript);
  hljs.registerLanguage('js', javascript);
  hljs.registerLanguage('typescript', typescript);
  hljs.registerLanguage('ts', typescript);
  hljs.registerLanguage('python', python);
  hljs.registerLanguage('rust', rust);
  hljs.registerLanguage('css', css);
  hljs.registerLanguage('json', json);
  hljs.registerLanguage('bash', bash);
  hljs.registerLanguage('sh', bash);
  hljs.registerLanguage('html', xml);
  hljs.registerLanguage('xml', xml);
  hljs.registerLanguage('svg', xml);
  hljs.registerLanguage('yaml', yaml);
  hljs.registerLanguage('sql', sql);
  hljs.registerLanguage('go', go);
  hljs.registerLanguage('java', java);
  hljs.registerLanguage('ruby', ruby);
  hljs.registerLanguage('markdown', markdown);

  marked.setOptions({
    highlight(code, lang) {
      if (lang && hljs.getLanguage(lang)) return hljs.highlight(code, { language: lang }).value;
      try { return hljs.highlightAuto(code).value; } catch { return code; }
    }
  });

  mermaid.initialize({ startOnLoad: false, theme: 'dark' });
  pdfjsLib.GlobalWorkerOptions.workerSrc = pdfjsWorker;

  let { session = '', onGoBack = null, visible = false, fontSize = 14 } = $props();

  function navPush() { history.pushState({ app: true }, ''); }

  // Register goBack for Android back gesture
  $effect(() => {
    if (onGoBack) onGoBack(() => {
      if (view === 'git') {
        if (gitPanelRef?.goBack()) return true;
        view = 'list'; return true;
      }
      if (view === 'edit') { if (isEdited && !confirm(t('discardChanges'))) return true; view = 'preview'; return true; }
      if (view === 'info') { view = fromGit ? (fromGit = false, 'git') : currentFile?.content != null ? 'preview' : 'list'; return true; }
      if (view === 'preview') { if (fromGit) { fromGit = false; view = 'git'; } else { view = 'list'; } currentFile = null; return true; }
      if (view === 'local') { view = 'list'; return true; }
      if (cwd !== '/') { goUp(); return true; }
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
  let splitEligible = $derived(!layout.isTouchDevice && (layout.forceDesktop || wideEnough));

  $effect(() => {
    const onResize = () => { wideEnough = window.innerWidth >= SPLIT_MIN_WIDTH; };
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  // Drag the splitter to adjust the browser/preview width ratio (desktop only).

  // Android local files
  const isAndroid = typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);
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
  let confirmDelete = $state(null);
  let deleteTimer;
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

  // Swipe right to go back (pull-to-refresh was removed — same rationale
  // as Sessions.svelte: on a scrollable list the gesture conflicts with
  // ordinary top-edge scrolling and produces dangling-indicator bugs.
  // Use the explicit refresh button / the automatic reload on directory
  // change instead.)
  let swipeStartX = 0;

  function onTouchStart(e) {
    swipeStartX = e.touches[0].clientX;
  }
  function onTouchEnd(e) {
    const dx = e.changedTouches[0].clientX - swipeStartX;
    if (dx > 60 && swipeStartX < 40) goBack();
  }

  function goBack() {
    if (view === 'edit') { view = 'preview'; }
    else if (view === 'info') { view = fromGit ? (fromGit = false, 'git') : currentFile?.content != null ? 'preview' : 'list'; }
    else if (view === 'preview') { if (fromGit) { fromGit = false; view = 'git'; } else { view = 'list'; } currentFile = null; }
    else goUp();
  }

  async function renderPdf(data) {
    if (!pdfContainer) return;
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
  let lastSourceDir = '';
  $effect(() => {
    if (!visible) return;
    // session may be '' when Files is opened before any terminal pane exists —
    // the server then reports the user's home directory. Once a terminal/team
    // session appears, its cwd differs from home and we follow it.
    fsCwd(session).then(r => {
      if (r.path && r.path !== lastSourceDir) {
        lastSourceDir = r.path;
        cwd = r.path;
        view = 'list';
        loadDir(r.path);
      }
    }).catch(() => {
      if (!lastSourceDir) { lastSourceDir = '/'; cwd = '/'; loadDir('/'); }
    });
  });

  // Refresh the preview content when returning to the tab on a preview.
  $effect(() => {
    if (visible && view === 'preview') reloadPreview();
  });

  async function loadDir(path, purpose = 'navigate') {
    loading = true;
    error = '';
    try {
      const r = await fsList(path, showHidden);
      entries = r.entries;
      cwd = path;
      ({ view, currentFile } = directoryLoadState({ view, currentFile }, purpose));
    } catch (e) {
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
    const parent = cwd.replace(/\/[^/]+\/?$/, '') || '/';
    loadDir(parent);
  }

  function scrollEnd(el) { el.scrollLeft = el.scrollWidth; }

  function goHome() {
    fsCwd(session).then(r => loadDir(r.path)).catch(() => loadDir('/'));
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
    if (entry.type === 'dir') {
      navPush();
      loadDir(entry.path);
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
    if (fromGit) { fromGit = false; view = 'git'; } else {
      // Stay in the file's parent directory, not session cwd
      const dir = currentFile?.path?.replace(/\/[^/]+$/, '') || cwd;
      view = 'list';
      if (dir !== cwd) loadDir(dir);
    }
    currentFile = null;
  }

  function backToPreview() {
    if (isEdited && !confirm('Discard unsaved changes?')) return;
    view = 'preview';
  }

  async function handleDelete(path) {
    if (confirmDelete === path) {
      clearTimeout(deleteTimer);
      try {
        await fsDelete(path);
        confirmDelete = null;
        if (view !== 'list') backToList();
        loadDir(cwd);
      } catch (e) { error = e.message; }
    } else {
      confirmDelete = path;
      clearTimeout(deleteTimer);
      deleteTimer = setTimeout(() => confirmDelete = null, 3000);
    }
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
        await tauriReady;
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
        await tauriReady;
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

  async function handleUpload() {
    if (isTauri) {
      await tauriReady;
      const selected = await tauriDialog.open({ multiple: true });
      if (!selected) return;
      const files = Array.isArray(selected) ? selected : [selected];
      for (const filePath of files) {
        const name = String(filePath).split('/').pop().split('\\').pop();
        const bytes = new Uint8Array(await tauriFs.readFile(filePath));
        let binary = '';
        for (let i = 0; i < bytes.length; i += 8192) {
          binary += String.fromCharCode(...bytes.subarray(i, i + 8192));
        }
        const b64 = btoa(binary);
        const dest = cwd.replace(/\/$/, '') + '/' + name;
        try { await fsUpload(dest, b64); } catch (e) { error = e.message; }
      }
      loadDir(cwd);
      return;
    }
    // Browser fallback
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = true;
    document.body.appendChild(input);
    input.onchange = async () => {
      for (const file of Array.from(input.files || [])) {
        const reader = new FileReader();
        await new Promise((resolve) => {
          reader.onload = async () => {
            const b64 = reader.result.split(',')[1];
            const path = cwd.replace(/\/$/, '') + '/' + file.name;
            try { await fsUpload(path, b64); } catch (e) { error = e.message; }
            resolve();
          };
          reader.readAsDataURL(file);
        });
      }
      document.body.removeChild(input);
      loadDir(cwd);
    };
    input.click();
  }

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

  function highlightCode(text, mime) {
    if (text == null) return '';
    const lang = hljsLang(mime);
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(text, { language: lang }).value;
    }
    try { return hljs.highlightAuto(text).value; } catch { return text.replace(/</g, '&lt;'); }
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
    const lang = hljsLang(mime);
    if (lang && hljs.getLanguage(lang)) {
      try { return hljs.highlight(line, { language: lang }).value; } catch {}
    }
    return line.replace(/</g, '&lt;');
  }

  function renderMarkdown(text) {
    if (text == null) return '';
    // Protect code blocks/inline code from KaTeX processing
    const codeHoles = [];
    let safe = text
      .replace(/```[\s\S]*?```/g, m => { codeHoles.push(m); return `\x00CODE${codeHoles.length - 1}\x00`; })
      .replace(/`[^`]+`/g, m => { codeHoles.push(m); return `\x00CODE${codeHoles.length - 1}\x00`; });

    // KaTeX: replace $$ blocks and $ inline
    safe = safe
      .replace(/\$\$([^$]+?)\$\$/g, (_, math) => {
        try { return katex.renderToString(math.trim(), { displayMode: true, throwOnError: false }); }
        catch { return `<pre>${math}</pre>`; }
      })
      .replace(/\$([^$\n]+?)\$/g, (_, math) => {
        try { return katex.renderToString(math.trim(), { displayMode: false, throwOnError: false }); }
        catch { return `<code>${math}</code>`; }
      });

    // Restore code blocks
    safe = safe.replace(/\x00CODE(\d+)\x00/g, (_, i) => codeHoles[i]);

    return marked.parse(safe, { breaks: true, gfm: true });
  }

  let mermaidId = 0;
  async function renderMermaidBlocks(container) {
    if (!container) return;
    const blocks = container.querySelectorAll('code.language-mermaid');
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
      <button class="tool-btn" onclick={goHome}><Icon name="home" size={13} /></button>
      <button class="tool-btn" onclick={() => loadDir(cwd)} aria-label="Refresh"><Icon name="refresh" size={13} /></button>
      <button class="tool-btn" onclick={() => { newType = newType ? '' : 'file'; newName = ''; }}><Icon name="plus" size={13} /></button>
      <button class="tool-btn" onclick={handleUpload}><Icon name="upload" size={13} /></button>
      <button class="tool-btn" class:tool-active={showHidden} onclick={() => { showHidden = !showHidden; loadDir(cwd); }}>
        <Icon name="eye" size={13} />
      </button>
      <button class="tool-btn" class:starred={isBookmarked(cwd)} onclick={() => toggleBookmark(cwd)} title="Bookmark">
        <Icon name={isBookmarked(cwd) ? 'star-filled' : 'star'} size={13} />
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
      <button class="bc-seg" onclick={() => loadDir('/')}>/</button>
      {#each breadcrumbs as bc}
        <button class="bc-seg" onclick={() => loadDir(bc.path)}>{bc.name}</button>
        <span class="bc-sep">/</span>
      {/each}
    </div>

    {#if showBookmarks && bookmarks.length}
      <div class="bookmarks-panel">
        {#each bookmarks as bm}
          <div class="bm-row">
            <span class="bm-icon"><Icon name="star-filled" size={13} /></span>
            <button class="bm-path" onclick={() => { loadDir(bm); showBookmarks = false; }} use:scrollEnd>
              {bm}
            </button>
            <button class="bm-del" onclick={() => toggleBookmark(bm)}><Icon name="x" size={12} /></button>
          </div>
        {/each}
      </div>
    {/if}

    {#if showRecent && recentFiles.length}
      <div class="bookmarks-panel">
        {#each recentFiles as rf}
          <div class="bm-row">
            <span class="bm-icon"><Icon name="clock" size={13} /></span>
            <button class="bm-path" onclick={() => { showRecent = false; openEntry({ type: 'file', path: rf.path, name: rf.name }); }} use:scrollEnd>
              <span style="color:var(--text3);font-size:10px">{rf.path.replace(/\/[^/]+$/, '')}/</span>{rf.name}
            </button>
            <button class="bm-del" onclick={() => { recentFiles = recentFiles.filter(f => f.path !== rf.path); setPref('recentFiles', recentFiles).catch(() => {}); }}><Icon name="x" size={12} /></button>
          </div>
        {/each}
      </div>
    {/if}

    <!-- New item input -->
    {#if newType}
      <div class="new-item">
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
      <div class="new-item">
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
      <div class="error">{error}</div>
    {/if}

    <!-- File list -->
    <div class="file-list" class:panel-open={showBookmarks || showRecent}>
      {#if loading}
        <div class="loading">{t('loading')}</div>
      {:else}
        {#each entries as entry}
          <div class="file-row" class:broken={entry.type === 'broken'}>
            <button class="file-main" onclick={() => openEntry(entry)}>
              <span class="file-icon" class:is-link={entry.is_symlink}>
                <Icon name={fileIcon(entry)} size={16} />
              </span>
              <span
                class="file-name"
                class:dir-name={entry.type === 'dir'}
                class:link-name={entry.is_symlink}
                title={entry.is_symlink && entry.link_target ? `→ ${entry.link_target}` : entry.name}
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
              <button class="act-btn del" class:confirm={confirmDelete === entry.path} onclick={() => handleDelete(entry.path)} title="Delete">
                {#if confirmDelete === entry.path}
                  <span class="del-text">{t('del')}</span>
                {:else}
                  <Icon name="trash" size={12} />
                {/if}
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
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'code'}
        <div class="code-lined" class:wrap={wrapLines}>
          {#each (currentFile.content ?? '').split('\n') as line, i}
            <div class="cl-row"><span class="cl-num">{i + 1}</span><code class="cl-code">{@html highlightLine(line, currentFile.stat?.mime_hint) || '\u200b'}</code></div>
          {/each}
        </div>
      {:else}
        <div class="code-lined" class:wrap={wrapLines}>
          {#each (currentFile.content ?? '').split('\n') as line, i}
            <div class="cl-row"><span class="cl-num">{i + 1}</span><code class="cl-code">{@html highlightLine(line, currentFile.stat?.mime_hint) || '\u200b'}</code></div>
          {/each}
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
      <button class="back-btn" onclick={() => { view = 'list'; }}><Icon name="chevron-left" size={16} /></button>
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
          <button class="act-btn del" onclick={() => deleteLocalFile(f.name)}><Icon name="trash" size={12} /></button>
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
      <button class="back-btn" onclick={() => { view = currentFile?.content != null ? 'preview' : 'list'; }}><Icon name="chevron-left" size={16} /></button>
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
<div class="files" bind:this={filesEl} ontouchstart={onTouchStart} ontouchend={onTouchEnd}>
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
    <div class="copy-toast">{t('copied')}</div>
  {/if}
  {#if downloading}
    <div class="copy-toast download-toast">
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
    <div class="copy-toast download-toast">
      {t('saved')} <span class="dl-path">{downloadToast}</span>
      {#if downloadedPath}
        <button class="toast-open" onclick={openDownloaded}>{t('open')}</button>
      {/if}
      <button class="toast-close" onclick={dismissDownload}><Icon name="x" size={12} /></button>
    </div>
  {/if}
</div>

<style>
  .files { display: flex; flex-direction: column; flex: 1; min-height: 0; background: var(--bg); }

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
    gap: 10px; color: var(--text3); font-size: 13px; padding: 24px; text-align: center;
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
    width: var(--ui-control-height); height: var(--ui-control-height);
    padding: 0; border: 1px solid var(--border2); border-radius: var(--ui-radius-pill);
    background: var(--input-bg); color: var(--text2); cursor: pointer;
    font-size: var(--ui-font-control); display: flex; align-items: center; justify-content: center;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .tool-btn:active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }
  .tool-btn.tool-active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }
  .tool-btn.starred { color: var(--accent); }

  /* Path row */
  .bc-path-row {
    display: flex; align-items: center; gap: 1px; padding: 4px 10px;
    overflow-x: auto; font-size: 12px; font-family: var(--font-mono);
    scrollbar-width: none; border-bottom: 1px solid var(--border2); flex-shrink: 0;
  }
  .bc-path-row::-webkit-scrollbar { display: none; }
  .bc-seg {
    padding: 2px 4px; border: none; background: none; color: var(--text2);
    cursor: pointer; white-space: nowrap; font-size: 12px; font-family: inherit;
  }
  .bc-seg:last-of-type { color: var(--accent); }
  .bc-sep { color: var(--text3); font-size: 11px; }

  /* Bookmarks / Recent panel */
  .bookmarks-panel {
    border-bottom: 1px solid var(--border2); flex-shrink: 0;
    max-height: 40vh; overflow-y: auto;
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
    font-size: 12px; font-family: var(--font-mono);
    cursor: pointer; text-align: left; overflow-x: auto;
    white-space: nowrap; scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
  }
  .bm-path::-webkit-scrollbar { display: none; }
  .bm-path:active { color: var(--accent); }
  .bm-del {
    padding: 4px; border: none; border-radius: 4px; background: none;
    color: var(--text3); cursor: pointer; display: flex;
  }
  .bm-del:active { color: var(--danger); }

  /* New item / rename */
  .new-item {
    display: flex; gap: 6px; padding: 6px 10px;
    border-bottom: 1px solid var(--border2); min-width: 0;
  }
  .new-item input {
    flex: 1; min-width: 0; padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 6px;
    background: var(--input-bg); color: var(--text); font-size: 13px;
    font-family: var(--font-mono);
  }
  .new-item button {
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 6px;
    background: var(--surface2); color: var(--text2); cursor: pointer;
  }
  .new-type-btn { display: flex; align-items: center; color: var(--accent); }

  .error {
    padding: 8px 12px; background: var(--bg2); color: var(--danger);
    font-size: 12px; border-bottom: 1px solid var(--danger);
  }

  /* File list */
  .file-list { flex: 1; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  /* When the bookmarks/recent panel is open, lock the file list so touch
     gestures on the panel can't bleed through and drag the list too. */
  .file-list.panel-open { overflow: hidden; touch-action: none; }
  .file-row {
    display: flex; align-items: center; border-bottom: 1px solid var(--border2);
  }
  .file-main {
    flex: 1; display: flex; align-items: center; gap: 10px; padding: 14px 12px;
    border: none; background: none; color: var(--text); cursor: pointer; text-align: left;
    font-size: 14px; min-width: 0; -webkit-tap-highlight-color: transparent;
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
    font-size: 10px;
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
  .file-size { color: var(--text3); font-size: 11px; font-family: var(--font-mono); white-space: nowrap; }
  .file-actions { display: flex; gap: 2px; padding-right: 8px; }
  .act-btn {
    padding: 6px; border: none; border-radius: 6px; background: none;
    color: var(--text3); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .act-btn:active { color: var(--accent); }
  .act-btn.on { color: var(--accent); }
  .act-btn.del:active, .act-btn.del.confirm { color: var(--danger); }
  .del-text { font-size: 10px; font-weight: 600; }
  .act-btn.save { color: var(--accent); }
  .act-btn.save:disabled { color: var(--text3); opacity: 0.5; }
  .act-btn:disabled { color: var(--text3); opacity: 0.5; }
  .empty, .loading { padding: 40px; text-align: center; color: var(--text3); font-size: 14px; }

  /* Preview header */
  .preview-header {
    display: flex; align-items: center; gap: 8px; padding: 8px 10px;
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .back-btn {
    padding: 6px; border: none; border-radius: 6px; background: var(--surface2);
    color: var(--text2); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .preview-name {
    flex: 1; font-size: 14px; font-weight: 500; overflow: hidden;
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
  .md-render :global(h1) { font-size: 22px; margin: 16px 0 8px; color: var(--accent); border-bottom: 1px solid var(--border); padding-bottom: 6px; }
  .md-render :global(h2) { font-size: 18px; margin: 14px 0 6px; color: var(--accent); }
  .md-render :global(h3) { font-size: 16px; margin: 10px 0 4px; color: var(--accent); }
  .md-render :global(h4), .md-render :global(h5), .md-render :global(h6) { font-size: 14px; margin: 8px 0 4px; color: var(--accent); }
  .md-render :global(p) { margin: 8px 0; }
  .md-render :global(code) { background: var(--surface2); padding: 2px 5px; border-radius: 3px; font-size: 12px; font-family: var(--font-mono); }
  .md-render :global(pre) { background: var(--code-bg); border-radius: 8px; padding: 12px; overflow-x: auto; margin: 8px 0; }
  .md-render :global(pre code) { background: none; padding: 0; font-size: 12px; line-height: 1.5; }
  .md-render :global(strong) { color: var(--text); }
  .md-render :global(em) { color: var(--text2); }
  .md-render :global(a) { color: var(--accent); text-decoration: none; }
  .md-render :global(a:hover) { text-decoration: underline; }
  .md-render :global(ul), .md-render :global(ol) { padding-left: 20px; margin: 6px 0; }
  .md-render :global(li) { margin: 3px 0; }
  .md-render :global(blockquote) { border-left: 3px solid var(--accent); margin: 8px 0; padding: 4px 12px; color: var(--text2); }
  .md-render :global(hr) { border: none; border-top: 1px solid var(--border); margin: 12px 0; }
  .md-render :global(img) { max-width: 100%; border-radius: 6px; }
  .md-render :global(table) { border-collapse: collapse; width: 100%; margin: 8px 0; font-size: 13px; }
  .md-render :global(th), .md-render :global(td) { padding: 8px 12px; border: 1px solid var(--input-border); text-align: left; }
  .md-render :global(th) { background: var(--surface2); color: var(--accent); font-weight: 600; }
  .md-render :global(input[type="checkbox"]) { margin-right: 6px; }
  .md-render :global(.katex-display) { overflow-x: auto; margin: 8px 0; }
  .md-render :global(.mermaid-block) { background: var(--surface); border-radius: 8px; padding: 12px; margin: 8px 0; overflow-x: auto; }
  .md-render :global(.mermaid-block svg) { max-width: 100%; }
  .csv-render { overflow: auto; }
  .csv-render :global(table) { border-collapse: collapse; font-size: 12px; width: 100%; }
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
  .info-label { width: 100px; flex-shrink: 0; color: var(--text3); font-size: 12px; }
  .info-val { flex: 1; font-size: 13px; word-break: break-all; }
  .info-val.mono { font-family: var(--font-mono); }
  .info-path {
    flex: 1; font-size: 13px; word-break: break-all; text-align: left;
    background: none; border: none; color: var(--text); cursor: pointer; padding: 0;
    display: flex; align-items: center; gap: 4px; -webkit-tap-highlight-color: transparent;
  }
  .info-path:active { color: var(--accent); }

  /* Copy toast */
  .copy-toast {
    position: absolute; bottom: 80px; left: 50%; transform: translateX(-50%);
    background: var(--bg); border: 1px solid var(--border); color: var(--accent); padding: 8px 20px;
    border-radius: 8px; font-size: 13px; font-weight: 500;
    box-shadow: 0 4px 16px rgba(0,0,0,0.3); pointer-events: none;
    animation: toast-fade 1.2s ease forwards;
  }
  .download-toast {
    pointer-events: auto; display: flex; align-items: center; gap: 8px;
    animation: none; max-width: 90%; font-size: 12px;
  }
  .dl-path {
    flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    direction: rtl; text-align: left; min-width: 0;
    font-family: var(--font-mono); font-size: 11px; color: var(--text2);
  }
  .dl-ring { flex-shrink: 0; }
  /* No transition on the progress arc — it must track the displayed integer
     percentage exactly. Square arc ends avoid the extra visual length that
     rounded caps add at both ends, especially below 10%.
     A `transition: stroke-dashoffset` made the arc lag the % number on fast
     (LAN) downloads: the number would read 94% while the arc was still easing
     through ~1/3. The two are now always in sync. */
  .dl-pct {
    font-family: var(--font-mono); font-size: 11px;
    font-weight: 600; color: var(--accent); min-width: 30px;
  }
  .dl-name {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
  }
  .toast-open {
    padding: 4px 12px; border: 1px solid var(--accent); border-radius: 6px;
    background: var(--accent-bg); color: var(--accent); font-size: 12px;
    font-weight: 600; cursor: pointer; -webkit-tap-highlight-color: transparent;
    flex-shrink: 0;
  }
  .toast-close {
    padding: 2px; border: none; background: none; color: var(--text3);
    cursor: pointer; display: flex; flex-shrink: 0;
  }
  @keyframes toast-fade {
    0%, 60% { opacity: 1; }
    100% { opacity: 0; }
  }

</style>
