// WebSocket client for tmux-mobile server

// ─── Wire framing for the encrypted binary path ─────────────────────────
// Encrypted frames travel as binary; the plaintext (post-decrypt) starts
// with a 1-byte tag telling us how to decode the rest:
//   0x00 = raw UTF-8 JSON
//   0x01 = raw deflate (RFC 1951) of UTF-8 JSON
// Plaintext-token connections (no Web Crypto) keep using TEXT frames with
// no framing.
const WIRE_PLAIN_JSON = 0x00;
const WIRE_DEFLATE_JSON = 0x01;
// Same threshold as the server: below this, deflate's overhead loses to
// the input size, so we just send plaintext.
const COMPRESS_MIN_BYTES = 256;

const CONNECT_TIMEOUT_MS = 5000;
// Default timeout for ordinary RPCs. Long-running methods (fs_download,
// fs_upload) override with a much larger value at their call site.
// Short default → consecutive timeouts feel a dead link quickly AND the
// UI flips to "Reconnecting" fast. See call() for the interplay with
// `pending.size` gating.
const RPC_TIMEOUT_MS = 6000;
const BASE64_CHUNK_SIZE = 8192;

// Idle-ping threshold. The server pushes pane snapshots at 200 ms and
// PINGs at 15 s, so under normal conditions the link sees inbound traffic
// every few hundred ms even when the user is idle. If we go this long
// without hearing anything, the link is plausibly half-open (mobile
// network parked the TCP socket without telling us) — issue a ping RPC
// so onclose fires within ~6 s instead of waiting 45 s for the server's
// own PING deadline.
const IDLE_PROBE_THRESHOLD_MS = 8000;
const IDLE_PROBE_INTERVAL_MS = 4000;  // how often we check the threshold

// Consecutive RPC timeouts only force a disconnect when the inbound channel
// has ALSO been silent this long. On a high-RTT link (bad cellular, 5-7 s
// round trips) small RPCs time out while pane_output pushes keep arriving —
// the link is alive, just slow; tearing it down and re-handshaking makes
// things strictly worse.
const TIMEOUT_DISCONNECT_INBOUND_SILENCE_MS = 10000;

// Liveness is handled at the WebSocket protocol layer: the server sends
// PING frames periodically and the browser auto-replies with PONG at a
// layer we never touch. When TCP really dies, `ws.onclose` fires and the
// app layer reacts via `onDisconnect`. Client-side JSON-RPC "ping" RPCs
// are no longer needed here.

let ws = null;
let wsUrl = null;
let requestId = 0;
const pending = new Map();
// Per-target listener registries. Each mounted Terminal registers a callback
// keyed by its own target, so multiple terminals (split-screen) coexist —
// pane_output / pane_closed are routed to the matching listeners instead of a
// single shared callback that instances would overwrite.
//
// Value is a Set of callbacks, NOT a single cb: two split cells can show the
// SAME target (same window), and both must receive every push. With a single
// cb the second registration silently replaced the first, so only one cell
// refreshed. A Set fans out to all, and removing one cell's cb leaves the
// other's intact.
const paneOutputListeners = new Map(); // target -> Set<cb(target, content, cursor, current_command)>
const paneClosedListeners = new Map(); // target -> Set<cb(target)>
// Team group-chat message push listeners (Team tab). Unkeyed — one stream
// for the whole room — so a plain Set, not a per-target map.
const teamMessageListeners = new Set(); // Set<cb(message)>

function addListener(map, target, cb) {
  let set = map.get(target);
  if (!set) { set = new Set(); map.set(target, set); }
  set.add(cb);
}
function removeListener(map, target, cb) {
  const set = map.get(target);
  if (!set) return;
  set.delete(cb);
  if (set.size === 0) map.delete(target);
}
let onDisconnect = null;
let recoveryEnabled = false;
let disconnectNotified = false;
// Idle-probe state. lastInboundAt is updated on every inbound message
// (any frame: handshake, RPC reply, push). idleProbeTimer is the periodic
// checker that fires a `ping` RPC if nothing has arrived for too long.
let lastInboundAt = 0;
let idleProbeTimer = null;
let idleProbeInFlight = false;

