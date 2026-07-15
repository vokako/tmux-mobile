export function restoreViewportAfterPaneSwitch({ isMobile, fullHeight, root }) {
  if (!isMobile) return;
  root.style.setProperty('--app-height', fullHeight + 'px');
  root.classList.remove('keyboard-open');
}
