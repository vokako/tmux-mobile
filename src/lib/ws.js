// WebSocket client for tmux-mobile server

const CONNECT_TIMEOUT_MS = 5000;
// RPC timeout 6s (was 10s): most calls are lightweight (send_keys, list_panes,
// capture_pane). Long-running calls (fs_download, fs_upload) pass explicit
// timeouts. Shorter default means N consecutive timeouts hit the disconnect
// threshold faster, so the reconnect UI appears sooner on a dead link.
const RPC_TIMEOUT_MS = 6000;
// Heartbeat every 5s: cheap to run, quick to notice a dead link.
const HEARTBEAT_INTERVAL_MS = 5000;
// Skip the heartbeat ping if we've received *any* message from the server
// within this window. Data flowing in either direction is proof of liveness,
// so during a download/upload or active subscription stream we don't bother
// pinging. If the wire goes quiet for longer than this, we fall back to an
// explicit ping with a relaxed timeout.
const HEARTBEAT_QUIET_MS = 8000;
// Explicit ping response window. Longer than the usual RPC timeout because
// the response can legitimately queue behind a large in-flight transfer
// (e.g. the big base64 frame of an fs_download), especially on mobile.
const PING_TIMEOUT_MS = 20000;
const BASE64_CHUNK_SIZE = 8192;

let ws = null;
let heartbeatTimer = null;
let requestId = 0;
const pending = new Map();
// Timestamp (Date.now()) of the last incoming WS message. Any frame from the
// server — RPC response, pane_output, encrypted or plain — updates this.
// The heartbeat uses it as a "connection is breathing" signal.
let lastRxAt = 0;
let onPaneOutput = null;
let onPaneClosed = null;
let onDisconnect = null;
let sessionCipher = null; // {key, sendCounter, recvCounter}

export function setOnPaneOutput(cb) { onPaneOutput = cb; }
export function setOnPaneClosed(cb) { onPaneClosed = cb; }
export function setOnDisconnect(cb) { onDisconnect = cb; }

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

async function encryptMsg(text) {
  if (!sessionCipher) return text;
  const nonce = makeNonce(sessionCipher.sendCounter++);
  const ct = await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce }, sessionCipher.key, new TextEncoder().encode(text));
  // Chunked base64 encoding to avoid stack overflow on large messages
  const bytes = new Uint8Array(ct);
  let binary = '';
  for (let i = 0; i < bytes.length; i += BASE64_CHUNK_SIZE) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + BASE64_CHUNK_SIZE));
  }
  return btoa(binary);
}

async function decryptMsg(b64) {
  if (!sessionCipher) return b64;
  const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
  const nonce = makeNonce(sessionCipher.recvCounter++);
  const pt = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: nonce }, sessionCipher.key, bytes);
  return new TextDecoder().decode(pt);
}

function rejectAllPending(reason) {
  const err = new Error(reason || 'disconnected');
  err.code = 'DISCONNECTED';
  for (const { reject: rej } of pending.values()) rej(err);
  pending.clear();
}