// These take the cb so the caller can register/unregister its own listener
// without disturbing other cells on the same target. Callers MUST pass the
// same function reference to remove that they passed to add.
export function addPaneOutputListener(target, cb) { addListener(paneOutputListeners, target, cb); }
export function removePaneOutputListener(target, cb) { removeListener(paneOutputListeners, target, cb); }
export function addPaneClosedListener(target, cb) { addListener(paneClosedListeners, target, cb); }
export function removePaneClosedListener(target, cb) { removeListener(paneClosedListeners, target, cb); }
export function addTeamMessageListener(cb) { teamMessageListeners.add(cb); }
export function removeTeamMessageListener(cb) { teamMessageListeners.delete(cb); }
export function setOnDisconnect(cb) { onDisconnect = cb; }

function notifyDisconnect(reason) {
  if (!recoveryEnabled || disconnectNotified) return;
  disconnectNotified = true;
  window.__dbg?.(`ws: recovery requested (${reason})`);
  onDisconnect?.();
}

// --- Crypto helpers (Web Crypto API) ---

function hexToBytes(hex) {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) bytes[i / 2] = parseInt(hex.substr(i, 2), 16);
  return bytes;
}

function bytesToHex(bytes) {
  return Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join('');
}

async function deriveKey(token, serverNonce, clientNonce) {
  const salt = new Uint8Array(32);
  salt.set(serverNonce, 0);
  salt.set(clientNonce, 16);
  const ikm = new TextEncoder().encode(token);
  const baseKey = await crypto.subtle.importKey('raw', ikm, 'HKDF', false, ['deriveBits']);
  const bits = await crypto.subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt, info: new TextEncoder().encode('tmux-mobile-e2e') }, baseKey, 256);
  return crypto.subtle.importKey('raw', bits, { name: 'AES-GCM' }, false, ['encrypt', 'decrypt']);
}

async function computeProof(token, serverNonce, clientNonce) {
  const salt = new Uint8Array(32);
  salt.set(serverNonce, 0);
  salt.set(clientNonce, 16);
  const ikm = new TextEncoder().encode(token);
  const baseKey = await crypto.subtle.importKey('raw', ikm, 'HKDF', false, ['deriveBits']);
  const keyBits = await crypto.subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt, info: new TextEncoder().encode('tmux-mobile-e2e') }, baseKey, 256);
  const hmacKey = await crypto.subtle.importKey('raw', keyBits, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  // Bind proof to BOTH nonces (see server-side comment in handle auth).
  const msg = new Uint8Array(serverNonce.length + clientNonce.length);
  msg.set(serverNonce, 0);
  msg.set(clientNonce, serverNonce.length);
  const sig = await crypto.subtle.sign('HMAC', hmacKey, msg);
  return new Uint8Array(sig);
}

function makeNonce(counter) {
  const n = new Uint8Array(12);
  const view = new DataView(n.buffer);
  // counter in bytes 4-11 (big-endian u64)
  view.setUint32(4, Math.floor(counter / 0x100000000));
  view.setUint32(8, counter >>> 0);
  return n;
}

// Compress a JSON string into the wire-plaintext byte stream:
// [framing byte] [body bytes]. Returns Uint8Array. Falls back to plain
// when the input is small or compresses to no benefit.
async function encodeWirePayload(text) {
  const utf8 = new TextEncoder().encode(text);
  if (utf8.length < COMPRESS_MIN_BYTES || typeof CompressionStream === 'undefined') {
    const out = new Uint8Array(1 + utf8.length);
    out[0] = WIRE_PLAIN_JSON;
    out.set(utf8, 1);
    return out;
  }
  // CompressionStream is native (zlib via the platform), much faster than
  // any JS deflate library and zero CPU on V8's JIT.
  const stream = new Blob([utf8]).stream().pipeThrough(new CompressionStream('deflate-raw'));
  const compressed = new Uint8Array(await new Response(stream).arrayBuffer());
  if (compressed.length + 1 >= utf8.length + 1) {
    // Pathological: compressing made it bigger. Fall back to plain.
    const out = new Uint8Array(1 + utf8.length);
    out[0] = WIRE_PLAIN_JSON;
    out.set(utf8, 1);
    return out;
  }
  const out = new Uint8Array(1 + compressed.length);
  out[0] = WIRE_DEFLATE_JSON;
  out.set(compressed, 1);
  return out;
}

