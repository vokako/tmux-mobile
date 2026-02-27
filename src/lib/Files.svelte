<script>
  import Icon from './Icon.svelte';
  import { fsCwd, fsList, fsStat, fsRead, fsWrite, fsMkdir, fsDelete, fsRename, fsDownload, fsUpload } from './ws.js';

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
    if (entry.file_type === 'dir') {
      loadDir(entry.path);
      return;
    }
    loading = true;
    try {
      const stat = await fsStat(entry.path);
      currentFile = { path: entry.path, name: entry.name, stat };
      if (stat.is_text && stat.size <= 512 * 1024) {
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
    return entry.file_type === 'dir' ? 'folder' : 'file';
  }

  function mimeCategory(mime) {
    if (!mime) return 'other';
    if (mime.startsWith('image/')) return 'image';
    if (mime === 'text/markdown') return 'markdown';
    if (mime === 'text/csv') return 'csv';
    return 'other';
  }

  function renderMarkdown(text) {
    return text
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/^### (.+)$/gm, '<h3>$1</h3>')
      .replace(/^## (.+)$/gm, '<h2>$1</h2>')
      .replace(/^# (.+)$/gm, '<h1>$1</h1>')
      .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
      .replace(/\*(.+?)\*/g, '<em>$1</em>')
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      .replace(/^- (.+)$/gm, '<li>$1</li>')
      .replace(/\n/g, '<br>');
  }

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
      <div class="bc-path">
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
      <label class="tool-toggle">
        <input type="checkbox" bind:checked={showHidden} onchange={() => loadDir(cwd)} /> Hidden
      </label>
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
              <span class="file-name" class:dir-name={entry.file_type === 'dir'}>{entry.name}</span>
              {#if entry.file_type !== 'dir'}
                <span class="file-size">{formatSize(entry.size)}</span>
              {/if}
            </button>
            <div class="file-actions">
              {#if entry.file_type !== 'dir'}
                <button class="act-btn" onclick={() => handleDownload(entry.path)} title="Download"><Icon name="download" size={12} /></button>
              {/if}
              <button class="act-btn" onclick={() => { renaming = entry.path; renameValue = entry.name; }} title="Rename"><Icon name="edit" size={12} /></button>
              <button class="act-btn del" class:confirm={confirmDelete === entry.path} onclick={() => handleDelete(entry.path)} title="Delete">
                <Icon name="trash" size={12} />
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
        <div class="md-render">{@html renderMarkdown(currentFile.content)}</div>
      {:else if mimeCategory(currentFile.stat?.mime_hint) === 'csv'}
        <div class="csv-render">{@html renderCsv(currentFile.content)}</div>
      {:else}
        <pre class="code-preview">{currentFile.content}</pre>
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
    <textarea
      class="editor"
      value={editContent}
      oninput={onEditInput}
      spellcheck="false"
      autocapitalize="off"
      autocomplete="off"
    ></textarea>

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
  .tool-toggle {
    margin-left: auto; font-size: 11px; color: rgba(226,232,240,0.4);
    display: flex; align-items: center; gap: 4px;
  }
  .tool-toggle input { width: 14px; height: 14px; }

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
    line-height: 1.5; color: #e2e8f0; white-space: pre-wrap; word-break: break-all;
  }
  .md-render { font-size: 14px; line-height: 1.6; color: #e2e8f0; }
  .md-render :global(h1) { font-size: 20px; margin: 12px 0 8px; color: #00d4ff; }
  .md-render :global(h2) { font-size: 17px; margin: 10px 0 6px; color: #00d4ff; }
  .md-render :global(h3) { font-size: 15px; margin: 8px 0 4px; color: #00d4ff; }
  .md-render :global(code) { background: rgba(255,255,255,0.08); padding: 2px 5px; border-radius: 3px; font-size: 12px; }
  .md-render :global(strong) { color: #fff; }
  .md-render :global(li) { margin-left: 16px; }
  .csv-render { overflow: auto; }
  .csv-render :global(table) { border-collapse: collapse; font-size: 12px; width: 100%; }
  .csv-render :global(th), .csv-render :global(td) {
    padding: 6px 10px; border: 1px solid rgba(255,255,255,0.1); text-align: left;
  }
  .csv-render :global(th) { background: rgba(255,255,255,0.06); color: #00d4ff; font-weight: 600; }
  .csv-render :global(td) { color: #e2e8f0; }

  /* Editor */
  .editor {
    flex: 1; width: 100%; padding: 12px; border: none; resize: none;
    background: #0a0a0f; color: #e2e8f0; font-family: 'SF Mono', Menlo, monospace;
    font-size: 13px; line-height: 1.5; outline: none;
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
