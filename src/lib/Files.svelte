<script>
  import * as pdfjsLib from 'pdfjs-dist';
  import pdfjsWorker from 'pdfjs-dist/build/pdf.worker.min.mjs?url';
  import { marked } from 'marked';
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
  import Icon from './Icon.svelte';
  import { fsCwd, fsList, fsStat, fsRead, fsWrite, fsMkdir, fsDelete, fsRename, fsDownload, fsUpload, getBookmarks, saveBookmarks, gitCmd, getPrefs, setPref, fsConvert } from './ws.js';

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
      if (view === 'git' && gitDiff) { gitDiff = null; return true; }
      if (view === 'git') { view = 'list'; return true; }
      if (view === 'edit') { if (isEdited && !confirm('Discard unsaved changes?')) return true; view = 'preview'; return true; }
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
  let loading = $state(false);
  let error = $state('');

  // View modes: 'list', 'preview', 'edit', 'info', 'local'
  let view = $state('list');
  let previewZoom = $state(100);

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
      localFiles = files.map(name => ({ name }));
      localDir = '/storage/emulated/0/Download/TmuxMobile/';
      view = 'local';
      navPush();
    } catch (e) { error = e.message; }
  }

  async function waitForFileOpener(maxWait = 2000) {
    if (window.AndroidFileOpener) return true;
    const start = Date.now();
    while (Date.now() - start < maxWait) {
      await new Promise(r => setTimeout(r, 100));
      if (window.AndroidFileOpener) return true;
    }
    return false;
  }

  async function openFileNative(path) {
    if (!window.AndroidFileOpener) await waitForFileOpener();
    if (window.AndroidFileOpener) {
      const result = window.AndroidFileOpener.openFile(path);
      if (result !== 'ok') throw new Error(result);
    } else {
      throw new Error('No file opener available');
    }
  }

  async function openLocalFile(name) {
    try {
      await openFileNative(localDir + name);
    } catch (e) { error = 'Open failed: ' + (e.message || e); }
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

  $effect(() => {
    getPrefs().then(p => { recentFiles = p.recentFiles || []; }).catch(() => {});
  });

  function addRecent(path, name) {
    recentFiles = [{ path, name }, ...recentFiles.filter(f => f.path !== path)].slice(0, 20);
    setPref('recentFiles', recentFiles).catch(() => {});
  }

  // Git view state
  let gitTab = $state('status'); // 'status' | 'log'
  let gitStatus = $state([]);
  let gitLog = $state([]);
  let gitBranch = $state('');
  let gitDiff = $state(null); // { file, diff }
  let fromGit = $state(false);
  let gitLoading = $state(false);
  let gitListEl = $state(null);
  let hasGit = $state(false);

  $effect(() => {
    if (cwd) {
      gitCmd('rev-parse', ['--git-dir'], cwd).then(r => { hasGit = r.code === 0; }).catch(() => { hasGit = false; });
    }
  });
  let gitError = $state('');

  let gitRoot = '';

  async function git(subcmd, ...args) {
    if (!gitRoot) {
      const r = await gitCmd('rev-parse', ['--show-toplevel'], cwd);
      gitRoot = r.stdout.trim();
    }
    const r = await gitCmd(subcmd, args, gitRoot);
    if (r.code !== 0 && r.stderr) throw new Error(r.stderr.trim());
    return r.stdout;
  }

  async function loadGitStatus() {
    const scrollTop = gitListEl?.scrollTop || 0;
    gitLoading = true;
    gitError = '';
    try {
      gitBranch = (await git('branch', '--show-current')).trim();
      const out = await git('status', '--porcelain');
      gitStatus = out.split('\n').filter(Boolean).map(line => ({
        status: line.slice(0, 2),
        file: line.slice(3),
      }));
    } catch (e) { gitError = e.message; }
    gitLoading = false;
    requestAnimationFrame(() => { if (gitListEl) gitListEl.scrollTop = scrollTop; });
  }

  async function loadGitLog() {
    gitLoading = true;
    gitError = '';
    try {
      const out = await git('log', '--oneline', '-30', '--format=%h|%s|%ar|%an');
      gitLog = out.split('\n').filter(Boolean).map(line => {
        const [hash, subject, date, author] = line.split('|');
        return { hash, subject, date, author };
      });
    } catch (e) { gitError = e.message; }
    gitLoading = false;
  }

  async function openGitView() {
    view = 'git';
    gitTab = 'status';
    gitDiff = null;
    gitRoot = '';
    await loadGitStatus();
    navPush();
  }

  async function showFileDiff(file, staged) {
    gitLoading = true;
    try {
      const isUntracked = gitStatus.find(f => f.file === file)?.status === '??';
      const isBinary = !isUntracked && (await git('diff', '--numstat', ...(staged ? ['--cached'] : []), '--', file)).startsWith('-');
      if (isUntracked || isBinary || file.match(/\.(png|jpe?g|gif|webp|svg|ico|bmp|avif|pdf|zip|tar|gz|mp[34]|mov|wav)$/i)) {
        gitLoading = false;
        fromGit = true;
        openEntry({ type: 'file', path: gitRoot + '/' + file, name: file.split('/').pop() });
        return;
      }
      const diff = await git('diff', ...(staged ? ['--cached'] : []), '--', file);
      gitDiff = { file, diff: diff || '(no changes)', _root: gitRoot };
    } catch (e) { gitDiff = { file, diff: e.message, _root: '' }; }
    gitLoading = false;
  }

  async function showCommitDiff(hash) {
    gitLoading = true;
    try {
      const diff = await git('show', '--stat', '--patch', hash);
      gitDiff = { file: hash, diff, _root: gitRoot };
    } catch (e) { gitDiff = { file: hash, diff: e.message, _root: '' }; }
    gitLoading = false;
  }

  let pushResult = $state('');
  let commitMsg = $state('');
  let showCommitInput = $state(false);

  async function gitPush() {
    gitLoading = true;
    pushResult = '';
    try {
      const r = await git('push');
      pushResult = r.trim() || 'Pushed';
    } catch (e) { pushResult = '✗ ' + e.message; }
    gitLoading = false;
    setTimeout(() => { pushResult = ''; }, 3000);
  }

  async function gitAddAll() {
    try { await git('add', '.'); } catch {}
    loadGitStatus();
  }

  async function gitAddFile(file) {
    try { await git('add', file); } catch {}
    loadGitStatus();
  }

  async function gitRestoreFile(file) {
    try { await git('restore', '--staged', file); } catch {}
    loadGitStatus();
  }

  async function gitCommit() {
    if (!commitMsg.trim()) return;
    gitLoading = true;
    pushResult = '';
    try {
      await git('commit', '-m', commitMsg.trim());
      pushResult = 'Committed';
      commitMsg = '';
      showCommitInput = false;
      loadGitStatus();
    } catch (e) { pushResult = '✗ ' + e.message; }
    gitLoading = false;
    setTimeout(() => { pushResult = ''; }, 3000);
  }

  $effect(() => {
    getBookmarks().then(r => { bookmarks = r.bookmarks || []; }).catch(() => {});
  });

  function isBookmarked(path) { return bookmarks.includes(path); }

  async function toggleBookmark(path) {
    if (isBookmarked(path)) {
      bookmarks = bookmarks.filter(b => b !== path);
    } else {
      bookmarks = [...bookmarks, path];
    }
    await saveBookmarks(bookmarks).catch(() => {});
  }

  // Swipe right to go back + pull to refresh
  const isMobile = 'ontouchstart' in window || navigator.maxTouchPoints > 0;
  let swipeStartX = 0;
  let pullStartY = 0;
  let pullDist = $state(0);
  let pulling = $state(false);
  let refreshing = $state(false);
  let canPull = false;

  function onTouchStart(e) {
    swipeStartX = e.touches[0].clientX;
    pullStartY = e.touches[0].clientY;
    pulling = false;
    pullDist = 0;
    const listEl = filesEl?.querySelector('.file-list');
    canPull = view === 'list' && listEl && listEl.scrollTop <= 0;
  }
  function onTouchMove(e) {
    if (!canPull || refreshing) return;
    const dy = e.touches[0].clientY - pullStartY;
    if (dy > 10) { pulling = true; pullDist = Math.min(100, dy * 0.5); }
    else if (dy < -5) { canPull = false; }
  }
  let refreshDone = $state(false);

  function onTouchEnd(e) {
    const dx = e.changedTouches[0].clientX - swipeStartX;
    if (dx > 60 && swipeStartX < 40) goBack();
    if (pulling && pullDist >= 60) {
      refreshing = true;
      pullDist = 60;
      loadDir(cwd).finally(() => {
        refreshing = false;
        refreshDone = true;
        setTimeout(() => { refreshDone = false; pullDist = 0; }, 600);
      });
    } else {
      pullDist = 0;
    }
    pulling = false;
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

  $effect(() => {
    cwd;
    setTimeout(() => { if (bcPathEl) bcPathEl.scrollLeft = bcPathEl.scrollWidth; }, 0);
  });

  // Sync CWD from terminal session only on first visit to list view
  let cwdSynced = false;
  $effect(() => {
    if (session && visible && view === 'list' && !cwdSynced) {
      cwdSynced = true;
      fsCwd(session).then(r => {
        if (r.path !== cwd) {
          cwd = r.path;
          loadDir(r.path);
        }
      }).catch(() => {});
    }
    if (visible && view === 'preview') {
      reloadPreview();
    }
  });

  // Init: get session CWD
  $effect(() => {
    if (session) {
      fsCwd(session).then(r => {
        cwd = r.path;
        loadDir(r.path);
      }).catch(() => {
        cwd = '/';
        loadDir('/');
      });
    }
  });

  async function loadDir(path) {
    loading = true;
    error = '';
    try {
      const r = await fsList(path, showHidden);
      entries = r.entries;
      cwd = path;
      view = 'list';
      currentFile = null;
    } catch (e) {
      error = e.message;
    }
    loading = false;
  }

  function goUp() {
    const parent = cwd.replace(/\/[^/]+\/?$/, '') || '/';
    loadDir(parent);
  }

  function scrollEnd(el) { el.scrollLeft = el.scrollWidth; }

  function goHome() {
    fsCwd(session).then(r => loadDir(r.path)).catch(() => loadDir('/'));
  }

  async function openEntry(entry) {
    if (entry.type === 'dir') {
      navPush();
      loadDir(entry.path);
      return;
    }
    loading = true;
    try {
      const stat = await fsStat(entry.path);
      currentFile = { path: entry.path, name: entry.name, stat };
      addRecent(entry.path, entry.name);
      navPush();
      if (stat.mime_hint === 'application/pdf') {
        const r = await fsDownload(entry.path);
        currentFile.pdfData = r.data;
        view = 'preview';
      } else if (stat.mime_hint.startsWith('image/')) {
        const r = await fsDownload(entry.path);
        currentFile.dataUrl = `data:${stat.mime_hint};base64,${r.data}`;
        view = 'preview';
      } else if (stat.is_text && stat.size <= 512 * 1024) {
        const r = await fsRead(entry.path);
        currentFile.content = r.content;
        view = 'preview';
      } else if (entry.name.match(/\.pptx$/i)) {
        const r = await fsConvert(entry.path);
        currentFile.convertedHtml = r.html;
        view = 'preview';
      } else {
        view = 'info';
      }
    } catch (e) {
      error = e.message;
    }
    loading = false;
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
  }

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
  let dlProgress = $state(0);
  let dlProgressTimer = null;

  // Simulate gradual progress while waiting for download RPC
  function startDlProgress() {
    dlProgress = 5;
    let target = 80; // simulate up to 80%, rest is real stages
    dlProgressTimer = setInterval(() => {
      if (dlProgress < target) {
        // Slow down as we approach target
        const remaining = target - dlProgress;
        dlProgress += Math.max(0.5, remaining * 0.06);
      }
    }, 100);
  }
  function stopDlProgress() {
    clearInterval(dlProgressTimer);
    dlProgressTimer = null;
  }

  async function openDownloaded() {
    if (!downloadedPath) return;
    try {
      if (isAndroid) {
        await openFileNative(downloadedPath);
      } else if (isTauri) {
        await tauriReady;
        await tauriOpener.openPath(downloadedPath);
      }
    } catch (e) { error = 'Open failed: ' + (e.message || e); }
    dismissDownload();
  }

  function dismissDownload() {
    downloadToast = ''; downloadedPath = ''; downloading = ''; dlProgress = 0;
  }

  async function handleDownload(path) {
    const name = path.split('/').pop();
    try {
      downloading = name;
      startDlProgress();
      if (isTauri && tauriFs) {
        await tauriReady;
        if (isAndroid) {
          const r = await fsDownload(path);
          stopDlProgress(); dlProgress = 85;
          const { invoke } = await import('@tauri-apps/api/core');
          dlProgress = 90;
          const filePath = await invoke('save_to_downloads', { name: r.name, data: r.data });
          dlProgress = 100;
          await new Promise(r => setTimeout(r, 300));
          downloading = '';
          downloadedPath = filePath;
          downloadToast = filePath;
          setTimeout(() => { if (downloadToast === filePath) dismissDownload(); }, 10000);
          return;
        }
        // macOS / desktop: use save dialog
        const savePath = await tauriDialog.save({ defaultPath: name });
        if (!savePath) { stopDlProgress(); downloading = ''; dlProgress = 0; return; }
        const r = await fsDownload(path);
        stopDlProgress(); dlProgress = 85;
        const bytes = Uint8Array.from(atob(r.data), c => c.charCodeAt(0));
        dlProgress = 90;
        await tauriFs.writeFile(savePath, bytes);
        dlProgress = 100;
        await new Promise(r => setTimeout(r, 300));
        downloading = '';
        downloadedPath = String(savePath);
        downloadToast = downloadedPath;
        setTimeout(() => { if (downloadToast === downloadedPath) dismissDownload(); }, 10000);
        return;
      }
      // Browser fallback
      const r = await fsDownload(path);
      stopDlProgress(); dlProgress = 90;
      const bytes = Uint8Array.from(atob(r.data), c => c.charCodeAt(0));
      const blob = new Blob([bytes]);
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url; a.download = r.name;
      document.body.appendChild(a);
      a.click();
      dlProgress = 100;
      await new Promise(r => setTimeout(r, 300));
      setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
      downloading = '';
      downloadToast = 'Downloaded';
      setTimeout(() => downloadToast = '', 2000);
    } catch (e) { stopDlProgress(); downloading = ''; dlProgress = 0; error = e.message; }
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
    try { await navigator.clipboard.writeText(path); } catch {}
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
    return entry.type === 'dir' ? 'folder' : 'file';
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
    const lang = hljsLang(mime);
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(text, { language: lang }).value;
    }
    try { return hljs.highlightAuto(text).value; } catch { return text.replace(/</g, '&lt;'); }
  }

  function renderMarkdown(text) {
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
<div class="files" bind:this={filesEl} ontouchstart={onTouchStart} ontouchmove={onTouchMove} ontouchend={onTouchEnd}>
  {#if view === 'list'}
    <!-- Toolbar: all buttons in one row -->
    <div class="toolbar">
      <button class="tool-btn" onclick={goHome}><Icon name="home" size={15} /></button>
      {#if !isMobile}<button class="tool-btn" onclick={() => loadDir(cwd)}><Icon name="refresh" size={15} /></button>{/if}
      <button class="tool-btn" onclick={() => { newType = newType ? '' : 'file'; newName = ''; }}><Icon name="plus" size={15} /></button>
      <button class="tool-btn" onclick={handleUpload}><Icon name="upload" size={15} /></button>
      <button class="tool-btn" class:tool-active={showHidden} onclick={() => { showHidden = !showHidden; loadDir(cwd); }}>
        <Icon name="eye" size={15} />
      </button>
      <button class="tool-btn" class:starred={isBookmarked(cwd)} onclick={() => toggleBookmark(cwd)} title="Bookmark">
        <Icon name={isBookmarked(cwd) ? 'star-filled' : 'star'} size={15} />
      </button>
      <button class="tool-btn" class:tool-active={showBookmarks} onclick={() => { showBookmarks = !showBookmarks; showRecent = false; }} title="Bookmarks">
        <Icon name="folder-star" size={15} />
      </button>
      <button class="tool-btn" class:tool-active={showRecent} onclick={() => { showRecent = !showRecent; showBookmarks = false; }} title="Recent">
        <Icon name="clock" size={15} />
      </button>
      {#if hasGit}
        <button class="tool-btn" onclick={openGitView} title="Git">
          <Icon name="git-branch" size={15} />
        </button>
      {/if}
      <div style="flex:1"></div>
      {#if isTauri}
        <button class="tool-btn" onclick={openLocalFiles} title="Local files"><Icon name="download" size={15} /></button>
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
            <button class="bm-path bm-path-recent" onclick={() => { showRecent = false; openEntry({ type: 'file', path: rf.path, name: rf.name }); }}>
              <span class="bm-dir">{rf.path.replace(/\/[^/]+$/, '')}/</span><span class="bm-name">{rf.name}</span>
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
          placeholder={newType === 'dir' ? 'folder name...' : 'file name...'}
          onkeydown={(e) => e.key === 'Enter' && handleNewItem()}
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
          placeholder="new name..."
          onkeydown={(e) => e.key === 'Enter' && handleRename()}
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
    {#if pullDist > 0}
      <div class="pull-indicator" style="height:{pullDist}px">
        {#if refreshDone}
          <svg class="pull-done" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
        {:else}
          <svg class="pull-arrow" class:pull-spin={refreshing} style="transform:rotate({refreshing ? 0 : Math.min(pullDist / 60 * 180, 180)}deg);opacity:{Math.min(pullDist / 30, 1)}" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
          </svg>
        {/if}
      </div>
    {/if}
    <div class="file-list">
      {#if loading}
        <div class="loading">Loading...</div>
      {:else}
        {#each entries as entry}
          <div class="file-row">
            <button class="file-main" onclick={() => openEntry(entry)}>
              <Icon name={fileIcon(entry)} size={16} />
              <span class="file-name" class:dir-name={entry.type === 'dir'} title={entry.name}>{entry.name}</span>
              {#if entry.type !== 'dir'}
                <span class="file-size">{formatSize(entry.size)}</span>
              {/if}
            </button>
            <div class="file-actions">
              {#if entry.type !== 'dir'}
                <button class="act-btn" onclick={() => handleDownload(entry.path)} title="Download"><Icon name="download" size={12} /></button>
              {/if}
              <button class="act-btn" onclick={() => { renaming = entry.path; renameValue = entry.name; }} title="Rename"><Icon name="edit" size={12} /></button>
              <button class="act-btn del" class:confirm={confirmDelete === entry.path} onclick={() => handleDelete(entry.path)} title="Delete">
                {#if confirmDelete === entry.path}
                  <span class="del-text">del</span>
                {:else}
                  <Icon name="trash" size={12} />
                {/if}
              </button>
            </div>
          </div>
        {/each}
        {#if !entries.length && !loading}
          <div class="empty">Empty directory</div>
        {/if}
      {/if}
    </div>

  {:else if view === 'preview'}
    <!-- File preview -->
    <div class="preview-header">
      <button class="back-btn" onclick={backToList}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile.name}</span>
      <div class="preview-actions">
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
        <iframe class="html-preview" srcdoc={currentFile.content} sandbox="allow-same-origin" title="HTML Preview"></iframe>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'pdf'}
        <div class="pdf-container" bind:this={pdfContainer} style="margin: -12px; padding: 0;"></div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'image'}
        <div class="image-preview"><img src={currentFile.dataUrl} alt={currentFile.name} /></div>
      {:else if currentFile.convertedHtml}
        <div class="md-render">{@html currentFile.convertedHtml}</div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'code'}
        <div class="code-lined">
          <div class="line-nums">{@html currentFile.content.split('\n').map((_, i) => i + 1).join('\n')}</div>
          <pre class="code-preview"><code>{@html highlightCode(currentFile.content, currentFile.stat?.mime_hint)}</code></pre>
        </div>
      {:else}
        <div class="code-lined">
          <div class="line-nums">{@html currentFile.content.split('\n').map((_, i) => i + 1).join('\n')}</div>
          <pre class="code-preview"><code>{@html highlightCode(currentFile.content, currentFile.stat?.mime_hint)}</code></pre>
        </div>
      {/if}
    </div>

  {:else if view === 'edit'}
    <!-- File editor -->
    <div class="preview-header">
      <button class="back-btn" onclick={backToPreview}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile.name}{isEdited ? ' *' : ''}</span>
      <div class="preview-actions">
        <button class="act-btn" onclick={undo} disabled={!undoStack.length && editContent === editOriginal}><Icon name="undo" size={14} /></button>
        <button class="act-btn save" onclick={saveFile} disabled={!isEdited}><Icon name="save" size={14} /></button>
      </div>
    </div>
    <div class="editor-wrap" style="--file-font-size:{fontSize}px">
      <div class="editor-nums">{@html editContent.split('\n').map((_, i) => i + 1).join('\n')}</div>
      <div class="editor-layer">
        <pre class="editor-highlight" aria-hidden="true"><code>{@html highlightCode(editContent, currentFile?.stat?.mime_hint)}</code>{'\n'}</pre>
        <textarea
          class="editor"
          value={editContent}
          oninput={onEditInput}
          spellcheck="false"
          autocapitalize="off"
          autocomplete="off"
        ></textarea>
      </div>
    </div>

  {:else if view === 'local'}
    <!-- Local downloaded files -->
    <div class="preview-header">
      <button class="back-btn" onclick={() => { view = 'list'; }}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">Downloads</span>
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
        <div class="empty">No downloaded files</div>
      {/if}
    </div>

  {:else if view === 'info'}
    <!-- File info -->
    <div class="preview-header">
      <button class="back-btn" onclick={() => { view = currentFile?.content != null ? 'preview' : 'list'; }}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile?.name}</span>
      <div class="preview-actions">
        <button class="act-btn" onclick={() => handleDownload(currentFile.path)}><Icon name="download" size={14} /></button>
        <button class="act-btn" onclick={() => copyPath(currentFile.path)}><Icon name="copy" size={14} /></button>
      </div>
    </div>
    <div class="info-body">
      {#if currentFile?.stat}
        <div class="info-row"><span class="info-label">Path</span><button class="info-path" onclick={() => copyPath(currentFile.stat.path)}>{currentFile.stat.path}</button></div>
        <div class="info-row"><span class="info-label">Type</span><span class="info-val">{currentFile.stat.mime_hint}</span></div>
        <div class="info-row"><span class="info-label">Size</span><span class="info-val">{formatSize(currentFile.stat.size)}</span></div>
        <div class="info-row"><span class="info-label">Modified</span><span class="info-val">{formatDate(currentFile.stat.modified)}</span></div>
        <div class="info-row"><span class="info-label">Permissions</span><span class="info-val mono">{currentFile.stat.permissions}</span></div>
        <div class="info-row"><span class="info-label">Readable</span><span class="info-val">{currentFile.stat.readable ? 'Yes' : 'No'}</span></div>
        <div class="info-row"><span class="info-label">Writable</span><span class="info-val">{currentFile.stat.writable ? 'Yes' : 'No'}</span></div>
        <div class="info-row"><span class="info-label">Text file</span><span class="info-val">{currentFile.stat.is_text ? 'Yes' : 'No'}</span></div>
      {/if}
    </div>
  {:else if view === 'git'}
    <!-- Git view -->
    <div class="preview-header">
      <button class="back-btn" onclick={() => { if (gitDiff) gitDiff = null; else view = 'list'; }}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name"><Icon name="git-branch" size={14} /> {gitBranch || 'Git'}</span>
      <div class="preview-actions">
        <button class="act-btn" onclick={() => { gitTab === 'status' ? loadGitStatus() : loadGitLog(); }}><Icon name="refresh" size={14} /></button>
      </div>
    </div>
    {#if pushResult}
      <div class="git-push-result" class:git-error={pushResult.startsWith('✗')}>{pushResult}</div>
    {/if}
    {#if !gitDiff}
      {@const stagedCount = gitStatus.filter(f => f.status[0] !== ' ' && f.status[0] !== '?').length}
      <div class="git-actions">
        <button class="git-act-btn" onclick={gitAddAll}>Add All</button>
        <button class="git-act-btn" onclick={() => showCommitInput = !showCommitInput}>Commit{stagedCount ? ` (${stagedCount})` : ''}</button>
        <button class="git-act-btn" onclick={gitPush} disabled={gitLoading}>Push</button>
      </div>
      {#if showCommitInput}
        <div class="git-commit-row">
          <textarea bind:value={commitMsg} placeholder="commit message…" onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); gitCommit(); } }} rows="2"></textarea>
          <button class="git-commit-btn" onclick={gitCommit} disabled={!commitMsg.trim()}>OK</button>
        </div>
      {/if}
      <div class="git-tabs">
        <button class:active={gitTab === 'status'} onclick={() => { gitTab = 'status'; loadGitStatus(); }}>Status</button>
        <button class:active={gitTab === 'log'} onclick={() => { gitTab = 'log'; loadGitLog(); }}>Log</button>
      </div>
      {#if gitError}
        <div class="git-error">{gitError}</div>
      {/if}
      {#if gitLoading}
        <div class="git-loading">Loading…</div>
      {:else if gitTab === 'status'}
        <div class="git-list" bind:this={gitListEl}>
          {#if !gitStatus.length}
            <div class="empty">Working tree clean</div>
          {/if}
          {#each gitStatus as f}
            {@const staged = f.status[0] !== ' ' && f.status[0] !== '?'}
            <div class="git-file-row">
              <button class="git-file" onclick={() => showFileDiff(f.file, staged)}>
                <span class="git-st" class:git-add={f.status.includes('A') || f.status.includes('?')} class:git-mod={f.status.includes('M')} class:git-del={f.status.includes('D')}>{f.status === '??' ? ' U' : f.status}</span>
                <span class="git-fname">{f.file}</span>
              </button>
              <button class="git-stage-btn" onclick={(e) => { e.stopPropagation(); staged ? gitRestoreFile(f.file) : gitAddFile(f.file); }}>
                {staged ? '−' : '+'}
              </button>
            </div>
          {/each}
        </div>
      {:else}
        <div class="git-list">
          {#each gitLog as c}
            <button class="git-file" onclick={() => showCommitDiff(c.hash)}>
              <span class="git-hash">{c.hash}</span>
              <span class="git-fname">{c.subject}</span>
              <span class="git-date">{c.date}</span>
            </button>
          {/each}
        </div>
      {/if}
    {:else}
      <div class="git-diff-header">
        <span class="git-diff-name">{gitDiff.file}</span>
        <button class="act-btn" onclick={() => {
          const root = gitDiff._root;
          if (root) { fromGit = true; openEntry({ type: 'file', path: root + '/' + gitDiff.file, name: gitDiff.file.split('/').pop() }); }
        }}><Icon name="eye" size={14} /></button>
      </div>
      <div class="git-diff-body" style="--file-font-size:{fontSize}px">
        {#each gitDiff.diff.split('\n') as line, i}
          <div class="diff-line" class:diff-add={line.startsWith('+') && !line.startsWith('+++')} class:diff-del={line.startsWith('-') && !line.startsWith('---')} class:diff-hunk={line.startsWith('@@')} class:diff-meta={line.startsWith('diff ') || line.startsWith('index ') || line.startsWith('---') || line.startsWith('+++')}><span class="diff-text">{line}</span></div>
        {/each}
      </div>
    {/if}
  {/if}
  {#if copyToast}
    <div class="copy-toast">Copied</div>
  {/if}
  {#if downloading}
    <div class="copy-toast download-toast">
      <svg class="dl-ring" width="28" height="28" viewBox="0 0 28 28">
        <circle cx="14" cy="14" r="11" fill="none" stroke="var(--border)" stroke-width="2.5" />
        <circle cx="14" cy="14" r="11" fill="none" stroke="var(--accent)" stroke-width="2.5"
          stroke-linecap="round" transform="rotate(-90 14 14)"
          stroke-dasharray={2 * Math.PI * 11}
          stroke-dashoffset={2 * Math.PI * 11 * (1 - dlProgress / 100)} />
      </svg>
      <span class="dl-pct">{Math.round(dlProgress)}%</span>
      <span class="dl-name">{downloading}</span>
    </div>
  {:else if downloadToast}
    <div class="copy-toast download-toast">
      Saved: <span class="dl-path">{downloadToast}</span>
      {#if downloadedPath}
        <button class="toast-open" onclick={openDownloaded}>Open</button>
      {/if}
      <button class="toast-close" onclick={dismissDownload}><Icon name="x" size={12} /></button>
    </div>
  {/if}
</div>

<style>
  .files { display: flex; flex-direction: column; flex: 1; min-height: 0; background: var(--bg); }

  /* Toolbar */
  .toolbar {
    display: flex; align-items: center; gap: 4px; padding: 6px 10px;
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .tool-btn {
    padding: 8px; border: none; border-radius: 6px;
    background: var(--surface2); color: var(--text2); cursor: pointer;
    font-size: 12px; display: flex; align-items: center; gap: 4px; -webkit-tap-highlight-color: transparent;
  }
  .tool-btn:active { background: var(--accent-bg); color: var(--accent); }
  .tool-btn.tool-active { background: var(--accent-bg); color: var(--accent); }
  .tool-btn.starred { color: var(--accent); }

  /* Path row */
  .bc-path-row {
    display: flex; align-items: center; gap: 1px; padding: 4px 10px;
    overflow-x: auto; font-size: 12px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    scrollbar-width: none; border-bottom: 1px solid var(--border2); flex-shrink: 0;
  }
  .bc-path-row::-webkit-scrollbar { display: none; }
  .bc-seg {
    padding: 2px 4px; border: none; background: none; color: var(--text2);
    cursor: pointer; white-space: nowrap; font-size: 12px; font-family: inherit;
  }
  .bc-seg:last-of-type { color: var(--accent); }
  .bc-sep { color: var(--text3); font-size: 11px; }

  /* Bookmarks panel */
  .bookmarks-panel {
    border-bottom: 1px solid var(--border2); flex-shrink: 0;
  }
  .bm-row {
    display: flex; align-items: center; gap: 6px; padding: 0 10px;
    border-bottom: 1px solid var(--border2);
  }
  .bm-icon { color: var(--accent); display: flex; flex-shrink: 0; }
  .bm-path {
    flex: 1; display: block;
    padding: 8px 0; border: none; background: none; color: var(--text);
    font-size: 12px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    cursor: pointer; text-align: left; overflow-x: auto;
    white-space: nowrap; scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
  }
  .bm-path::-webkit-scrollbar { display: none; }
  .bm-path:active { color: var(--accent); }
  .bm-path-recent {
    display: flex !important; align-items: baseline; overflow: hidden !important;
  }
  .bm-dir {
    flex-shrink: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis;
    color: var(--text3); font-size: 10px;
  }
  .bm-name { flex-shrink: 0; color: var(--text); }
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
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
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
  .pull-indicator {
    display: flex; align-items: center; justify-content: center;
    color: var(--accent); flex-shrink: 0; overflow: hidden;
    transition: height 0.25s ease;
  }
  .pull-arrow { transition: opacity 0.15s; }
  .pull-done { color: var(--status-ok); animation: pull-pop 0.3s ease; }
  @keyframes pull-pop { 0% { transform: scale(0.5); opacity: 0; } 100% { transform: scale(1); opacity: 1; } }
  .pull-spin { animation: pull-rotate 0.6s linear infinite; }
  @keyframes pull-rotate { to { transform: rotate(360deg) !important; } }
  .file-row {
    display: flex; align-items: center; border-bottom: 1px solid var(--border2);
  }
  .file-main {
    flex: 1; display: flex; align-items: center; gap: 10px; padding: 14px 12px;
    border: none; background: none; color: var(--text); cursor: pointer; text-align: left;
    font-size: 14px; min-width: 0; -webkit-tap-highlight-color: transparent;
  }
  .file-main:active { background: var(--input-bg); }
  .file-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dir-name { color: var(--accent); }
  .file-size { color: var(--text3); font-size: 11px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; white-space: nowrap; }
  .file-actions { display: flex; gap: 2px; padding-right: 8px; }
  .act-btn {
    padding: 6px; border: none; border-radius: 6px; background: none;
    color: var(--text3); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .act-btn:active { color: var(--accent); }
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
  .zoom-ctl { display: flex; align-items: center; gap: 2px; }
  .zoom-pct { font-size: 11px; color: var(--text3); min-width: 32px; text-align: center; }

  /* Preview body */
  .preview-body { flex: 1; overflow: auto; -webkit-overflow-scrolling: touch; padding: 12px; display: flex; flex-direction: column; min-height: 0; }
  .code-preview {
    margin: 0; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: var(--file-font-size, 13px);
    line-height: 1.5; color: var(--text); white-space: pre-wrap; word-break: break-all; flex: 1;
  }
  .code-preview :global(code) { font-family: inherit; background: none; padding: 0; }
  .code-lined {
    display: flex; flex: 1; overflow: auto; -webkit-overflow-scrolling: touch;
  }
  .line-nums {
    padding: 0 8px; text-align: right; color: var(--text3); font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: var(--file-font-size, 13px); line-height: 1.5; white-space: pre; user-select: none; flex-shrink: 0;
    border-right: 1px solid var(--border);
  }
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
  .md-render :global(code) { background: var(--surface2); padding: 2px 5px; border-radius: 3px; font-size: 12px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; }
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
    flex: 1; display: flex; overflow: auto; -webkit-overflow-scrolling: touch; min-height: 0; touch-action: pan-y;
  }
  .editor-nums {
    padding: 12px 8px; text-align: right; color: var(--text3); font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: var(--file-font-size, 13px); line-height: 1.5; white-space: pre; user-select: none; flex-shrink: 0;
    border-right: 1px solid var(--border);
  }
  .editor-layer { position: relative; flex: 1; min-width: 0; }
  .editor-highlight {
    margin: 0; padding: 12px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: var(--file-font-size, 13px);
    line-height: 1.5; white-space: pre-wrap; word-break: break-all; color: var(--text);
    pointer-events: none;
  }
  .editor-highlight :global(code) { font-family: inherit; background: none; padding: 0; }
  .editor {
    position: absolute; inset: 0; width: 100%; height: 100%; padding: 12px; border: none; resize: none;
    background: transparent; color: transparent; caret-color: var(--text);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: var(--file-font-size, 13px); line-height: 1.5; outline: none;
    white-space: pre-wrap; word-break: break-all; overflow: hidden;
  }
  .info-body { flex: 1; overflow: auto; padding: 12px; }
  .info-row {
    display: flex; padding: 10px 0; border-bottom: 1px solid var(--border2);
  }
  .info-label { width: 100px; flex-shrink: 0; color: var(--text3); font-size: 12px; }
  .info-val { flex: 1; font-size: 13px; word-break: break-all; }
  .info-val.mono { font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; }
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
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 11px; color: var(--text2);
  }
  .dl-ring { flex-shrink: 0; }
  .dl-ring circle:last-child { transition: stroke-dashoffset 0.3s ease; }
  .dl-pct {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 11px;
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

  /* Git view */
  .git-tabs {
    display: flex; gap: 2px; padding: 6px 10px; border-bottom: 1px solid var(--border);
    background: var(--pill-bg); flex-shrink: 0;
  }
  .git-tabs button {
    padding: 6px 16px; border: none; border-radius: 6px; background: transparent;
    color: var(--text3); font-size: 12px; font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .git-tabs button.active { background: var(--accent-bg); color: var(--accent); }
  .git-error { padding: 10px; color: var(--danger); font-size: 12px; background: var(--danger-bg); }
  .git-push-result { padding: 8px 12px; font-size: 12px; color: var(--status-ok); background: var(--accent-bg); flex-shrink: 0; }
  .git-push-result.git-error { color: var(--danger); background: var(--danger-bg); }
  .git-actions {
    display: flex; gap: 4px; padding: 6px 10px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .git-act-btn {
    padding: 5px 12px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--surface2); color: var(--text2); font-size: 12px; font-weight: 500;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .git-act-btn:active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }
  .git-act-btn:disabled { opacity: 0.4; }
  .git-commit-row {
    display: flex; gap: 6px; padding: 6px 10px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .git-commit-row textarea {
    flex: 1; padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 6px;
    background: var(--input-bg); color: var(--text); font-size: 13px; outline: none;
    font-family: inherit; resize: none; line-height: 1.4;
  }
  .git-commit-row textarea:focus { border-color: var(--accent); }
  .git-commit-btn {
    padding: 6px 14px; border: none; border-radius: 6px;
    background: var(--accent); color: var(--bg); font-size: 12px; font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .git-commit-btn:disabled { opacity: 0.4; }
  .git-file-row { display: flex; align-items: center; border-bottom: 1px solid var(--border2); }
  .git-file-row .git-file { flex: 1; border-bottom: none; }
  .git-stage-btn {
    width: 32px; height: 32px; border: none; border-radius: 6px;
    background: none; color: var(--accent); font-size: 18px; font-weight: 600;
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    flex-shrink: 0; -webkit-tap-highlight-color: transparent;
  }
  .git-stage-btn:active { background: var(--accent-bg); }
  .git-loading { padding: 20px; text-align: center; color: var(--text3); font-size: 13px; }
  .git-list { flex: 1; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  .git-file {
    display: flex; align-items: center; gap: 8px; width: 100%;
    padding: 10px 12px; border: none; background: none;
    border-bottom: 1px solid var(--border2); cursor: pointer;
    text-align: left; color: var(--text); font-size: 13px;
    -webkit-tap-highlight-color: transparent;
  }
  .git-file:active { background: var(--accent-bg); }
  .git-st {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 12px; font-weight: 600;
    min-width: 24px; color: var(--text3);
  }
  .git-st.git-add { color: var(--status-ok); }
  .git-st.git-mod { color: var(--status-warn); }
  .git-st.git-del { color: var(--danger); }
  .git-fname {
    flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 12px;
  }
  .git-hash {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 11px;
    color: var(--accent); min-width: 56px;
  }
  .git-date { font-size: 11px; color: var(--text3); white-space: nowrap; }
  .git-diff-header {
    display: flex; align-items: center; gap: 8px;
    padding: 8px 12px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 12px;
    color: var(--accent); background: var(--accent-bg); border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .git-diff-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .git-diff-body {
    flex: 1; overflow: auto; -webkit-overflow-scrolling: touch; background: var(--code-bg);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: var(--file-font-size, 13px); line-height: 1.6;
  }
  .diff-line {
    padding: 0 12px; white-space: pre; min-height: 1.6em;
    border-left: 3px solid transparent;
  }
  .diff-line.diff-add { background: rgba(74,222,128,0.1); border-left-color: var(--status-ok); color: var(--status-ok); }
  .diff-line.diff-del { background: rgba(255,80,80,0.1); border-left-color: var(--danger); color: var(--danger); }
  .diff-line.diff-hunk { color: var(--accent); font-weight: 600; background: var(--accent-bg); }
  .diff-line.diff-meta { color: var(--text3); }
  .diff-text { user-select: text; -webkit-user-select: text; }
</style>
