<script>
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
  import { fsCwd, fsList, fsStat, fsRead, fsWrite, fsMkdir, fsDelete, fsRename, fsDownload, fsUpload } from './ws.js';

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

  let { session = '' } = $props();

  // State
  let cwd = $state('');
  let entries = $state([]);
  let showHidden = $state(false);
  let loading = $state(false);
  let error = $state('');

  // View modes: 'list', 'preview', 'edit', 'info'
  let view = $state('list');
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
  let bcPathEl;

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

  function goHome() {
    fsCwd(session).then(r => loadDir(r.path)).catch(() => loadDir('/'));
  }

  async function openEntry(entry) {
    if (entry.type === 'dir') {
      loadDir(entry.path);
      return;
    }
    loading = true;
    try {
      const stat = await fsStat(entry.path);
      currentFile = { path: entry.path, name: entry.name, stat };
      if (stat.mime_hint === 'application/pdf') {
        const r = await fsDownload(entry.path);
        currentFile.dataUrl = `data:application/pdf;base64,${r.data}`;
        view = 'preview';
      } else if (stat.mime_hint.startsWith('image/')) {
        const r = await fsDownload(entry.path);
        currentFile.dataUrl = `data:${stat.mime_hint};base64,${r.data}`;
        view = 'preview';
      } else if (stat.is_text && stat.size <= 512 * 1024) {
        const r = await fsRead(entry.path);
        currentFile.content = r.content;
        view = 'preview';
      } else {
        view = 'info';
      }
    } catch (e) {
      error = e.message;
    }
    loading = false;
  }

  function startEdit() {
    editContent = currentFile.content;
    editOriginal = currentFile.content;
    undoStack = [];
    view = 'edit';
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
    view = 'list';
    currentFile = null;
  }

  function backToPreview() {
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

  async function handleDownload(path) {
    try {
      const r = await fsDownload(path);
      const bytes = Uint8Array.from(atob(r.data), c => c.charCodeAt(0));
      const blob = new Blob([bytes]);
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url; a.download = r.name; a.click();
      URL.revokeObjectURL(url);
    } catch (e) { error = e.message; }
  }

  async function handleUpload() {
    const input = document.createElement('input');
    input.type = 'file';
    input.onchange = async () => {
      const file = input.files[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = async () => {
        const b64 = reader.result.split(',')[1];
        const path = cwd.replace(/\/$/, '') + '/' + file.name;
        try {
          await fsUpload(path, b64);
          loadDir(cwd);
        } catch (e) { error = e.message; }
      };
      reader.readAsDataURL(file);
    };
    input.click();
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
    // KaTeX: replace $$ blocks and $ inline before marked processes them
    let processed = text
      .replace(/\$\$([^$]+?)\$\$/g, (_, math) => {
        try { return katex.renderToString(math.trim(), { displayMode: true, throwOnError: false }); }
        catch { return `<pre>${math}</pre>`; }
      })
      .replace(/\$([^$\n]+?)\$/g, (_, math) => {
        try { return katex.renderToString(math.trim(), { displayMode: false, throwOnError: false }); }
        catch { return `<code>${math}</code>`; }
      });

    return marked.parse(processed, { breaks: true, gfm: true });
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

  let previewEl;
  $effect(() => {
    if (view === 'preview' && mimeCategory(currentFile?.stat?.mime_hint) === 'markdown' && previewEl) {
      // tick then render mermaid
      setTimeout(() => renderMermaidBlocks(previewEl), 50);
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

<div class="files">
  {#if view === 'list'}
    <!-- Breadcrumb -->
    <div class="breadcrumb">
      <button class="bc-btn" onclick={goHome}><Icon name="home" size={14} /></button>
      <button class="bc-btn" onclick={goUp}><Icon name="folder-up" size={14} /></button>
      <button class="bc-btn" onclick={() => loadDir(cwd)}><Icon name="refresh" size={14} /></button>
      <div class="bc-path" bind:this={bcPathEl}>
        <button class="bc-seg" onclick={() => loadDir('/')}>/</button>
        {#each breadcrumbs as bc}
          <button class="bc-seg" onclick={() => loadDir(bc.path)}>{bc.name}</button>
          <span class="bc-sep">/</span>
        {/each}
      </div>
    </div>

    <!-- Toolbar -->
    <div class="toolbar">
      <button class="tool-btn" onclick={() => { newType = 'file'; newName = ''; }}>
        <Icon name="plus" size={12} /> File
      </button>
      <button class="tool-btn" onclick={() => { newType = 'dir'; newName = ''; }}>
        <Icon name="folder" size={12} /> Folder
      </button>
      <button class="tool-btn" onclick={handleUpload}>
        <Icon name="upload" size={12} /> Upload
      </button>
      <button class="tool-btn" class:tool-active={showHidden} onclick={() => { showHidden = !showHidden; loadDir(cwd); }}>
        <Icon name="eye" size={12} /> Hidden
      </button>
    </div>

    <!-- New item input -->
    {#if newType}
      <div class="new-item">
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
    <div class="file-list">
      {#if loading}
        <div class="loading">Loading...</div>
      {:else}
        {#each entries as entry}
          <div class="file-row">
            <button class="file-main" onclick={() => openEntry(entry)}>
              <Icon name={fileIcon(entry)} size={16} />
              <span class="file-name" class:dir-name={entry.type === 'dir'}>{entry.name}</span>
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
        {#if currentFile.stat?.writable}
          <button class="act-btn" onclick={startEdit}><Icon name="edit" size={14} /></button>
        {/if}
        <button class="act-btn" onclick={() => handleDownload(currentFile.path)}><Icon name="download" size={14} /></button>
        <button class="act-btn" onclick={() => { view = 'info'; }}><Icon name="info" size={14} /></button>
      </div>
    </div>
    <div class="preview-body">
      {#if mimeCategory(currentFile.stat?.mime_hint) === 'markdown'}
        <div class="md-render" bind:this={previewEl}>{@html renderMarkdown(currentFile.content)}</div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'csv'}
        <div class="csv-render">{@html renderCsv(currentFile.content)}</div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'html'}
        <iframe class="html-preview" srcdoc={currentFile.content} sandbox="allow-scripts allow-same-origin" title="HTML Preview"></iframe>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'pdf'}
        <iframe class="html-preview" src={currentFile.dataUrl} title="PDF Preview"></iframe>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'image'}
        <div class="image-preview"><img src={currentFile.dataUrl} alt={currentFile.name} /></div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'code'}
        <div class="code-lined">
          <div class="line-nums">{@html currentFile.content.split('\n').map((_, i) => i + 1).join('\n')}</div>
          <pre class="code-preview"><code>{@html highlightCode(currentFile.content, currentFile.stat?.mime_hint)}</code></pre>
        </div>
      {:else}
        <div class="code-lined">
          <div class="line-nums">{@html currentFile.content.split('\n').map((_, i) => i + 1).join('\n')}</div>
          <pre class="code-preview">{currentFile.content}</pre>
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
    <div class="editor-wrap">
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

  {:else if view === 'info'}
    <!-- File info -->
    <div class="preview-header">
      <button class="back-btn" onclick={() => { view = currentFile?.content != null ? 'preview' : 'list'; }}><Icon name="chevron-left" size={16} /></button>
      <span class="preview-name">{currentFile?.name}</span>
      <div class="preview-actions">
        <button class="act-btn" onclick={() => handleDownload(currentFile.path)}><Icon name="download" size={14} /></button>
      </div>
    </div>
    <div class="info-body">
      {#if currentFile?.stat}
        <div class="info-row"><span class="info-label">Path</span><span class="info-val">{currentFile.stat.path}</span></div>
        <div class="info-row"><span class="info-label">Type</span><span class="info-val">{currentFile.stat.mime_hint}</span></div>
        <div class="info-row"><span class="info-label">Size</span><span class="info-val">{formatSize(currentFile.stat.size)}</span></div>
        <div class="info-row"><span class="info-label">Modified</span><span class="info-val">{formatDate(currentFile.stat.modified)}</span></div>
        <div class="info-row"><span class="info-label">Permissions</span><span class="info-val mono">{currentFile.stat.permissions}</span></div>
        <div class="info-row"><span class="info-label">Readable</span><span class="info-val">{currentFile.stat.readable ? 'Yes' : 'No'}</span></div>
        <div class="info-row"><span class="info-label">Writable</span><span class="info-val">{currentFile.stat.writable ? 'Yes' : 'No'}</span></div>
        <div class="info-row"><span class="info-label">Text file</span><span class="info-val">{currentFile.stat.is_text ? 'Yes' : 'No'}</span></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .files { display: flex; flex-direction: column; flex: 1; min-height: 0; background: #0a0a0f; }

  /* Breadcrumb */
  .breadcrumb {
    display: flex; align-items: center; gap: 4px; padding: 8px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.06); flex-shrink: 0;
  }
  .bc-btn {
    padding: 6px; border: none; border-radius: 6px; background: rgba(255,255,255,0.06);
    color: rgba(226,232,240,0.6); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .bc-btn:active { background: rgba(0,212,255,0.15); color: #00d4ff; }
  .bc-path {
    display: flex; align-items: center; gap: 1px; overflow-x: auto; flex: 1;
    font-size: 12px; font-family: 'SF Mono', Menlo, monospace; scrollbar-width: none;
  }
  .bc-path::-webkit-scrollbar { display: none; }
  .bc-seg {
    padding: 3px 4px; border: none; background: none; color: rgba(226,232,240,0.5);
    cursor: pointer; white-space: nowrap; font-size: 12px; font-family: inherit;
  }
  .bc-seg:last-of-type { color: #00d4ff; }
  .bc-sep { color: rgba(226,232,240,0.2); font-size: 11px; }

  /* Toolbar */
  .toolbar {
    display: flex; align-items: center; gap: 6px; padding: 6px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.04); flex-shrink: 0;
  }
  .tool-btn {
    padding: 5px 10px; border: 1px solid rgba(255,255,255,0.08); border-radius: 6px;
    background: rgba(255,255,255,0.04); color: rgba(226,232,240,0.6); cursor: pointer;
    font-size: 12px; display: flex; align-items: center; gap: 4px; -webkit-tap-highlight-color: transparent;
  }
  .tool-btn:active { background: rgba(0,212,255,0.1); color: #00d4ff; }
  .tool-btn.tool-active { background: rgba(0,212,255,0.12); color: #00d4ff; border-color: rgba(0,212,255,0.2); }

  /* New item / rename */
  .new-item {
    display: flex; gap: 6px; padding: 6px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.04);
  }
  .new-item input {
    flex: 1; padding: 6px 10px; border: 1px solid rgba(255,255,255,0.1); border-radius: 6px;
    background: rgba(255,255,255,0.04); color: #e2e8f0; font-size: 13px;
    font-family: 'SF Mono', Menlo, monospace;
  }
  .new-item button {
    padding: 6px 10px; border: 1px solid rgba(255,255,255,0.1); border-radius: 6px;
    background: rgba(255,255,255,0.06); color: rgba(226,232,240,0.6); cursor: pointer;
  }

  .error {
    padding: 8px 12px; background: rgba(255,50,50,0.1); color: #ff5050;
    font-size: 12px; border-bottom: 1px solid rgba(255,50,50,0.2);
  }

  /* File list */
  .file-list { flex: 1; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  .file-row {
    display: flex; align-items: center; border-bottom: 1px solid rgba(255,255,255,0.03);
  }
  .file-main {
    flex: 1; display: flex; align-items: center; gap: 10px; padding: 12px 12px;
    border: none; background: none; color: #e2e8f0; cursor: pointer; text-align: left;
    font-size: 14px; min-width: 0; -webkit-tap-highlight-color: transparent;
  }
  .file-main:active { background: rgba(255,255,255,0.04); }
  .file-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dir-name { color: #00d4ff; }
  .file-size { color: rgba(226,232,240,0.3); font-size: 11px; font-family: 'SF Mono', Menlo, monospace; white-space: nowrap; }
  .file-actions { display: flex; gap: 2px; padding-right: 8px; }
  .act-btn {
    padding: 6px; border: none; border-radius: 6px; background: none;
    color: rgba(226,232,240,0.3); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .act-btn:active { color: #00d4ff; }
  .act-btn.del:active, .act-btn.del.confirm { color: #ff5050; }
  .del-text { font-size: 10px; font-weight: 600; }
  .act-btn.save { color: #4ade80; }
  .act-btn.save:disabled { color: rgba(226,232,240,0.15); }
  .act-btn:disabled { color: rgba(226,232,240,0.15); }
  .empty, .loading { padding: 40px; text-align: center; color: rgba(226,232,240,0.3); font-size: 14px; }

  /* Preview header */
  .preview-header {
    display: flex; align-items: center; gap: 8px; padding: 8px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.06); flex-shrink: 0;
  }
  .back-btn {
    padding: 6px; border: none; border-radius: 6px; background: rgba(255,255,255,0.06);
    color: rgba(226,232,240,0.6); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .preview-name {
    flex: 1; font-size: 14px; font-weight: 500; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap;
  }
  .preview-actions { display: flex; gap: 4px; }

  /* Preview body */
  .preview-body { flex: 1; overflow: auto; -webkit-overflow-scrolling: touch; padding: 12px; }
  .code-preview {
    margin: 0; font-family: 'SF Mono', Menlo, monospace; font-size: 13px;
    line-height: 1.5; color: #e2e8f0; white-space: pre-wrap; word-break: break-all; flex: 1;
  }
  .code-preview :global(code) { font-family: inherit; background: none; padding: 0; }
  .code-lined {
    display: flex; flex: 1; overflow: auto; -webkit-overflow-scrolling: touch;
  }
  .line-nums {
    padding: 0 8px; text-align: right; color: rgba(226,232,240,0.2); font-family: 'SF Mono', Menlo, monospace;
    font-size: 13px; line-height: 1.5; white-space: pre; user-select: none; flex-shrink: 0;
    border-right: 1px solid rgba(255,255,255,0.06);
  }
  .html-preview {
    flex: 1; width: 100%; border: none; background: #fff; border-radius: 4px;
  }
  .image-preview {
    flex: 1; display: flex; align-items: center; justify-content: center; overflow: auto; padding: 12px;
  }
  .image-preview img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 4px; }
  .md-render { font-size: 14px; line-height: 1.6; color: #e2e8f0; overflow-wrap: break-word; }
  .md-render :global(h1) { font-size: 22px; margin: 16px 0 8px; color: #00d4ff; border-bottom: 1px solid rgba(255,255,255,0.1); padding-bottom: 6px; }
  .md-render :global(h2) { font-size: 18px; margin: 14px 0 6px; color: #00d4ff; }
  .md-render :global(h3) { font-size: 16px; margin: 10px 0 4px; color: #00d4ff; }
  .md-render :global(h4), .md-render :global(h5), .md-render :global(h6) { font-size: 14px; margin: 8px 0 4px; color: rgba(0,212,255,0.7); }
  .md-render :global(p) { margin: 8px 0; }
  .md-render :global(code) { background: rgba(255,255,255,0.08); padding: 2px 5px; border-radius: 3px; font-size: 12px; font-family: 'SF Mono', Menlo, monospace; }
  .md-render :global(pre) { background: rgba(255,255,255,0.05); border-radius: 8px; padding: 12px; overflow-x: auto; margin: 8px 0; }
  .md-render :global(pre code) { background: none; padding: 0; font-size: 12px; line-height: 1.5; }
  .md-render :global(strong) { color: #fff; }
  .md-render :global(em) { color: rgba(226,232,240,0.8); }
  .md-render :global(a) { color: #00d4ff; text-decoration: none; }
  .md-render :global(a:hover) { text-decoration: underline; }
  .md-render :global(ul), .md-render :global(ol) { padding-left: 20px; margin: 6px 0; }
  .md-render :global(li) { margin: 3px 0; }
  .md-render :global(blockquote) { border-left: 3px solid rgba(0,212,255,0.4); margin: 8px 0; padding: 4px 12px; color: rgba(226,232,240,0.6); }
  .md-render :global(hr) { border: none; border-top: 1px solid rgba(255,255,255,0.1); margin: 12px 0; }
  .md-render :global(img) { max-width: 100%; border-radius: 6px; }
  .md-render :global(table) { border-collapse: collapse; width: 100%; margin: 8px 0; font-size: 13px; }
  .md-render :global(th), .md-render :global(td) { padding: 8px 12px; border: 1px solid rgba(255,255,255,0.1); text-align: left; }
  .md-render :global(th) { background: rgba(255,255,255,0.06); color: #00d4ff; font-weight: 600; }
  .md-render :global(input[type="checkbox"]) { margin-right: 6px; }
  .md-render :global(.katex-display) { overflow-x: auto; margin: 8px 0; }
  .md-render :global(.mermaid-block) { background: rgba(255,255,255,0.03); border-radius: 8px; padding: 12px; margin: 8px 0; overflow-x: auto; }
  .md-render :global(.mermaid-block svg) { max-width: 100%; }
  .csv-render { overflow: auto; }
  .csv-render :global(table) { border-collapse: collapse; font-size: 12px; width: 100%; }
  .csv-render :global(th), .csv-render :global(td) {
    padding: 6px 10px; border: 1px solid rgba(255,255,255,0.1); text-align: left;
  }
  .csv-render :global(th) { background: rgba(255,255,255,0.06); color: #00d4ff; font-weight: 600; }
  .csv-render :global(td) { color: #e2e8f0; }

  /* Editor */
  .editor-wrap {
    flex: 1; display: flex; overflow: auto; -webkit-overflow-scrolling: touch; min-height: 0;
  }
  .editor-nums {
    padding: 12px 8px; text-align: right; color: rgba(226,232,240,0.2); font-family: 'SF Mono', Menlo, monospace;
    font-size: 13px; line-height: 1.5; white-space: pre; user-select: none; flex-shrink: 0;
    border-right: 1px solid rgba(255,255,255,0.06);
  }
  .editor-layer { position: relative; flex: 1; min-width: 0; }
  .editor-highlight {
    margin: 0; padding: 12px; font-family: 'SF Mono', Menlo, monospace; font-size: 13px;
    line-height: 1.5; white-space: pre-wrap; word-break: break-all; color: #e2e8f0;
    pointer-events: none;
  }
  .editor-highlight :global(code) { font-family: inherit; background: none; padding: 0; }
  .editor {
    position: absolute; inset: 0; width: 100%; height: 100%; padding: 12px; border: none; resize: none;
    background: transparent; color: transparent; caret-color: #e2e8f0;
    font-family: 'SF Mono', Menlo, monospace; font-size: 13px; line-height: 1.5; outline: none;
    white-space: pre-wrap; word-break: break-all;
  }

  /* Info */
  .info-body { flex: 1; overflow: auto; padding: 12px; }
  .info-row {
    display: flex; padding: 10px 0; border-bottom: 1px solid rgba(255,255,255,0.04);
  }
  .info-label { width: 100px; flex-shrink: 0; color: rgba(226,232,240,0.4); font-size: 12px; }
  .info-val { flex: 1; font-size: 13px; word-break: break-all; }
  .info-val.mono { font-family: 'SF Mono', Menlo, monospace; }
</style>
