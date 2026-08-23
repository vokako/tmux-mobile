export const DEV_WEB_PORT = 5173;
export const DEV_SERVER_PORT = 9899;
export const DEV_WS_PATH = '/ws';
export const DEV_DOWNLOAD_PATH = '/dl';
export const DEV_SERVER_PORT_ENV = 'TMUX_MOBILE_DEV_SERVER_PORT';

export function devServerPort(env = process.env) {
  const raw = env[DEV_SERVER_PORT_ENV] || env.PORT || String(DEV_SERVER_PORT);
  const port = Number(raw);
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`invalid internal dev server port: ${raw}`);
  }
  return port;
}

export function devServerTargets(env = process.env) {
  const port = devServerPort(env);
  return {
    ws: `ws://127.0.0.1:${port}`,
    http: `http://127.0.0.1:${port}`,
  };
}
