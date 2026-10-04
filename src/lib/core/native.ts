// The ONE door to the shell's Tauri commands (list_downloads, download_*).
// A lazy import keeps @tauri-apps/api out of the browser bundle's entry chunk,
// and a relative module is what the mount tier can replace.
export function invokeNative<T = unknown>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  return import('@tauri-apps/api/core').then((m) => m.invoke<T>(cmd, args));
}
