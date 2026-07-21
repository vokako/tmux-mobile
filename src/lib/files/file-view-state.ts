export interface FileViewState<F> {
  view: string;
  currentFile: F | null;
}

export function directoryLoadState<F>(
  { view, currentFile }: FileViewState<F>,
  purpose: string,
): FileViewState<F> {
  if (purpose !== 'navigate') return { view, currentFile };
  return { view: 'list', currentFile: null };
}
