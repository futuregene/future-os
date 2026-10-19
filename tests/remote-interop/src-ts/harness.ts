import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  void promise.catch(() => undefined);
  return { promise, resolve, reject };
}
export async function bounded<T>(operation: Promise<T>, milliseconds: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([operation, new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error("interop operation timed out")), milliseconds);
    })]);
  } finally { if (timer !== undefined) clearTimeout(timer); }
}
export interface Ready {
  type: "ready";
  brokerPid: number;
  credentials: {
    pairId: string; desktopId: string; deviceId: string; nkeySeed: string; userJwt: string;
    refreshToken: string; natsWsUrl: string; tokenUrl: string;
    secure: { privateKey: string; publicKey: string; peerPublicKey: string; secret: string };
  };
  desktopNkey: string;
}
export class Harness {
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly readiness = deferred<Ready>();
  private readonly exited = deferred<number | null>();
  private readonly pending = new Map<number, ReturnType<typeof deferred<Record<string, unknown>>>>();
  private sequence = 0;
  private brokerPid: number | undefined;
  private constructor(label?: string) {
    const executable = process.env.REMOTE_INTEROP_HOST ?? resolve("../../target/debug", `remote-interop-host${process.platform === "win32" ? ".exe" : ""}`);
    this.child = spawn(executable, label === undefined ? [] : [label], { stdio: ["pipe", "pipe", "pipe"] });
    // Stdout contains ephemeral credentials and is private control IPC. Never
    // forward it to CI output or attach it as a test snapshot.
    createInterface({ input: this.child.stdout }).on("line", line => {
      let value: Record<string, unknown>;
      try {
        value = JSON.parse(line) as Record<string, unknown>;
        if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("invalid control object");
      }
      catch { this.readiness.reject(new Error("invalid harness control JSON")); return; }
      if (value.type === "ready") {
        if (!Number.isSafeInteger(value.brokerPid) || Number(value.brokerPid) <= 0) { this.readiness.reject(new Error("missing owned broker PID")); return; }
        this.brokerPid = Number(value.brokerPid);
        this.readiness.resolve(value as unknown as Ready);
      } else if (typeof value.id === "number") {
        this.pending.get(value.id)?.resolve(value);
      }
    });
    this.child.stderr.on("data", bytes => { process.stderr.write(bytes); });
    this.child.once("error", error => { this.readiness.reject(error); this.exited.resolve(null); });
    this.child.once("exit", code => {
      this.exited.resolve(code);
      this.readiness.reject(new Error("interop host exited before readiness"));
      for (const pending of this.pending.values()) pending.reject(new Error("interop host exited"));
    });
    this.child.stdin.on("error", () => { /* exit rejects the pending operation */ });
  }
  static async start(label?: string): Promise<{ harness: Harness; ready: Ready }> {
    const harness = new Harness(label);
    try { return { harness, ready: await bounded(harness.readiness.promise, 30_000) }; }
    catch (error) { await harness.close(); throw error; }
  }
  async command(type: "metrics" | "emit" | "shutdown", data?: string): Promise<Record<string, unknown>> {
    const id = ++this.sequence;
    const pending = deferred<Record<string, unknown>>();
    this.pending.set(id, pending);
    this.child.stdin.write(`${JSON.stringify({ id, type, data })}\n`);
    try { return await bounded(pending.promise, 10_000); }
    finally { this.pending.delete(id); }
  }
  async close(): Promise<void> {
    try {
      if (this.child.exitCode === null && this.child.signalCode === null && this.child.pid !== undefined) {
        try { await this.command("shutdown"); } catch { /* EOF still requests graceful shutdown */ }
        this.child.stdin.end();
        const code = await bounded(this.exited.promise, 6_000);
        if (code !== 0) throw new Error("interop host shutdown failed");
        this.brokerPid = undefined;
      }
    } finally {
      // These PIDs come only from our own child/control channel. A last-resort
      // kill must include the broker rather than orphaning a test process.
      if (this.child.exitCode === null) this.child.kill();
      if (this.brokerPid !== undefined) {
        try { process.kill(this.brokerPid); } catch { /* already reaped by Broker::drop */ }
      }
    }
  }
}
export async function until(check: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 5_000;
  while (!(await check())) {
    if (Date.now() >= deadline) throw new Error("interop observable state timed out");
    await delay(10);
  }
}
