<script lang="ts">
  // Git panel: status / staging / commit / push / log / diff for the
  // directory the file browser is currently in. Extracted from Files.svelte —
  // it owns all git state; the host only decides when the panel is shown.
  //
  // Contract with the host (Files.svelte):
  // - mounts fresh each time the git view opens (state resets naturally);
  //   loads `git status` on init.
  // - `onOpenFile(entry)`: untracked/binary/image files can't be shown as a
  //   text diff — ask the host to open them in its regular file preview.
  // - `onClose()`: user backed out of the panel root (host switches view).
  // - `goBack()` (instance export): host's back-navigation calls this first;
  //   returns true if the panel consumed the back (closed an open diff).
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { gitCmd } from '../core/ws.ts';

  type GitFile = { status: string; file: string };
  type GitCommit = { hash: string; subject: string; date: string; author: string };
  type GitDiff = { file: string; diff: string; _root: string };
  type PreviewEntry = { type: 'file'; path: string; name: string };

  let { cwd, fontSize = 14, onOpenFile, onClose }: {
    cwd: string;
    fontSize?: number;
    onOpenFile: (entry: PreviewEntry) => void;
    onClose: () => void;
  } = $props();

  let gitTab = $state<'status' | 'log'>('status');
  let gitStatus = $state<GitFile[]>([]);
  let gitLog = $state<GitCommit[]>([]);
  let gitBranch = $state('');
  let gitDiff = $state<GitDiff | null>(null);
  let gitLoading = $state(false);
  let gitListEl = $state<HTMLElement | null>(null);
  let gitError = $state('');
  let pushResult = $state('');
  let commitMsg = $state('');
  let showCommitInput = $state(false);

  let gitRoot = '';

  export function goBack() {
    if (gitDiff) { gitDiff = null; return true; }
    return false;
  }

  async function git(subcmd: string, ...args: string[]): Promise<string> {
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
    } catch (e) { gitError = (e as Error).message; }
    gitLoading = false;
    requestAnimationFrame(() => { if (gitListEl) gitListEl.scrollTop = scrollTop; });
  }

  async function loadGitLog() {
    gitLoading = true;
    gitError = '';
    try {
      const out = await git('log', '--oneline', '-30', '--format=%h|%s|%ar|%an');
      gitLog = out.split('\n').filter(Boolean).map(line => {
        const [hash = '', subject = '', date = '', author = ''] = line.split('|');
        return { hash, subject, date, author };
      });
    } catch (e) { gitError = (e as Error).message; }
    gitLoading = false;
  }

  async function showFileDiff(file: string, staged: boolean) {
    gitLoading = true;
    try {
      const isUntracked = gitStatus.find(f => f.file === file)?.status === '??';
      const isBinary = !isUntracked && (await git('diff', '--numstat', ...(staged ? ['--cached'] : []), '--', file)).startsWith('-');
      if (isUntracked || isBinary || file.match(/\.(png|jpe?g|gif|webp|svg|ico|bmp|avif|pdf|zip|tar|gz|mp[34]|mov|wav)$/i)) {
        gitLoading = false;
        onOpenFile({ type: 'file', path: gitRoot + '/' + file, name: file.split('/').pop() ?? file });
        return;
      }
      const diff = await git('diff', ...(staged ? ['--cached'] : []), '--', file);
      gitDiff = { file, diff: diff || '(no changes)', _root: gitRoot };
    } catch (e) { gitDiff = { file, diff: (e as Error).message, _root: '' }; }
    gitLoading = false;
  }

  async function showCommitDiff(hash: string) {
    gitLoading = true;
    try {
      const diff = await git('show', '--stat', '--patch', hash);
      gitDiff = { file: hash, diff, _root: gitRoot };
    } catch (e) { gitDiff = { file: hash, diff: (e as Error).message, _root: '' }; }
    gitLoading = false;
  }

  async function gitPush() {
    gitLoading = true;
    pushResult = '';
    try {
      const r = await git('push');
      pushResult = r.trim() || 'Pushed';
    } catch (e) { pushResult = '✗ ' + (e as Error).message; }
    gitLoading = false;
    setTimeout(() => { pushResult = ''; }, 3000);
  }

  async function gitAddAll() {
    try { await git('add', '.'); } catch {}
    loadGitStatus();
  }

  async function gitAddFile(file: string) {
    try { await git('add', file); } catch {}
    loadGitStatus();
  }

  async function gitRestoreFile(file: string) {
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
    } catch (e) { pushResult = '✗ ' + (e as Error).message; }
    gitLoading = false;
    setTimeout(() => { pushResult = ''; }, 3000);
  }

  // Initial load on mount (the panel mounts each time the git view opens).
  loadGitStatus();
