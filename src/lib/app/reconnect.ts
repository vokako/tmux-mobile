// Reconnect state machine, extracted from App.svelte.
//
// Framework-free and fully dependency-injected so the whole strategy —
// parallel probe on the first attempt, viability-filtered round-robin,
// class-scaled connect timeouts, capped backoff, give-up, watchdog — is
// unit-testable without a browser or a server.
//
// The host (App.svelte) owns the UI consequences: it mirrors state via
// onStateChange, runs post-success work in onSuccess, and decides what
// "give up" looks like (drop to settings). The machine owns every timer
// and never touches the DOM.

export interface ReconnectState {
  reconnecting: boolean;
  /** 1-indexed while attempting; 0 means idle. */
  attempt: number;
  /** Address-class label of the current try ('LAN' | 'Tailscale' | 'WAN' | ''). */
  label: string;
}

export interface ReconnectDeps {
  connect: (url: string, token: string, timeoutMs: number) => Promise<unknown>;
  findBestAddress: (addresses: string[]) => Promise<string | null>;
  isAddressViable: (url: string) => boolean;
  noteAddressUnreachable: (url: string) => void;
  classifyAddress: (url: string) => number;
  addressLabels: readonly string[];
  /** Reads tmux_address / tmux_token / tmux_machine_id / tmux_machines. */
  storage: Pick<Storage, 'getItem'>;
  onStateChange: (state: ReconnectState) => void;
  /** An attempt authenticated. The machine is already idle when this runs. */
  onSuccess: (useAddr: string, primaryAddr: string) => void;
  /** Max attempts exhausted, no stored address, or watchdog fired. */
  onGiveUp: () => void;
  maxAttempts?: number;
  watchdogMs?: number;
  /** Injectable for tests; default to the real timers. */
  setTimeoutFn?: typeof setTimeout;
  clearTimeoutFn?: typeof clearTimeout;
  debug?: (msg: string) => void;
}

const DEFAULT_MAX_ATTEMPTS = 10;
const DEFAULT_WATCHDOG_MS = 180000;

