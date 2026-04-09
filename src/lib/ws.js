// WebSocket client for tmux-mobile server

let ws = null;
let requestId = 0;
const pending = new Map();
let onPaneOutput = null;
let onDisconnect = null;
let sessionCipher = null; // {key, sendCounter, recvCounter}

export function setOnPaneOutput(cb) { onPaneOutput = cb; }
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
  const sig = await crypto.subtle.sign('HMAC', hmacKey, serverNonce);
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
  for (let i = 0; i < bytes.length; i += 8192) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
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

function rejectAllPending() {
  const err = new Error('disconnected');
  for (const { reject: rej } of pending.values()) rej(err);
  pending.clear();
}

export function connect(url, token) {
  // Close any existing connection before creating a new one
  if (ws) {
    try { ws.onclose = null; ws.onerror = null; ws.close(); } catch {}
    ws = null;
    rejectAllPending();
  }
  sessionCipher = null;

  return new Promise((resolve, reject) => {
    try {
      ws = new WebSocket(url);
    } catch (e) {
      reject(e);
      return;
    }

    const timeout = setTimeout(() => {
      ws?.close();
      reject(new Error('connection timeout'));
    }, 5000);

    let authed = false;
    let serverNonce = null;
    let machineId = null;
    let hostname = null;

    // Expose getters
    ws._getMachineId = () => machineId;
    ws._getHostname = () => hostname;

    ws.onmessage = async (event) => {
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
            if (resp.result?.authenticated) { clearTimeout(timeout); authed = true; machineId = resp.result.machine_id; hostname = resp.result.hostname; resolve(machineId); return; }
          } catch {}
          clearTimeout(timeout); sessionCipher = null; reject(new Error('auth failed')); return;
        } else {
          // Plain response
          if (data?.result?.authenticated) { clearTimeout(timeout); authed = true; machineId = data.result.machine_id; hostname = data.result.hostname; resolve(machineId); return; }
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

      if (data.id != null && pending.has(data.id)) {
        const { resolve: res, reject: rej } = pending.get(data.id);
        pending.delete(data.id);
        if (data.error) rej(new Error(data.error.message));
        else res(data.result);
      }
    };

    ws.onclose = () => {
      clearTimeout(timeout);
      const wasAuthed = authed;
      authed = false;
      ws = null;
      sessionCipher = null;
      rejectAllPending();
      if (wasAuthed) onDisconnect?.();
    };

    ws.onerror = () => {
      clearTimeout(timeout);
      if (!authed) reject(new Error('connection failed'));
    };
  });
}

export function disconnect() {
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

function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error('not connected'));
      return;
    }
    const id = ++requestId;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error('request timeout'));
    }, 10000);
    pending.set(id, {
      resolve: (v) => { clearTimeout(timer); resolve(v); },
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
export const fsDownload = (path) => call('fs_download', { path });
export const fsUpload = (path, data) => call('fs_upload', { path, data });
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