</script>

<div class="preview-header">
  <button class="back-btn" onclick={() => { if (gitDiff) gitDiff = null; else onClose(); }}><Icon name="chevron-left" size={16} /></button>
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
    <button class="git-act-btn" onclick={gitAddAll}>{t('addAll')}</button>
    <button class="git-act-btn" onclick={() => showCommitInput = !showCommitInput}>{t('commit')}{stagedCount ? ` (${stagedCount})` : ''}</button>
    <button class="git-act-btn" onclick={gitPush} disabled={gitLoading}>{t('push')}</button>
  </div>
  {#if showCommitInput}
    <div class="git-commit-row">
      <textarea bind:value={commitMsg} placeholder={t('commitMsg')} onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) { e.preventDefault(); gitCommit(); } }} rows="2"></textarea>
      <button class="git-commit-btn" onclick={gitCommit} disabled={!commitMsg.trim()}>OK</button>
    </div>
  {/if}
  <div class="git-tabs">
    <button class:active={gitTab === 'status'} onclick={() => { gitTab = 'status'; loadGitStatus(); }}>{t('status')}</button>
    <button class:active={gitTab === 'log'} onclick={() => { gitTab = 'log'; loadGitLog(); }}>{t('log')}</button>
  </div>
  {#if gitError}
    <div class="git-error">{gitError}</div>
  {/if}
  {#if gitLoading}
    <div class="git-loading">{t('gitLoading')}</div>
  {:else if gitTab === 'status'}
    <div class="git-list" bind:this={gitListEl}>
      {#if !gitStatus.length}
        <div class="empty">{t('cleanTree')}</div>
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
      const d = gitDiff;
      if (d?._root) { onOpenFile({ type: 'file', path: d._root + '/' + d.file, name: d.file.split('/').pop() ?? d.file }); }
    }}><Icon name="eye" size={14} /></button>
  </div>
  <div class="git-diff-body" style="--file-font-size:{fontSize}px">
    {#each gitDiff.diff.split('\n') as line}
      <div class="diff-line" class:diff-add={line.startsWith('+') && !line.startsWith('+++')} class:diff-del={line.startsWith('-') && !line.startsWith('---')} class:diff-hunk={line.startsWith('@@')} class:diff-meta={line.startsWith('diff ') || line.startsWith('index ') || line.startsWith('---') || line.startsWith('+++')}><span class="diff-text">{line}</span></div>
    {/each}
  </div>
{/if}

<style>
  /* Header chrome — same look as Files' preview header (duplicated until a
     shared ui/ primitive exists; keep the two in sync). */
  .preview-header {
    display: flex; align-items: center; gap: 8px; padding: 8px 10px;
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .back-btn {
    padding: 6px; border: none; border-radius: var(--ui-radius-control); background: var(--surface2);
    color: var(--text2); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .preview-name {
    flex: 1; font-size: var(--fs-body); font-weight: 500; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap;
  }
  .preview-actions { display: flex; gap: 4px; }
  .act-btn {
    padding: 6px; border: none; border-radius: var(--ui-radius-control); background: none;
    color: var(--text3); cursor: pointer; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .act-btn:active { color: var(--accent); }
  .empty { padding: 40px; text-align: center; color: var(--text3); font-size: var(--fs-body); }

  .git-tabs {
    display: flex; gap: 2px; padding: 6px 10px; border-bottom: 1px solid var(--border);
    background: var(--pill-bg); flex-shrink: 0;
  }
  .git-tabs button {
    padding: 6px 16px; border: none; border-radius: var(--ui-radius-control); background: transparent;
    color: var(--text3); font-size: var(--fs-ui); font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .git-tabs button.active { background: var(--accent-bg); color: var(--accent); }
  .git-error { padding: 10px; color: var(--danger); font-size: var(--fs-ui); background: var(--danger-bg); }
  .git-push-result { padding: 8px 12px; font-size: var(--fs-ui); color: var(--status-ok); background: var(--accent-bg); flex-shrink: 0; }
  .git-push-result.git-error { color: var(--danger); background: var(--danger-bg); }
  .git-actions {
    display: flex; gap: 4px; padding: 6px 10px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .git-act-btn {
    padding: 5px 12px; border: 1px solid var(--border); border-radius: var(--ui-radius-control);
    background: var(--surface2); color: var(--text2); font-size: var(--fs-ui); font-weight: 500;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .git-act-btn:active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }
  .git-act-btn:disabled { opacity: 0.4; }
  .git-commit-row {
    display: flex; gap: 6px; padding: 6px 10px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .git-commit-row textarea {
    flex: 1; padding: 6px 10px; border: 1px solid var(--input-border); border-radius: var(--ui-radius-control);
    background: var(--input-bg); color: var(--text); font-size: var(--fs-body); outline: none;
    font-family: inherit; resize: none; line-height: 1.4;
  }
  .git-commit-row textarea:focus { border-color: var(--accent); }
  .git-commit-btn {
    padding: 6px 14px; border: none; border-radius: var(--ui-radius-control);
    background: var(--accent-fill); color: var(--accent-fill-ink); font-size: var(--fs-ui); font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .git-commit-btn:disabled { opacity: 0.4; }
  .git-file-row { display: flex; align-items: center; border-bottom: 1px solid var(--border2); }
  .git-file-row .git-file { flex: 1; border-bottom: none; }
  .git-stage-btn {
    width: 32px; height: 32px; border: none; border-radius: var(--ui-radius-control);
    /* The label is a +/− GLYPH acting as an icon, so it takes the largest
       chrome step rather than an off-scale size of its own. */
    background: none; color: var(--accent); font-size: var(--fs-title); font-weight: 600;
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    flex-shrink: 0; -webkit-tap-highlight-color: transparent;
  }
  .git-stage-btn:active { background: var(--accent-bg); }
  .git-loading { padding: 20px; text-align: center; color: var(--text3); font-size: var(--fs-body); }
  .git-list { flex: 1; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  .git-file {
    display: flex; align-items: center; gap: 8px; width: 100%;
    padding: 10px 12px; border: none; background: none;
    border-bottom: 1px solid var(--border2); cursor: pointer;
    text-align: left; color: var(--text); font-size: var(--fs-body);
    -webkit-tap-highlight-color: transparent;
  }
  .git-file:active { background: var(--accent-bg); }
  .git-st {
    font-family: var(--font-mono); font-size: var(--fs-ui); font-weight: 600;
    min-width: 24px; color: var(--text3);
  }
  .git-st.git-add { color: var(--status-ok); }
  .git-st.git-mod { color: var(--status-warn); }
  .git-st.git-del { color: var(--danger); }
  .git-fname {
    flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font-family: var(--font-mono); font-size: var(--fs-ui);
  }
  .git-hash {
    font-family: var(--font-mono); font-size: var(--fs-sub);
    color: var(--accent); min-width: 56px;
  }
  .git-date { font-size: var(--fs-sub); color: var(--text3); white-space: nowrap; }
  .git-diff-header {
    display: flex; align-items: center; gap: 8px;
    padding: 8px 12px; font-family: var(--font-mono); font-size: var(--fs-ui);
    color: var(--accent); background: var(--accent-bg); border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .git-diff-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .git-diff-body {
    flex: 1; overflow: auto; -webkit-overflow-scrolling: touch; background: var(--code-bg);
    font-family: var(--font-mono);
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