export function createReconnectMachine(deps: ReconnectDeps) {
  const {
    connect, findBestAddress, isAddressViable, noteAddressUnreachable,
    classifyAddress, addressLabels, storage, onStateChange, onSuccess, onGiveUp,
    maxAttempts = DEFAULT_MAX_ATTEMPTS,
    watchdogMs = DEFAULT_WATCHDOG_MS,
    setTimeoutFn = setTimeout,
    clearTimeoutFn = clearTimeout,
    debug = (msg) => { if (typeof window !== 'undefined') window.__dbg?.(msg); },
  } = deps;

  let reconnecting = false;
  let attempt = 0;
  let label = '';
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let watchdog: ReturnType<typeof setTimeout> | null = null;
  // Loop identity. `reconnecting` says whether SOME loop is running; `gen`
  // says WHICH. Every async continuation (probe, connect, retry timer,
  // watchdog) captures the generation it was started under and bails when it
  // is not the current one — a boolean cannot tell "cancelled" from
  // "cancelled and restarted", so a superseded chain's late timeout used to
  // continue the NEW loop's counter and mark a reachable address unreachable.
  let gen = 0;

  function emit() {
    onStateChange({ reconnecting, attempt, label });
  }

  function clearTimers() {
    if (retryTimer) clearTimeoutFn(retryTimer);
    if (watchdog) clearTimeoutFn(watchdog);
    retryTimer = null;
    watchdog = null;
    attempt = 0;
    label = '';
  }

  function armWatchdog() {
    // Hard cap: if reconnecting never finishes (stuck promise, platform
    // WebSocket hang), force-reset so the user can escape without killing
    // the app.
    if (watchdog) clearTimeoutFn(watchdog);
    const myGen = gen;
    watchdog = setTimeoutFn(() => {
      if (!reconnecting || gen !== myGen) return;
      debug('reconnect: watchdog fired — force reset');
      reconnecting = false;
      if (retryTimer) clearTimeoutFn(retryTimer);
      retryTimer = null;
      emit();
      onGiveUp();
    }, watchdogMs);
  }

  function altAddresses(): string[] {
    const mid = storage.getItem('tmux_machine_id');
    const primary = storage.getItem('tmux_address');
    if (!mid) return [];
    try {
      const map = JSON.parse(storage.getItem('tmux_machines') || '{}');
      return (map[mid] || []).filter((a: string) => a !== primary);
    } catch { return []; }
  }

  async function tryAttempt(n: number, myGen: number): Promise<void> {
    const live = () => reconnecting && gen === myGen;
    if (!live()) return;
    const primary = storage.getItem('tmux_address');
    const token = storage.getItem('tmux_token') || '';
    if (!primary) {
      reconnecting = false;
      clearTimers();
      emit();
      onGiveUp();
      return;
    }

    const allAddrs = [primary, ...altAddresses()];
    let useAddr: string;

    // First attempt with multiple candidates: parallel probe → pick first
    // reachable. Avoids burning 3s × N timeouts cycling through dead
    // addresses serially.
    if (n === 0 && allAddrs.length > 1) {
      debug(`reconnect: probing ${allAddrs.length} addresses in parallel`);
      try {
        const best = await findBestAddress(allAddrs);
        if (!live()) return; // cancelled (or restarted) mid-probe
        useAddr = best || allAddrs[0]!;
      } catch {
        useAddr = allAddrs[0]!;
      }
    } else {
      // Round-robin, but skip addresses that recently failed a probe or
      // connect (LAN/Tailscale IPs while on cellular keep failing until a
      // network change, which clears the memory in ws.ts). If everything
      // is in cooldown, fall back to plain round-robin — a total outage
      // shouldn't stop us from retrying at all.
      const viable = allAddrs.filter(isAddressViable);
      const pool = viable.length > 0 ? viable : allAddrs;
      useAddr = pool[n % pool.length]!;
    }

    debug(`reconnect: attempt ${n + 1}/${maxAttempts} → ${useAddr}`);
    attempt = n + 1;
    label = addressLabels[classifyAddress(useAddr)] || '';
    emit();

    // Per-attempt connect timeout scales with address class: LAN is fast and
    // should fail fast; WAN (public internet, slow cellular, far regions)
    // legitimately needs more time for TCP + TLS handshake.
    const cls = classifyAddress(useAddr);
    const attemptTimeout = cls === 0 ? 2000 : cls === 1 ? 3000 : 5000;

    connect(useAddr, token, attemptTimeout).then(() => {
      if (!live()) return;
      reconnecting = false;
      clearTimers();
      emit();
      debug('reconnect: success');
      onSuccess(useAddr, primary);
    }).catch((e: Error) => {
      if (!live()) return;
      debug(`reconnect: failed (${e.message})`);
      // Reachability failures (timeout / refused, NOT auth errors) feed the
      // same cooldown memory the prober uses, so the next attempts skip
      // this address instead of re-burning its timeout.
      if (/timeout|connection failed|closed during auth/i.test(e.message || '')) {
        noteAddressUnreachable(useAddr);
      }
      if (n + 1 < maxAttempts) {
        const delay = Math.min(500 * (n + 1), 3000); // tighter backoff since timeouts are short
        retryTimer = setTimeoutFn(() => { void tryAttempt(n + 1, myGen); }, delay);
      } else {
        debug('reconnect: gave up');
        reconnecting = false;
        clearTimers();
        emit();
        onGiveUp();
      }
    });
  }

  return {
    /**
     * Begin the reconnect loop. Arms the watchdog. A no-op while a loop is
     * already running: the disconnect callback, a foreground check and a
     * failed address switch can all fire during one outage, and a second
     * chain would race the first for the single socket — each superseding
     * the other's connect(), each timing out, each marking a live address
     * unreachable. To start over, cancel() first.
     */
    start(): boolean {
      if (reconnecting) {
        debug('reconnect: start ignored — a loop is already running');
        return false;
      }
      gen++;
      reconnecting = true;
      emit();
      armWatchdog();
      void tryAttempt(0, gen);
      return true;
    },
    /** User-initiated abort. Resets all internal state and timers; any
     *  continuation of the old loop that lands later is discarded. */
    cancel() {
      gen++;
      reconnecting = false;
      clearTimers();
      emit();
    },
    isActive(): boolean {
      return reconnecting;
    },
  };
}
