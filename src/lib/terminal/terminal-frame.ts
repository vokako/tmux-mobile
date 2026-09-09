import type { Terminal } from '@xterm/xterm';

/** Erase and rebuild the authoritative snapshot inside ONE queued frame.
 * clear() emits an out-of-band scroll-to-zero event before write() can run. */
export function writeTerminalFrame(
  term: Pick<Terminal, 'write'>,
  content: string,
  onWritten: () => void,
): void {
  term.write('\x1b[?2026h\x1b[?25l\x1b[3J\x1b[H' + content + '\x1b[?25h\x1b[?2026l', onWritten);
}
