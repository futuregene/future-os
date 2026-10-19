/** A candidate owns its subscriptions before it becomes the serving connection.
 * Buffer only until the readiness barrier; retiring it never touches its peer. */
export class ConnectionGeneration {
  private phase: "candidate" | "serving" | "retired" = "candidate";
  private buffered: { deliver(): void; bytes: number }[] = [];
  private bufferedBytes = 0;
  private failure: unknown;
  private draining = false;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(readonly id: number) {}

  get live(): boolean {
    return this.phase !== "retired";
  }

  deliver(deliver: () => void, bytes: number): void {
    if (this.phase === "retired") return;
    if (this.phase === "serving" && !this.draining) {
      deliver();
      return;
    }
    if (this.buffered.length >= 4096 || this.bufferedBytes + bytes > 8 * 1024 * 1024) {
      const error = new Error("candidate_buffer_exhausted");
      const serving = this.phase === "serving";
      this.fail(error);
      if (serving) throw error;
      return;
    }
    this.buffered.push({ deliver, bytes });
    this.bufferedBytes += bytes;
  }

  fail(error: unknown): void {
    this.failure = error;
    this.retire();
  }

  check(): void {
    if (this.failure) throw this.failure;
    if (!this.live) throw new Error("connection_cancelled");
  }

  activate(): void {
    this.check();
    this.phase = "serving";
    this.draining = true;
    this.drain();
  }

  private drain(): void {
    this.timer = null;
    if (!this.live) return;
    let count = 0;
    let bytes = 0;
    while (count < this.buffered.length && count < 64 && bytes < 256 * 1024) {
      bytes += this.buffered[count++]!.bytes;
    }
    const batch = this.buffered.splice(0, count);
    this.bufferedBytes -= bytes;
    for (const entry of batch) {
      if (!this.live) break;
      entry.deliver();
    }
    if (!this.live) return;
    if (this.buffered.length) this.timer = setTimeout(() => this.drain(), 0);
    else this.draining = false;
  }

  retire(): void {
    this.phase = "retired";
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.draining = false;
    this.buffered = [];
    this.bufferedBytes = 0;
  }
}