export function connect(url, token) {
  // Close any existing connection before creating a new one
  if (ws) {
    try { ws.onclose = null; ws.onerror = null; ws.close(); } catch {}
    ws = null;
    rejectAllPending('superseded by new connect');
  }
  sessionCipher = null;
  clearInterval(heartbeatTimer);
  heartbeatTimer = null;
  rpcTimeouts = 0;
  lastRxAt = Date.now();
  window.__dbg?.(`ws: connecting to ${url}`);

  return new Promise((resolve, reject) => {
    try {
      ws = new WebSocket(url);
    } catch (e) {
      window.__dbg?.(`ws: connect error: ${e.message}`);
      reject(e);
      return;
    }

    const timeout = setTimeout(() => {
      window.__dbg?.('ws: connect timeout');
      ws?.close();
      reject(new Error('connection timeout'));
    }, CONNECT_TIMEOUT_MS);

    let authed = false;
    let serverNonce = null;
    let machineId = null;
    let hostname = null;

    function authSuccess() {
      clearTimeout(timeout);
      authed = true;
      window.__dbg?.(`ws: authenticated (machine=${machineId})`);
      // Liveness strategy:
      //   - Any inbound frame refreshes lastRxAt, so downloads / pane pushes
      //     / RPC responses all count as "connection is breathing".
      //   - Every HEARTBEAT_INTERVAL_MS we check: if data flowed recently,
      //     skip the tick and don't pile up another ping RPC.
      //   - Only if the line is quiet for HEARTBEAT_QUIET_MS do we actively
      //     probe with call('ping'). Its timeout is PING_TIMEOUT_MS (larger
      //     than default) because the response can legitimately queue behind
      //     a big in-flight transfer; we'd rather wait than reconnect.
      //   - One pending ping at a time (pingInFlight flag); overlapping pings
      //     would inflate the fail counter on a busy but healthy link.
      //   - 2 consecutive genuine failures → disconnect + reconnect UI.
      let pingFails = 0;
      let pingInFlight = false;
      heartbeatTimer = setInterval(() => {
        if (pingInFlight) return;
        if (Date.now() - lastRxAt < HEARTBEAT_QUIET_MS) {
          pingFails = 0;
          return;
        }
        pingInFlight = true;
        call('ping', {}, PING_TIMEOUT_MS).then(() => {
          pingFails = 0;
        }).catch(() => {
          pingFails++;
          window.__dbg?.(`heartbeat: ping failed #${pingFails}`);
          if (pingFails >= 2) {
            window.__dbg?.('heartbeat: 2 failures, forcing disconnect');
            forceDisconnect('heartbeat: 2 consecutive ping failures');
          }
        }).finally(() => { pingInFlight = false; });
      }, HEARTBEAT_INTERVAL_MS);
      resolve(machineId);
    }

    // Expose getters
    ws._getMachineId = () => machineId;
    ws._getHostname = () => hostname;

    ws.onmessage = async (event) => {
      // Any inbound frame is proof of liveness — downloads, pane pushes,
      // RPC responses all count.
      lastRxAt = Date.now();
      let data;
      try { data = JSON.parse(event.data); } catch {}

      // Step 1: Receive server_nonce
      if (!authed && !serverNonce && data?.server_nonce) {
        serverNonce = hexToBytes(data.server_nonce);
        if (crypto.subtle) {
          // Encrypted auth
          const clientNonce = crypto.getRandomValues(new Uint8Array(16));
          const proof = await computeProof(token, serverNonce, clientNonce);
          const key = await deriveKey(token, serverNonce, clientNonce);
          sessionCipher = { key, sendCounter: 0, recvCounter: 0 };
          ws.send(JSON.stringify({
            method: 'auth',
            params: { client_nonce: bytesToHex(clientNonce), proof: bytesToHex(proof) }
          }));
        } else {
          // Fallback: plain token auth (http:// context, no Web Crypto)
          ws.send(JSON.stringify({ method: 'auth', params: { token } }));
        }
        return;
      }

      // Step 2: Auth response
      if (!authed && serverNonce) {
        if (sessionCipher) {
          // Encrypted response
          try {
            const pt = await decryptMsg(event.data);
            const resp = JSON.parse(pt);
            if (resp.result?.authenticated) { machineId = resp.result.machine_id; hostname = resp.result.hostname; authSuccess(); return; }
          } catch {}
          clearTimeout(timeout); sessionCipher = null; reject(new Error('auth failed')); return;
        } else {
          // Plain response
          if (data?.result?.authenticated) { machineId = data.result.machine_id; hostname = data.result.hostname; authSuccess(); return; }
          clearTimeout(timeout); reject(new Error(data?.error?.message || 'auth failed')); return;
        }
      }

      // Post-auth: decrypt all messages
      if (authed && sessionCipher) {
        let pt;
        try { pt = await decryptMsg(event.data); } catch { return; }
        try { data = JSON.parse(pt); } catch { return; }
      }

      if (!data) return;

      if (data.method === 'pane_output') {
        onPaneOutput?.(data.params?.target, data.params?.content, data.params?.cursor);
        return;
      }

      if (data.method === 'pane_closed') {
        onPaneClosed?.(data.params?.target);
        return;
      }

      if (data.id != null && pending.has(data.id)) {
        const { resolve: res, reject: rej } = pending.get(data.id);
        pending.delete(data.id);
        if (data.error) rej(new Error(data.error.message));
        else res(data.result);
      }
    };

    ws.onclose = () => {
      clearTimeout(timeout);
      clearInterval(heartbeatTimer);
      heartbeatTimer = null;
      const wasAuthed = authed;
      authed = false;
      ws = null;
      sessionCipher = null;
      rejectAllPending(wasAuthed ? 'connection lost' : 'connection closed during auth');
      window.__dbg?.(`ws: closed (wasAuthed=${wasAuthed})`);
      if (wasAuthed) onDisconnect?.();
    };

    ws.onerror = () => {
      clearTimeout(timeout);
      window.__dbg?.('ws: error');
      if (!authed) reject(new Error('connection failed'));
    };
  });
}