// Inverse of encodeWirePayload: decode a wire-plaintext byte buffer
// (framing byte + body) back into the original JSON string.
async function decodeWirePayload(bytes) {
  if (!bytes || bytes.length < 1) throw new Error('empty wire payload');
  const tag = bytes[0];
  const body = bytes.subarray(1);
  if (tag === WIRE_PLAIN_JSON) {
    return new TextDecoder().decode(body);
  }
  if (tag === WIRE_DEFLATE_JSON) {
    if (typeof DecompressionStream === 'undefined') {
      throw new Error('server sent deflate but DecompressionStream is unavailable');
    }
    const stream = new Blob([body]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
    return new Response(stream).text();
  }
  throw new Error(`unknown wire framing tag: 0x${tag.toString(16)}`);
}

// Encrypt a JSON string and return a Uint8Array suitable for ws.send().
// The plaintext is the wire-framed payload (compressed or not).
async function encryptMsg(text, cipher) {
  if (!cipher) return text; // plain path: caller sends it as text
  const plaintext = await encodeWirePayload(text);
  const nonce = makeNonce(cipher.sendCounter++);
  const ct = await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce }, cipher.key, plaintext);
  return new Uint8Array(ct);
}

// Decrypt an inbound binary ciphertext into the original JSON string.
async function decryptMsg(buf, cipher) {
  if (!cipher) {
    // Plain-token fallback: buf is already the decoded text.
    return typeof buf === 'string' ? buf : new TextDecoder().decode(buf);
  }
  const ctBytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  const nonce = makeNonce(cipher.recvCounter++);
  const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: nonce }, cipher.key, ctBytes);
  return decodeWirePayload(new Uint8Array(pt));
}

function sendOnSocket(socket, message) {
  const send = socket._sendQueue.then(async () => {
    const out = await encryptMsg(message, socket._cipher);
    if (ws !== socket || socket.readyState !== WebSocket.OPEN) {
      const error = new Error('connection changed before send');
      error.code = 'DISCONNECTED';
      throw error;
    }
    socket.send(out);
  });
  // Keep the socket queue usable after one send fails; callers still receive
  // the original rejection through the returned promise.
  socket._sendQueue = send.catch(() => {});
  return send;
}

// Issue a ping RPC if the inbound channel has been silent past the
// threshold. ping is cheap (one RTT, ~zero bytes), and using a regular
// RPC means timeout / error handling already exists in `call()` — three
// consecutive failures already trigger forceDisconnect.
//
// Skip when:
//   - another idle probe is in flight (avoid stacking)
//   - a real RPC is already in flight (its response will reset the clock,
//     and stacking probes on top of a slow legitimate RPC is wasteful)
//   - the threshold hasn't elapsed (pane_output / RPC reply already
//     reset lastInboundAt)
function maybeIdleProbe() {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  if (idleProbeInFlight) return;
  if (pending.size > 0) return;
  if (Date.now() - lastInboundAt < IDLE_PROBE_THRESHOLD_MS) return;
  idleProbeInFlight = true;
  // Use a tighter timeout than RPC_TIMEOUT_MS — we already know the link
  // hasn't said anything in 8 s, no point waiting another 6.
  call('ping', {}, 4000).catch(() => {}).finally(() => {
    idleProbeInFlight = false;
  });
}

function startIdleProbe() {
  stopIdleProbe();
  lastInboundAt = Date.now();
  idleProbeTimer = setInterval(maybeIdleProbe, IDLE_PROBE_INTERVAL_MS);
}

function stopIdleProbe() {
  if (idleProbeTimer) clearInterval(idleProbeTimer);
  idleProbeTimer = null;
  idleProbeInFlight = false;
}

function rejectAllPending(reason) {
  const err = new Error(reason || 'disconnected');
  err.code = 'DISCONNECTED';
  for (const { reject: rej } of pending.values()) rej(err);
  pending.clear();
}

