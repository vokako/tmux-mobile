# Guidance: Security

> Tenets: 8 (validate once at entry), 2 (token = shell access is deliberate;
> document that boundary).
> Review questions: **Where does this string come from and go, and which
> parsers handle it? Where could a secret end up? Who supplied this HTML?**
> A focused review on 2026-09-03 fixed eleven security issues in one day,
> showing that ordinary review had missed this discipline.
> Draft · 2026-09-09.

## 1. Principles

1. **One validator per name/path/URL entry point:** its allowlist is the
   intersection of every parser's constraints along the route. An agent
   name is a directory component, tmux target, CLI argument and addressed
   mention (`@address`).
2. **Text escaping does not make attributes safe:** escaping `&`/`<`
   makes text inert, but `"` ends an `href` attribute and `javascript:`
   is executable in scheme position. Each context needs its own guard.
3. **One renderer:** all Markdown uses `core/markdown` and its `marked`
   singleton with `markedSafeUrl` registered. CSP `script-src 'self'`
   is a second defense, not the first.
4. **No secrets in stdout, repositories or logs:** tokens, API keys and
   keystore passwords.
5. **Document deliberately broad capabilities:** `fs_*`/`/dl` accept any
   absolute path, and the git allowlist includes push/commit, because
   "token = shell access". The allowlist must not imply restrictions it
   does not enforce.
6. **Use cryptographic primitives as specified:** one owner per cipher
   direction, one outbound FIFO, separate directional keys and no nonce
   reuse. Compare tokens in constant time.

## 2. Required and Forbidden

**Required**
- Apply `valid_name` at name entry points. Reject path escapes such as `..`
  or absolute paths inserted under a relative root; sanitize filenames
  through `sanitize_filename` first.
- Markdown escapes `&` and `<`, never `>`. `link`/`image` permit only
  `http`/`https`/`mailto` (images only `http`/`https`). Decode entities and
  remove control characters before classifying schemes.
- Never combine iframe `allow-scripts` and `allow-same-origin`. Scope the
  opener capability to specific directories.
- Pass git arguments through `Command::args` without a shell; reject only NUL.
- Read HTTP requests through `\r\n\r\n`. Support `Range` for `/dl`, re-sign
  retries because signatures expire after 60s, and flush `BufStream`
  before returning from peek-then-dispatch.
- E2E uses separate directional keys and constant-time comparisons;
  `InitCipher` and data use the same queue.
- Git-ignore Android signing `key.properties`; build scripts contain no passwords.
- Invoke hook helpers through `/bin/sh`; macOS provenance enforcement
  can kill directly executed scripts with exit 137.

**Forbidden**
- A second `marked.parse`, shell quoter or name validator.
- Printing tokens or keys into server logs, `tmm` output or room messages.
- Comparing secrets with `==`.
- Treating descriptions, window names or process names as proof of managed identity.
- Patching delimiter problems with blacklists; the next parser will expose another gap.

## 3. Review Checklist

- [ ] Which parsers process the new string? Does the entry validator
  enforce the intersection of their constraints?
- [ ] Does rendered HTML go through `core/markdown`, with guards for attribute contexts?
- [ ] Could a secret reach stdout, logs, rooms or the repository?
- [ ] Is a new iframe, opener or download capability broader than before? Why?
- [ ] Are cryptographic comparisons constant-time, with correct directional keys?
- [ ] Are deletion targets constructed by controlled functions such as `agents::home_dir`?
- [ ] Was a CSP change in `tauri.conf.json` smoke-tested on a real device
  with Markdown, PDF, Mermaid and Hub?

## 4. Lessons

All are from the focused 2026-09-03 review, board #80-#84:

- `[x](javascript:)` reached the DOM, and `https://a.b/x"onclick=` became
  an autolink containing an `onclick` attribute. Both led to `markedSafeUrl`.
- Files' separate `marked.parse` let README `<img onerror>` execute in the
  token-bearing origin; use one renderer.
- `agent_remove("../..")` resolved to the workspace itself; enforce `valid_name`.
- `keystore.jks` and its password were in the Gradle script. They were removed
  from the repository, but the key was not rotated by owner decision and
  remains in history.
- Tokens appeared in supervised logs, token comparison used `==`, and both
  E2E directions shared a key and nonce.
- Opener scope was `**`; it was narrowed and iframe sandboxing tightened.
- CSP was added without a local webview, so a real-device smoke test remains
  necessary. Browser/PWA responses have no CSP header (todo).
