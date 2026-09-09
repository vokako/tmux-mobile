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
      if (cwd && path !== cwd) dirHist.push(cwd);
    },
    popDirectory(): string | undefined { return dirHist.pop(); },
    resetDirectories(): void { dirHist = []; },
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
