// Clipboard copy with a fallback for insecure contexts.
//
// navigator.clipboard.writeText requires a secure context (https / localhost /
// file://). LAN-accessed dev builds over http://<ip>:5173 — and anything else
// the user reaches over plain HTTP — either see navigator.clipboard as
// undefined (iOS Safari) or see writeText() reject with NotAllowedError.
//
// Fall back to a throwaway <textarea> + document.execCommand('copy'). It's
// deprecated but works everywhere that actually matters (including iOS Safari
// insecure contexts). If both paths fail we resolve false so callers can show
// a useful error instead of silently swallowing the problem.

export async function copyText(text) {
  if (text == null) return false;
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the execCommand path
  }
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.top = '0';
    ta.style.left = '0';
    ta.style.opacity = '0';
    ta.style.pointerEvents = 'none';
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    ta.setSelectionRange(0, ta.value.length);
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
