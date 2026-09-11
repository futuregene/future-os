/** A candidate owns its subscriptions before it becomes the serving connection.
 * Buffer only until the readiness barrier; retiring it never touches its peer. */
export class ConnectionGeneration {
  private phase: "candidate" | "serving" | "retired" = "candidate";
  private buffered: { deliver(): void; bytes: number }[] = [];
  private bufferedBytes = 0;
  private failure: unknown;

  constructor(readonly id: number) {}

  get live(): boolean {
    return this.phase !== "retired";
  }

  deliver(deliver: () => void, bytes: number): void {
    if (this.phase === "retired") return;
    if (this.phase === "serving") {
      deliver();
      return;
    }
    if (this.buffered.length >= 4096 || this.bufferedBytes + bytes > 8 * 1024 * 1024) {
      this.fail(new Error("candidate_buffer_exhausted"));
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
    const buffered = this.buffered;
    this.buffered = [];
    this.bufferedBytes = 0;
    for (const entry of buffered) {
      if (!this.live) break;
      entry.deliver();
    }
  }

  retire(): void {
    this.phase = "retired";
    this.buffered = [];
    this.bufferedBytes = 0;
  }
}
