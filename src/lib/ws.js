// WebSocket client for tmux-mobile server

let ws = null;
let requestId = 0;
const pending = new Map();
let onPaneOutput = null;
let onDisconnect = null;

export function setOnPaneOutput(cb) { onPaneOutput = cb; }
export function setOnDisconnect(cb) { onDisconnect = cb; }

export function connect(host, port, token) {
  return new Promise((resolve, reject) => {
    try {
      ws = new WebSocket(`ws://${host}:${port}`);
    } catch (e) {
      reject(e);
      return;
    }

    const timeout = setTimeout(() => {
      ws?.close();
      reject(new Error('connection timeout'));
    }, 5000);

    ws.onopen = () => {
      const msg = JSON.stringify({ method: 'auth', params: { token } });
      ws.send(msg);
    };

    let authed = false;

    ws.onmessage = (event) => {
      const data = JSON.parse(event.data);

      if (!authed) {
        if (data.result?.authenticated) {
          clearTimeout(timeout);
          authed = true;
          resolve();
        } else {
          clearTimeout(timeout);
          reject(new Error(data.error?.message || 'auth failed'));
        }
        return;
      }

      // Server push (subscribe)
      if (data.method === 'pane_output') {
        onPaneOutput?.(data.params.target, data.params.content);
        return;
      }

      // Response to a request
      if (data.id != null && pending.has(data.id)) {
        const { resolve: res, reject: rej } = pending.get(data.id);
        pending.delete(data.id);
        if (data.error) {
          rej(new Error(data.error.message));
        } else {
          res(data.result);
        }
      }
    };

    ws.onclose = () => {
      clearTimeout(timeout);
      authed = false;
      ws = null;
      onDisconnect?.();
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

function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error('not connected'));
      return;
    }
    const id = ++requestId;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
}

export const listSessions = () => call('list_sessions');
export const listPanes = (session) => call('list_panes', { session });
export const capturePane = (target, lines) => call('capture_pane', { target, lines });
export const sendKeys = (target, keys, literal = true) => call('send_keys', { target, keys, literal });
export const sendCommand = (target, command) => call('send_command', { target, command });
export const newSession = (name) => call('new_session', { name });
export const killSession = (name) => call('kill_session', { name });

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

export function subscribe(target) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  ws.send(JSON.stringify({ method: 'subscribe', params: { target } }));
}

export function unsubscribe(target) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;
  ws.send(JSON.stringify({ method: 'unsubscribe', params: { target } }));
}
