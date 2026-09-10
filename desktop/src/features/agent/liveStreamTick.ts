/**
 * Coalesced, rate-limited driver for the live streaming projection.
 *
 * The Agent pushes run-runtime updates at the WebView's frame cadence (~60/s),
 * and every one of them makes the frontend re-read the run journal and re-render
 * the streaming message. That render cost is not proportional to the tokens that
 * arrived since the last push: re-projecting the reply walks every segment and
 * re-parses the growing block, and the browser then re-lays-out the message. A
 * high-throughput reasoning model streams a few hundred KB in a couple of
 * minutes, so the per-push cost grows with the accumulated reply and the chat
 * can stop responding while the Agent keeps streaming happily — the "desktop
 * froze but the Agent log is fine" report.
 *
 * Coalescing to one projection per `intervalMs` cuts the cumulative work by the
 * same factor. 100ms still reads as a continuous stream — text arrives much
 * slower than the eye resolves — while keeping the UI thread free between
 * pushes.
 *
 * Coalescing rules:
 * - At most one projection is in flight; a request during one collapses into a
 *   single trailing run (the projection always reads the journal's newest tail,
 *   so skipping intermediate pushes loses no content).
 * - Successive projections are spaced by `intervalMs`, with a leading run that
 *   fires immediately.
 * - `stop()` is final: the settle path owns the final render, so a stopped tick
 *   must never write a late intermediate projection over it.
 */

/** Minimum spacing between live-preview projections. */
export const LIVE_TICK_INTERVAL_MS = 100;

export interface LiveTick {
  /** Request a projection. Bursts collapse into one trailing run. */
  request: () => void;
  /** Stop driving projections (the settle path owns the final render). */
  stop: () => void;
}

export interface LiveTickOptions {
  /** Whether the caller still wants projections (view alive, send still current). */
  isActive: () => boolean;
  /** The projection to run. It owns its own error handling. */
  project: () => Promise<void>;
  /** Runs after each projection (e.g. to bump a generation counter). */
  afterProject?: () => void;
  /** Override the coalescing interval (tests). */
  intervalMs?: number;
  /** Clock seam (tests). */
  now?: () => number;
  /** Sleep seam (tests). */
  sleep?: (ms: number) => Promise<void>;
}

const sleepTimer = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

export function createLiveTick(options: LiveTickOptions): LiveTick {
  const intervalMs = options.intervalMs ?? LIVE_TICK_INTERVAL_MS;
  const now = options.now ?? (() => Date.now());
  const sleep = options.sleep ?? sleepTimer;

  let queued = false;
  let running = false;
  let stopped = false;
  let lastRunAt = Number.NEGATIVE_INFINITY;

  async function drain(): Promise<void> {
    if (running)
      return;
    running = true;
    try {
      for (;;) {
        if (!queued || stopped || !options.isActive())
          break;
        const waitMs = lastRunAt + intervalMs - now();
        if (waitMs > 0) {
          // Sleep out the remainder of the interval, then re-check: the burst
          // that arrived meanwhile must still collapse into ONE run.
          await sleep(waitMs);
          continue;
        }
        queued = false;
        lastRunAt = now();
        await options.project();
        options.afterProject?.();
      }
    }
    finally {
      running = false;
      // A request landing after the loop's final check but before `running`
      // cleared would otherwise be stranded until the next push.
      if (queued && !stopped && options.isActive())
        void drain();
    }
  }

  return {
    request() {
      if (stopped)
        return;
      queued = true;
      void drain();
    },
    stop() {
      stopped = true;
      queued = false;
    },
  };
}