export function connect(url, token, timeoutMs = CONNECT_TIMEOUT_MS) {
  // Close any existing connection before creating a new one.
  // IMPORTANT: null ALL handlers (including onmessage) so any in-flight events on
  // the old socket don't fire handlers that still close over module-level `ws` and
  // accidentally send frames on the NEW socket (common race during reconnect).
  if (ws) {
    try {
      ws.onclose = null;
      ws.onerror = null;
      ws.onmessage = null;
      ws.onopen = null;
      ws.close();
    } catch {}
    ws = null;
    stopIdleProbe();
    rejectAllPending('superseded by new connect');
  }
  recoveryEnabled = false;
  disconnectNotified = false;
  rpcTimeouts = 0;
  wsUrl = url;
  window.__dbg?.(`ws: connecting to ${url} (timeout=${timeoutMs}ms)`);

  return new Promise((resolve, reject) => {
    let socket;
    try {
      ws = new WebSocket(url);
      socket = ws;
      // Every handler below closes over this identity. A stale close or an
      // async encrypted send from an older connection must never mutate or
      // write through the module-level pointer after reconnect replaced it.
      // Receive ciphertext as ArrayBuffer rather than Blob so we can decrypt
      // synchronously without a Blob → arrayBuffer round-trip per message.
      socket.binaryType = 'arraybuffer';
      socket._sendQueue = Promise.resolve();
      socket._cipher = null;
    } catch (e) {
      window.__dbg?.(`ws: connect error: ${e.message}`);
      reject(e);
      return;
    }

    const timeout = setTimeout(() => {
      window.__dbg?.('ws: connect timeout');
      try { socket?.close(); } catch {}
      reject(new Error('connection timeout'));
    }, timeoutMs);

    let authed = false;
    let cipher = null;
    let serverNonce = null;
    let machineId = null;
    let hostname = null;

    function authSuccess() {
      clearTimeout(timeout);
      authed = true;
      socket._cipher = cipher;
      recoveryEnabled = true;
      disconnectNotified = false;
      window.__dbg?.(`ws: authenticated (machine=${machineId})`);
      // Liveness is the server's job at the protocol layer: it sends WS
      // PING every 15 s and tears down TCP after a 45 s deadline. That
      // surfaces here as `ws.onclose`. We add an application-layer idle
      // probe (below) so we trip onclose within ~14 s instead of 45 s
      // when the link goes half-open (common on mobile networks).
      startIdleProbe();
      resolve(machineId);
    }

    // Expose getters
    socket._getMachineId = () => machineId;
    socket._getHostname = () => hostname;

    socket.onmessage = async (event) => {
      if (ws !== socket) return;
      // Any inbound message resets the idle clock — the link is alive,
      // even if the message is just a handshake / push / heartbeat.
      lastInboundAt = Date.now();
      // event.data is either a string (text frames: handshake, plain auth
      // path) or an ArrayBuffer (binary frames: encrypted messages).
      const isBinary = event.data instanceof ArrayBuffer;
      let data;
      if (!isBinary) {
        try { data = JSON.parse(event.data); } catch {}
      }

      // Step 1: Receive server_nonce (always text)
      if (!authed && !serverNonce && data?.server_nonce) {
        serverNonce = hexToBytes(data.server_nonce);
        if (crypto.subtle) {
          // Encrypted auth
          const clientNonce = crypto.getRandomValues(new Uint8Array(16));
          const proof = await computeProof(token, serverNonce, clientNonce);
          const key = await deriveKey(token, serverNonce, clientNonce);
          if (ws !== socket) return;
          cipher = { key, sendCounter: 0, recvCounter: 0 };
          socket.send(JSON.stringify({
            method: 'auth',
            params: { client_nonce: bytesToHex(clientNonce), proof: bytesToHex(proof) }
          }));
        } else {
          // Fallback: plain token auth (http:// context, no Web Crypto)
          socket.send(JSON.stringify({ method: 'auth', params: { token } }));
        }
        return;
      }

      // Step 2: Auth response
      if (!authed && serverNonce) {
        if (cipher) {
          // Encrypted auth response — must arrive as a binary frame.
          if (!isBinary) {
            clearTimeout(timeout); cipher = null;
            reject(new Error('auth failed: expected binary auth response'));
            return;
          }
          try {
            const pt = await decryptMsg(event.data, cipher);
            if (ws !== socket) return;
            const resp = JSON.parse(pt);
            if (resp.result?.authenticated) { machineId = resp.result.machine_id; hostname = resp.result.hostname; authSuccess(); return; }
          } catch {}
          clearTimeout(timeout); cipher = null; reject(new Error('auth failed')); return;
        } else {
          // Plain (token) response — text frame.
          if (data?.result?.authenticated) { machineId = data.result.machine_id; hostname = data.result.hostname; authSuccess(); return; }
          clearTimeout(timeout); reject(new Error(data?.error?.message || 'auth failed')); return;
        }
      }

      // Post-auth: every encrypted message arrives as a binary frame and
      // gets decrypted + wire-decoded into JSON. Plain-token connections
      // continue to receive text frames (data is already parsed above).
      if (authed && cipher) {
        if (!isBinary) return; // ignore unexpected text frames after encrypted auth
        let pt;
        try { pt = await decryptMsg(event.data, cipher); } catch { return; }
        if (ws !== socket) return;
        try { data = JSON.parse(pt); } catch { return; }
      }

      if (!data) return;

      if (data.method === 'pane_output') {
        // current_command is included only on the first push and when it
        // actually changes — most ticks omit it. Fan out to EVERY listener
        // on this target (multiple split cells may show the same window).
        const tgt = data.params?.target;
        const set = paneOutputListeners.get(tgt);
        if (set) for (const cb of set) cb(tgt, data.params?.content, data.params?.cursor, data.params?.current_command);
        return;
      }

      if (data.method === 'pane_closed') {
        const tgt = data.params?.target;
        const set = paneClosedListeners.get(tgt);
        if (set) for (const cb of set) cb(tgt);
        return;
      }

      if (data.method === 'team_message') {
        const m = data.params?.message;
        if (m) for (const cb of teamMessageListeners) cb(m);
        return;
      }

      if (data.id != null && pending.has(data.id)) {
        const { resolve: res, reject: rej } = pending.get(data.id);
        pending.delete(data.id);
        if (data.error) {
          const err = new Error(data.error.message);
          // JSON-RPC error code, so callers can tell a definitive server answer
          // (e.g. -32601 method-not-found → no team bus) from transport errors.
          err.code = data.error.code;
          rej(err);
        } else res(data.result);
      }
    };

    socket.onclose = (ev) => {
      clearTimeout(timeout);
      if (ws !== socket) {
        window.__dbg?.('ws: ignored stale socket close');
        return;
      }
      stopIdleProbe();
      const wasAuthed = authed;
      authed = false;
      ws = null;
      rejectAllPending(wasAuthed ? 'connection lost' : 'connection closed during auth');
      // Surface close-frame metadata so we can tell client-initiated from
      // server-initiated from network-killed disconnects:
      //   1000 = normal close
      //   1001 = "going away" — common when an Android webview is
      //          backgrounded; the OS suspends the WS without sending
      //          a clean close. May surface here without a code at all.
      //   1006 = abnormal closure, no close frame received (usual TCP RST
      //          on mobile networks)
      //   custom 4xxx codes = the server's `break`/`return` paths just
      //          drop the connection without a code, you'll see 1006.
      window.__dbg?.(`ws: closed code=${ev?.code ?? '?'} reason=${ev?.reason ? JSON.stringify(ev.reason) : '""'} clean=${!!ev?.wasClean} wasAuthed=${wasAuthed} idleSinceMs=${lastInboundAt ? Date.now() - lastInboundAt : 'n/a'}`);
      if (wasAuthed) notifyDisconnect('socket closed');
      else reject(new Error('connection closed during auth'));
    };

    socket.onerror = () => {
      clearTimeout(timeout);
      window.__dbg?.('ws: error');
      if (!authed) reject(new Error('connection failed'));
    };
  });
}

