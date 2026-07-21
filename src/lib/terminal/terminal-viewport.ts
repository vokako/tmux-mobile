export function restoreViewportAfterPaneSwitch(
  { isMobile, fullHeight, root }: { isMobile: boolean; fullHeight: number; root: HTMLElement },
): void {
  if (!isMobile) return;
  root.style.setProperty('--app-height', fullHeight + 'px');
  root.classList.remove('keyboard-open');
}
