import type { FileViewState } from './file-view-state.ts';

interface FileReference {
  path?: string;
  content?: unknown;
}

export interface FileLocation<F = FileReference, E = unknown> extends FileViewState<F> {
  cwd: string;
  entries: E[];
  fromGit: boolean;
  scroll: number;
  frameScroll: number;
}

interface BackContext<F> {
  cwd: string;
  currentFile: F | null;
  fromGit: boolean;
}

export type FileBackStep<L> =
  | { kind: 'restore'; location: L }
  | { kind: 'preview' }
  | { kind: 'git' }
  | { kind: 'list'; path: string };

/** Below directory history, tab visits climb; a chat jump returns to chat. */
export function directoryBackFloor(cwd: string, jumped: boolean): string {
  if (!jumped && cwd && cwd !== '/') return cwd.replace(/\/[^/]+\/?$/, '') || '/';
  return '';
}

/** Non-reactive history/request state, one instance per Files component.
 * The host owns RPCs, confirmation, animation and applying DOM scroll offsets. */
export function createFileNavigation<F extends FileReference = FileReference, E = unknown>() {
  let dirHist: string[] = [];
  // The browser model (board #187): Back pushes where you WERE here; Forward
  // pops it; any fresh navigation throws it away.
  let dirFwd: string[] = [];
  let fileHist: FileLocation<F, E>[] = [];
  let fileSeq = 0;

  function backFromPreview({ cwd, currentFile, fromGit }: BackContext<F>): FileBackStep<FileLocation<F, E>> {
    const previous = fileHist.pop();
    if (previous) return { kind: 'restore', location: previous };
    if (fromGit) return { kind: 'git' };
    return { kind: 'list', path: currentFile?.path?.replace(/\/[^/]+$/, '') || cwd };
  }

  return {
    rememberDirectory(cwd: string, path: string): void {
      if (cwd && path !== cwd) { dirHist.push(cwd); dirFwd = []; }
    },
    /** Back: the previous directory, or undefined at the start. `from` (where
     * the user is now) becomes the Forward step. */
    popDirectory(from = ''): string | undefined {
      const prev = dirHist.pop();
      if (prev != null && from) dirFwd.push(from);
      return prev;
    },
    /** Forward: undoes the last Back; `from` becomes a Back step again. */
    forwardDirectory(from = ''): string | undefined {
      const next = dirFwd.pop();
      if (next != null && from) dirHist.push(from);
      return next;
    },
    canGoBack(): boolean { return dirHist.length > 0; },
    canGoForward(): boolean { return dirFwd.length > 0; },
    resetDirectories(): void { dirHist = []; dirFwd = []; },
    rememberFile(location: FileLocation<F, E>): void { fileHist.push(location); },
    resetFiles(): void { fileHist = []; },
    nextFile(): number { return ++fileSeq; },
    isCurrentFile(request: number): boolean { return request === fileSeq; },
    backFromPreview,
    backFromInfo(context: BackContext<F>): FileBackStep<FileLocation<F, E>> {
      if (context.currentFile?.content != null) return { kind: 'preview' };
      return backFromPreview(context);
    },
  };
}