export function disconnect() {
  const socket = ws;
  ws = null;
  recoveryEnabled = false;
  disconnectNotified = false;
  stopIdleProbe();
  rejectAllPending('disconnected');
  if (!socket) return;
  socket.onclose = null;
  socket.onerror = null;
  socket.onmessage = null;
  socket.onopen = null;
  try { socket.close(); } catch {}
}

export function isConnected() {
  return ws?.readyState === WebSocket.OPEN;
}

export function getMachineId() {
  return ws?._getMachineId?.();
}

export function getHostname() {
  return ws?._getHostname?.();
}

let rpcTimeouts = 0;

function forceDisconnect(reason) {
  if (!ws) return;
  window.__dbg?.(`ws: forcing disconnect (${reason || 'unknown'})`);
  const socket = ws;
  ws = null;
  socket.onclose = null;
  socket.onerror = null;
  socket.onmessage = null;
  socket.onopen = null;
  try { socket.close(); } catch {}
  stopIdleProbe();
  rejectAllPending(reason || 'forced disconnect');
  notifyDisconnect(reason || 'forced disconnect');
}

function call(method, params = {}, timeoutMs = RPC_TIMEOUT_MS) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      notifyDisconnect('RPC attempted without an open socket');
      const error = new Error('not connected');
      error.code = 'DISCONNECTED';
      reject(error);
      return;
    }
    const socket = ws;
    const id = ++requestId;
    const timer = setTimeout(() => {
      pending.delete(id);
      rpcTimeouts++;
      window.__dbg?.(`ws: timeout method=${method} (${rpcTimeouts} consecutive)`);
      // Only tear down the connection when consecutive timeouts *and* there
      // is nothing else still in flight. If another RPC is still pending
      // (e.g. a big download), its single huge response frame is almost
      // certainly what's delaying our little polling RPCs — the link is
      // alive, just momentarily monopolized. We'd rather let the pollers
      // fail individually and keep the connection open. Real dead-link
      // scenarios still trip: once the long RPC also fails or completes,
      // the next idle window will re-check.
      const inboundSilenceMs = Date.now() - lastInboundAt;
      if (rpcTimeouts >= 3 && pending.size === 0 && inboundSilenceMs >= TIMEOUT_DISCONNECT_INBOUND_SILENCE_MS) {
        window.__dbg?.('ws: 3 consecutive timeouts with no pending RPC and silent inbound → forcing disconnect');
        forceDisconnect('3 consecutive RPC timeouts');
      } else if (rpcTimeouts >= 3 && inboundSilenceMs < TIMEOUT_DISCONNECT_INBOUND_SILENCE_MS) {
        // Server pushes are still arriving — the link is alive but slow
        // (or our requests are being starved by a big inbound frame).
        // Reset the counter so we re-evaluate from scratch instead of
        // tripping the breaker the moment pushes pause.
        window.__dbg?.(`ws: 3 consecutive timeouts but inbound is fresh (${inboundSilenceMs}ms ago) — staying connected`);
        rpcTimeouts = 0;
      } else if (rpcTimeouts >= 3) {
        window.__dbg?.(`ws: 3 consecutive timeouts but ${pending.size} RPC still pending — staying connected`);
      }
      reject(new Error('request timeout'));
    }, timeoutMs);
    pending.set(id, {
      resolve: (v) => { clearTimeout(timer); rpcTimeouts = 0; resolve(v); },
      reject: (e) => { clearTimeout(timer); reject(e); },
    });
    const msg = JSON.stringify({ id, method, params });
    sendOnSocket(socket, msg).catch((error) => {
      pending.delete(id);
      clearTimeout(timer);
      notifyDisconnect('RPC send lost its socket');
      reject(error);
    });
  });
}

