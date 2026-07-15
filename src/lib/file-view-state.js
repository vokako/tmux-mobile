export function directoryLoadState({ view, currentFile }, purpose) {
  if (purpose !== 'navigate') return { view, currentFile };
  return { view: 'list', currentFile: null };
}
