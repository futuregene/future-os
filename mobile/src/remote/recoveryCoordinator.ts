/** Serializes state recovery per client without coupling it to transport or React. */
export class RecoveryCoordinator<Owner extends object> {
  private readonly jobs = new WeakMap<Owner, { revision: number; pending?: Promise<void> }>();

  revision(owner: Owner): number {
    return this.jobs.get(owner)?.revision ?? 0;
  }

  settled(owner: Owner): Promise<void> {
    return this.jobs.get(owner)?.pending ?? Promise.resolve();
  }

  request(owner: Owner, current: () => boolean, recover: () => Promise<void>): Promise<void> {
    let job = this.jobs.get(owner);
    if (!job) {
      job = { revision: 0 };
      this.jobs.set(owner, job);
    }
    job.revision += 1;
    if (job.pending) return job.pending;
    const state = job;
    // Batch synchronous notifications. A reconnect during a pull requests one
    // trailing pass, since the in-flight pass may have read the previous socket.
    const pending = Promise.resolve().then(async () => {
      try {
        while (current()) {
          const revision = state.revision;
          await recover();
          if (revision === state.revision) break;
        }
      } finally {
        state.pending = undefined;
      }
    });
    state.pending = pending;
    return pending;
  }
}