export const listSessions = () => call('list_sessions');
export const listPanes = (session) => call('list_panes', { session });
// Single round-trip alternative for callers (Sessions page) that need both
// the session list AND all their panes — saves N+1 RPCs vs listSessions
// followed by N × listPanes.
export const listSessionsWithPanes = () => call('list_sessions_with_panes');
export const capturePane = (target, lines) => call('capture_pane', { target, lines });
export const sendKeys = (target, keys, literal = true) => call('send_keys', { target, keys, literal });
export const sendCommand = (target, command) => call('send_command', { target, command });
export const newSession = (name, path, command) => call('new_session', { name, path, command });
export const killSession = (name) => call('kill_session', { name });
export const newWindow = (session) => call('new_window', { session });
export const killWindow = (target) => call('kill_window', { target });
export const paneCommand = (target) => call('pane_command', { target });
export const resizePane = (target, cols, rows) => call('resize_pane', { target, cols, rows });
export const setSocket = (socket) => call('set_socket', { socket });
export const getBookmarks = () => call('get_bookmarks');
export const saveBookmarks = (bookmarks) => call('save_bookmarks', { bookmarks });
export const getPrefs = () => call('get_prefs');
export const setPref = (key, value) => call('set_pref', { key, value });

// File system
export const fsCwd = (session) => call('fs_cwd', { session });
export const fsList = (path, show_hidden = false) => call('fs_list', { path, show_hidden });
export const fsStat = (path) => call('fs_stat', { path });
export const fsRead = (path) => call('fs_read', { path });
export const fsWrite = (path, content) => call('fs_write', { path, content });
export const fsMkdir = (path) => call('fs_mkdir', { path });
export const fsDelete = (path) => call('fs_delete', { path });
export const fsRename = (from, to) => call('fs_rename', { from, to });
// Large transfers have a long explicit timeout — they're allowed to sit in
// flight longer than the default RPC timeout. Liveness detection during the
// transfer is handled at the WS protocol layer (server PING / browser PONG),
// so even a 50 MB frame in the air won't make us give up on the socket.
export const fsDownload = (path) => call('fs_download', { path }, 60000);
export const fsDownloadUrl = (path) => call('fs_download_url', { path });
export function fsDownloadHttp(path) {
  // Both ws:// and wss:// use the streaming HTTP /dl endpoint — the server
  // peeks the first bytes of every accepted (plain or TLS) connection and
  // branches HTTP vs WS. Streaming avoids the 50 MB cap on fs_download and
  // the base64 overhead. wsUrl maps cleanly: ws://host → http://host,
  // wss://host → https://host.
  return fsDownloadUrl(path).then(({ url, name }) => {
    const base = wsUrl.replace(/^ws/, 'http').replace(/\/$/, '');
    return { url: base + url, name };
  });
}
export const fsUpload = (path, data) => call('fs_upload', { path, data }, 60000);
export const fsConvert = (path, format = 'html') => call('fs_convert', { path, format });
export const gitCmd = (subcmd, args = [], cwd) => call('git', { subcmd, args, cwd });

