export const COMPLETION_FEEDBACK_MS = 1500;

export interface FeedbackValue {
  kind: 'success' | 'error' | 'progress' | 'result';
  message: string;
  detail?: string;
  progress?: number | null;
  /** The progress glyph. Default 'refresh' (spins only while indeterminate);
   * 'download' is the Files download glyph (board #307). */
  glyph?: 'download';
}

interface FeedbackClock {
  set(run: () => void, delay: number): unknown;
  clear(handle: unknown): void;
}
const clock: FeedbackClock = {
  set: (run, delay) => setTimeout(run, delay),
  clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/** Message action rows retain their own context generation; only the clock
 * is shared with ordinary notices. Transport/watchdog timers do not use this. */
export function scheduleCompletion(run: () => void, timer: FeedbackClock = clock): () => void {
  const handle = timer.set(run, COMPLETION_FEEDBACK_MS);
  return () => timer.clear(handle);
}

/** One instance belongs to one local feedback slot, never the app or an RPC. */
export function createFeedbackLifetime<T extends FeedbackValue>(
  publish: (value: T | null) => void,
  timer: FeedbackClock = clock,
) {
  let attempt = 0;
  let disposed = false;
  let displayed: T | null = null;
  let cancelExpiry = () => {};
  const current = (token: number) => !disposed && token === attempt;
  function begin() {
    if (disposed) return attempt;
    attempt++;
    cancelExpiry();
    cancelExpiry = () => {};
    displayed = null;
    publish(null);
    return attempt;
  }
  return {
    begin, current,
    update(token: number, value: T): boolean {
      if (!current(token)) return false;
      cancelExpiry();
      cancelExpiry = () => {};
      const next = { ...value };
      displayed = next;
      publish(next);
      if (next.kind === 'success') {
        cancelExpiry = scheduleCompletion(() => {
          if (!current(token) || displayed !== next) return;
          cancelExpiry = () => {};
          displayed = null;
          publish(null);
        }, timer);
      }
      return true;
    },
    clear: begin,
    dispose() {
      if (disposed) return;
      cancelExpiry();
      attempt++;
      disposed = true;
      displayed = null;
      publish(null);
    },
  };
}
