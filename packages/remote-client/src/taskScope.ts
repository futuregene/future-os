/** Waiting observes real completion. A deadline never detaches pending work. */
export interface TaskWaitResult {
  complete: boolean;
  pendingTasks: string[];
}

export class TaskScope {
  private readonly pending = new Map<Promise<unknown>, string>();
  private readonly listeners = new Set<() => void>();

  run<T>(label: string, operation: () => T | PromiseLike<T>): Promise<T> {
    let resolve!: (value: T | PromiseLike<T>) => void;
    let reject!: (error: unknown) => void;
    const pending = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
    // Register before invoking host code: a synchronous callback may stop us.
    this.pending.set(pending, label);
    const finished = () => {
      this.pending.delete(pending);
      for (const listener of [...this.listeners]) listener();
    };
    void pending.then(finished, finished);
    try { resolve(operation()); }
    catch (error) { reject(error); }
    return pending;
  }

  wait(totalBudgetMs?: number): Promise<TaskWaitResult> {
    if (totalBudgetMs !== undefined && (!Number.isFinite(totalBudgetMs) || totalBudgetMs < 0))
      return Promise.reject(new Error("invalid_shutdown_budget"));
    const result = (): TaskWaitResult => ({ complete: this.pending.size === 0, pendingTasks: [...this.pending.values()] });
    if (this.pending.size === 0 || totalBudgetMs === 0) return Promise.resolve(result());
    return new Promise(resolve => {
      let timer: ReturnType<typeof setTimeout> | undefined;
      const finish = () => {
        if (timer !== undefined) clearTimeout(timer);
        this.listeners.delete(changed);
        resolve(result());
      };
      const changed = () => { if (this.pending.size === 0) finish(); };
      this.listeners.add(changed);
      if (totalBudgetMs !== undefined) timer = setTimeout(finish, totalBudgetMs);
    });
  }
}

/** A delay has no work left after cancellation, including its timer/listener. */
export function cancellableDelay(delayMs: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(new Error("client_closed"));
  return new Promise((resolve, reject) => {
    const onAbort = () => { cleanup(); reject(new Error("client_closed")); };
    const timer = setTimeout(() => { cleanup(); resolve(); }, delayMs);
    const cleanup = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", onAbort);
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });
}