// Team multi-agent bus (Team tab). Only available when the server has the
// in-process bus wired (desktop); on a server without it these reject with a
// method-not-found error, which the Team tab uses to hide itself.
// All chat ops are scoped to a team `room`. team_status / team_teams are
// team-agnostic (they list teams); the rest take the active room.
export const teamStatus = () => call('team_status');
export const teamTeams = () => call('team_teams');
export const teamHistory = (room, limit = 100) => call('team_history', { room, limit });
export const teamRoster = (room) => call('team_roster', { room });
export const teamEmployees = (room) => call('team_employees', { room });
export const teamPost = (room, body, requires_reply) => call('team_post', { room, body, requires_reply });
// Operator actions: spin up a team for a workspace (room = its slug) from a
// named roster template, or close one.
export const teamStartTeam = (workspace, template) => call('team_start_team', { workspace, template });
export const teamCloseTeam = (room) => call('team_close_team', { room });
// Roster templates (named agent rosters; edited in the Templates settings panel).
export const teamTemplates = () => call('team_templates');
export const teamTemplateSave = (name, def) => call('team_template_save', { name, def });
export const teamTemplateDelete = (name) => call('team_template_delete', { name });
// Global system prompt prepended to every agent's brief (team_status returns it).
export const teamSystemPromptSave = (text) => call('team_system_prompt_save', { text });

// Subscription refcount per target. The server keeps ONE subscription entry
// per target, so two split cells on the same window must NOT let the first
// cell's unmount send `unsubscribe` and cut the survivor's feed. We send the
// wire subscribe only on the 0→1 transition and unsubscribe only on 1→0.
// (The set of who-wants-it is the count; resubscribe-on-reconnect re-sends
// for every still-positive target.)
const subRefcount = new Map(); // target -> count

function sendSubscribe(target) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const msg = JSON.stringify({ method: 'subscribe', params: { target } });
  const socket = ws;
  sendOnSocket(socket, msg).catch(() => {});
}
function sendUnsubscribe(target) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const msg = JSON.stringify({ method: 'unsubscribe', params: { target } });
  const socket = ws;
  sendOnSocket(socket, msg).catch(() => {});
}

export function subscribe(target) {
  const n = (subRefcount.get(target) || 0) + 1;
  subRefcount.set(target, n);
  if (n === 1) sendSubscribe(target); // first subscriber → tell the server
}