export function disconnect() {
  clearInterval(heartbeatTimer);
  heartbeatTimer = null;
  ws?.close();
  ws = null;
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
  clearInterval(heartbeatTimer);
  heartbeatTimer = null;
  try { ws.onclose = null; ws.close(); } catch {}
  ws = null;
  sessionCipher = null;
  rejectAllPending(reason || 'forced disconnect');
  onDisconnect?.();
}

function call(method, params = {}, timeoutMs = RPC_TIMEOUT_MS) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error('not connected'));
      return;
    }
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
      if (rpcTimeouts >= 3 && pending.size === 0) {
        window.__dbg?.('ws: 3 consecutive timeouts with no pending RPC → forcing disconnect');
        forceDisconnect('3 consecutive RPC timeouts');
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
    encryptMsg(msg).then(out => ws?.send(out)).catch((e) => {
      pending.delete(id);
      clearTimeout(timer);
      reject(e);
    });
  });
}

export const listSessions = () => call('list_sessions');
export const listPanes = (session) => call('list_panes', { session });
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
// Large transfers have a long explicit timeout. The heartbeat stays out of
// their way automatically because any incoming frame (including the transfer
// response itself) refreshes `lastRxAt`, and the heartbeat skips ticks while
// the connection is recently active.
export const fsDownload = (path) => call('fs_download', { path }, 60000);
export const fsUpload = (path, data) => call('fs_upload', { path, data }, 60000);
export const fsConvert = (path, format = 'html') => call('fs_convert', { path, format });
export const gitCmd = (subcmd, args = [], cwd) => call('git', { subcmd, args, cwd });

export function subscribe(target) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const msg = JSON.stringify({ method: 'subscribe', params: { target } });
  encryptMsg(msg).then(out => ws?.send(out)).catch(() => {});
}

export function unsubscribe(target) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  const msg = JSON.stringify({ method: 'unsubscribe', params: { target } });
  encryptMsg(msg).then(out => ws?.send(out)).catch(() => {});
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

// Lightweight probe: WebSocket handshake only, no auth
function probeAddress(url) {
  return new Promise(resolve => {
    try {
      const probe = new WebSocket(url);
      const timer = setTimeout(() => { try { probe.close(); } catch {} resolve(false); }, PROBE_TIMEOUT_MS);
      probe.onopen = () => { clearTimeout(timer); try { probe.close(); } catch {} resolve(true); };
      probe.onerror = () => { clearTimeout(timer); resolve(false); };
    } catch { resolve(false); }
  });
}

// Probe all addresses in parallel, return best reachable one (LAN > Tailscale > Internet)
export async function findBestAddress(addresses) {
  if (!addresses || addresses.length <= 1) return addresses?.[0] || null;
  const sorted = [...addresses].sort((a, b) => classifyAddress(a) - classifyAddress(b));
  const results = await Promise.all(sorted.map(url => probeAddress(url)));
  for (let i = 0; i < sorted.length; i++) {
    if (results[i]) return sorted[i];
  }
  return null;
}