export function unsubscribe(target) {
  const n = (subRefcount.get(target) || 0) - 1;
  if (n <= 0) {
    subRefcount.delete(target);
    sendUnsubscribe(target); // last subscriber left → tell the server
  } else {
    subRefcount.set(target, n);
  }
}

// Re-send subscribe for every target with a live refcount. Used after a
// reconnect, where the server forgot all subscriptions. Does NOT change
// refcounts (the cells are still mounted; only the wire state was lost).
export function resubscribeActive() {
  for (const target of subRefcount.keys()) sendSubscribe(target);
}

// --- Address optimization ---

const PROBE_TIMEOUT_MS = 3000;

// Classify address: 0=LAN, 1=Tailscale, 2=Internet
export function classifyAddress(url) {
  try {
    const host = new URL(url).hostname;
    if (/^(192\.168\.|10\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host)) return 0;
    if (/^100\./.test(host)) return 1;
    return 2;
  } catch { return 2; }
}

export const ADDRESS_LABELS = ['LAN', 'Tailscale', 'WAN'];

// ─── Probe failure memory ────────────────────────────────────────────────
// The browser can't read its own subnet (no reliable "am I on this LAN?"
// signal in a WebView), so we approximate it from history: an address that
// just failed a probe will keep failing until the device changes networks.
// Remember failures and skip those addresses for a cooldown window; clear
// the memory the moment the platform reports a network change (wifi join,
// cellular handoff) — that's exactly when a dead LAN address may have come
// alive.
const PROBE_FAIL_COOLDOWN_MS = 2 * 60 * 1000;
const probeFailedAt = new Map(); // url -> timestamp of last failed probe

function clearProbeMemory() {
  probeFailedAt.clear();
}

// True if the address has no fresh probe/connect failure on record.
// Used by the reconnect round-robin to skip addresses that just proved
// unreachable (e.g. LAN IPs while the phone is on cellular).
export function isAddressViable(url) {
  const failedAt = probeFailedAt.get(url);
  return !failedAt || Date.now() - failedAt > PROBE_FAIL_COOLDOWN_MS;
}

// Record a reachability failure observed outside probeAddress (e.g. a real
// connect() attempt that timed out or failed before auth).
export function noteAddressUnreachable(url) {
  if (url) probeFailedAt.set(url, Date.now());
}
window.addEventListener('online', clearProbeMemory);
// Network type / subnet change (wifi↔cellular, AP switch) on supporting platforms.
navigator.connection?.addEventListener?.('change', clearProbeMemory);

// Lightweight probe: WebSocket handshake only, no auth
function probeAddress(url) {
  return new Promise(resolve => {
    try {
      const probe = new WebSocket(url);
      const timer = setTimeout(() => { try { probe.close(); } catch {} resolve(false); }, PROBE_TIMEOUT_MS);
      probe.onopen = () => { clearTimeout(timer); try { probe.close(); } catch {} resolve(true); };
      probe.onerror = () => { clearTimeout(timer); resolve(false); };
    } catch { resolve(false); }
  }).then(ok => {
    if (ok) probeFailedAt.delete(url);
    else probeFailedAt.set(url, Date.now());
    return ok;
  });
}

// Probe addresses in parallel, return best reachable one (LAN > Tailscale > Internet).
// Addresses with a fresh probe failure are skipped — they cannot have come
// back without a network change, and that clears the memory. If every
// candidate is in cooldown (e.g. total outage just now), probe them all
// anyway rather than returning nothing.
export async function findBestAddress(addresses) {
  if (!addresses || addresses.length <= 1) return addresses?.[0] || null;
  const sorted = [...addresses].sort((a, b) => classifyAddress(a) - classifyAddress(b));
  const now = Date.now();
  let candidates = sorted.filter(url => {
    const failedAt = probeFailedAt.get(url);
    return !failedAt || now - failedAt > PROBE_FAIL_COOLDOWN_MS;
  });
  if (candidates.length === 0) candidates = sorted;
  else if (candidates.length < sorted.length) {
    window.__dbg?.(`probe: skipping ${sorted.length - candidates.length} recently-failed address(es)`);
  }
  const results = await Promise.all(candidates.map(url => probeAddress(url)));
  for (let i = 0; i < candidates.length; i++) {
    if (results[i]) return candidates[i];
  }
  return null;
}
