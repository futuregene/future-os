// Cold-open traffic probe: what a phone must actually download to open a
// session, measured through the production Mobile SyncEngine, read paging and
// projector against a real DesktopHost → isolated Agent → real SQLite snapshot.
//
// Two modes per sample, both from a fresh SyncEngine (a real cold open):
//   idle-open   — the session has no active run: history only.
//   active-open — the session's heaviest run is still "current", i.e. the cost
//                 of opening a session *while it streams*. Same read path the
//                 phone uses when a run is in flight; the run id is selected
//                 through SyncEngine's public reconcile API, and get_state is
//                 the real one (no forged activeRun).
//
// For every logical read we report BOTH numbers that matter:
//   wireBytes    — the bytes the socket would carry (response bodies the client
//                  had to receive, including readChunk's base64 inflation);
//   payloadBytes — the reassembled JSON the app has to hold;
//   requests     — round trips, split into logical pages vs read-chunk fetches.
//
// Keep this file dependency-free: everything imported below is production
// Mobile code from mobile/src/remote.
import { SyncEngine } from "../mobile/src/remote/syncEngine";
import { decodeRemoteJson } from "../mobile/src/remote/remoteJson";
import { fetchEventsSince } from "../mobile/src/remote/replay";
import { requestReadPage } from "../mobile/src/remote/readPages";
import { timelineFromEntries } from "../mobile/src/remote/timeline";
import type { RemoteClient } from "../mobile/src/remote/client";
import type { EntriesData, RemoteCommand, RemoteSessionState } from "../mobile/src/remote/types";

type Sample = { label: string; session: string; run: string; expected_events: number };

/** Order-independent-free canonical JSON, so a digest comparison is not
 * defeated by key ordering differences between two decoders. */
function canonical(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "null";
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, item]) => item !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return `{${entries.map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`).join(",")}}`;
}

/** FNV-1a over the canonical form. Two lanes with different offsets give a
 * 64-bit digest, enough to catch a divergent decode in a measurement. */
function digest(value: unknown): string {
  const text = canonical(value);
  let a = 0x811c9dc5;
  let b = 0x01000193;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    a = Math.imul(a ^ code, 0x01000193) >>> 0;
    b = Math.imul(b ^ code, 0x85ebca6b) >>> 0;
  }
  return `${a.toString(16).padStart(8, "0")}${b.toString(16).padStart(8, "0")}`;
}
type Mode = "idle-open" | "active-open";

interface ReadRecord {
  command: string;
  phase: string;
  requests: number;
  chunks: number;
  wireBytes: number;
  payloadBytes: number;
  backendMs: number;
  elapsedMs: number;
  decodeMs: number;
  bytesIn: number;
}

const output = document.querySelector("pre")!;
const status = document.querySelector("#status")!;
const records: unknown[] = [];

// ─── Accounting ─────────────────────────────────────────────────────────────

/** Live scope for the logical read currently in flight. Every HTTP round trip
 * the client makes while it is open belongs to that read — including the
 * `get_read_chunk` fetches `requestReadPage` issues internally. */
interface OpenRead {
  command: string;
  startedAt: number;
  requests: number;
  chunks: number;
  wireBytes: number;
  backendMs: number;
  decodeMs: number;
  bytesIn: number;
}
let phase = "state";
let openRead: OpenRead | null = null;
const trace: ReadRecord[] = [];

async function record<T>(
  command: string,
  payloadBytes: (value: T) => number,
  run: () => Promise<T>,
): Promise<T> {
  const scope: OpenRead = {
    command,
    startedAt: performance.now(),
    requests: 0,
    chunks: 0,
    wireBytes: 0,
    backendMs: 0,
    decodeMs: 0,
    bytesIn: 0,
  };
  openRead = scope;
  try {
    const value = await run();
    trace.push({ ...scope, phase, payloadBytes: payloadBytes(value), elapsedMs: performance.now() - scope.startedAt });
    return value;
  } finally {
    if (openRead === scope) openRead = null;
  }
}

/** Transport double that pages exactly like the phone (the production
 * `requestReadPage` always sends `chunkedRead`) and accounts every response
 * body it had to receive. */
function measuredClient(): RemoteClient {
  return {
    async requestRetry<T>(command: RemoteCommand) {
      const response = await fetch("/rpc", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Sync-Measurement": "1" },
        body: JSON.stringify(command),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const bytes = await response.arrayBuffer();
      if (openRead) {
        openRead.requests += 1;
        openRead.wireBytes += bytes.byteLength;
        openRead.backendMs += Number(response.headers.get("X-Backend-Ms"));
        if (command.type === "get_read_chunk") openRead.chunks += 1;
      }
      // Decode exactly the way the phone does: `decodeRemoteJson` selects
      // plain JSON or gzip by magic bytes. Using the production decoder here
      // means the measurement also exercises the gzip path and its bomb guard.
      // Timing it separately isolates the marginal decode cost (gunzip) from
      // the engine work around it.
      const decodeStarted = performance.now();
      const parsed = decodeRemoteJson<{ success: boolean; data: T; error?: string }>(
        new Uint8Array(bytes),
      );
      const decodeMs = performance.now() - decodeStarted;
      if (openRead) {
        openRead.decodeMs += decodeMs;
        openRead.bytesIn += bytes.byteLength;
      }
      if (!parsed.success) throw new Error(parsed.error ?? "RPC failed");
      return parsed;
    },
  } as unknown as RemoteClient;
}

// ─── One cold open ──────────────────────────────────────────────────────────

async function measure(sample: Sample, mode: Mode) {
  trace.length = 0;
  phase = "state";
  openRead = null;
  const client = measuredClient();
  let finish!: () => void;
  let fail!: (error: unknown) => void;
  const done = new Promise<void>((resolve, reject) => {
    finish = resolve;
    fail = reject;
  });
  let firstCommitMs: number | null = null;
  let rawEvents = 0;
  let projectedEvents = 0;
  let historyEntries = 0;
  const start = performance.now();

  const engine = new SyncEngine({
    requestGetState: async (sessionId) => {
      phase = "state";
      return record("get_state", (state: RemoteSessionState) => JSON.stringify(state).length, () =>
        client.requestRetry<RemoteSessionState>({ type: "get_state", sessionId }).then((r) => r.data),
      );
    },
    // The phone's history read: one backward page of the latest 3 exchanges.
    requestHistory: async (sessionId, isCurrent) => {
      phase = "history";
      return record(
        "get_session_entries",
        (result: { data: EntriesData }) => JSON.stringify(result.data.entries ?? []).length,
        async () => {
          const result = await requestReadPage<EntriesData>(
            client,
            { type: "get_session_entries", sessionId, before: Number.MAX_SAFE_INTEGER, limit: 3 },
            sessionId,
            isCurrent,
          );
          historyEntries = result.data.entries?.length ?? 0;
          return result;
        },
      ).then((result) => timelineFromEntries(result.data.entries ?? []));
    },
    fetchReplay: async (sessionId, runId, sinceIdx, isCurrent) => {
      phase = "replay";
      const merged = await record(
        "get_events_since",
        (result: { projection?: unknown; events?: unknown[] }) =>
          JSON.stringify(result.projection ?? result.events ?? []).length,
        () =>
          fetchEventsSince(client, sessionId, runId, sinceIdx, () =>
            isCurrent(),
          ),
      );
      rawEvents += merged.events?.length ?? 0;
      projectedEvents += merged.projection?.events?.length ?? 0;
      return { ...merged, events: merged.events ?? [] };
    },
    onSyncStatus: (_sessionId, value) => {
      if (value === "idle") finish();
    },
    onFailure: (value) => fail(value.error),
  });
  engine.subscribe(() => {
    firstCommitMs ??= performance.now() - start;
  });
  const timeout = setTimeout(() => fail(new Error("measurement exceeded 120 seconds")), 120_000);
  try {
    // Both modes are a real open: the run id is passed through the production
    // reconcile API (never by forging get_state).
    if (mode === "active-open") engine.reconcile(sample.session, "open", sample.run);
    else engine.reconcile(sample.session, "open");
    await done;
    const syncMs = performance.now() - start;
    const summarize = (name: string) => {
      const rows = trace.filter((row) => row.phase === name);
      return {
        commands: [...new Set(rows.map((row) => row.command))],
        requests: rows.reduce((sum, row) => sum + row.requests, 0),
        chunks: rows.reduce((sum, row) => sum + row.chunks, 0),
        wireBytes: rows.reduce((sum, row) => sum + row.wireBytes, 0),
        payloadBytes: rows.reduce((sum, row) => sum + row.payloadBytes, 0),
        backendMs: Math.round(rows.reduce((sum, row) => sum + row.backendMs, 0)),
        elapsedMs: Math.round(rows.reduce((sum, row) => sum + row.elapsedMs, 0)),
      };
    };
    const cursor = sample.run ? engine.cursorFor(sample.session).get(sample.run) : undefined;
    const timeline = engine.timelineFor(sample.session);
    return {
      sample: sample.label,
      mode,
      sourceEvents: sample.expected_events,
      syncMs: Math.round(syncMs),
      firstCommitMs: firstCommitMs === null ? null : Math.round(firstCommitMs),
      state: summarize("state"),
      history: summarize("history"),
      replay: summarize("replay"),
      totals: {
        requests: trace.reduce((sum, row) => sum + row.requests, 0),
        wireBytes: trace.reduce((sum, row) => sum + row.wireBytes, 0),
        payloadBytes: trace.reduce((sum, row) => sum + row.payloadBytes, 0),
      },
      historyEntries,
      rawEvents,
      projectedEvents,
      reads: trace.map((row) => ({
        ...row,
        elapsedMs: Math.round(row.elapsedMs),
        backendMs: Math.round(row.backendMs),
        decodeMs: Math.round(row.decodeMs * 100) / 100,
      })),
      highWater: cursor?.highWater ?? null,
      prefixComplete: cursor?.prefixComplete ?? null,
      timelineItems: timeline?.items.length ?? 0,
      // A content digest, not just an item count: two decode paths (plain and
      // gzip) must produce a byte-identical timeline, and only a hash proves it.
      timelineDigest: timeline ? digest(timeline.items) : null,
    };
  } finally {
    clearTimeout(timeout);
    engine.clear();
  }
}

async function main() {
  const samples: Sample[] = await (await fetch("/samples")).json();
  for (const sample of samples) {
    for (const mode of ["idle-open", "active-open"] as Mode[]) {
      status.textContent = `${sample.label} / ${mode}`;
      try {
        const result = await measure(sample, mode);
        records.push(result);
        console.info("COLD_OPEN", JSON.stringify(result));
      } catch (error) {
        records.push({ sample: sample.label, mode, error: String(error) });
      }
      output.textContent = JSON.stringify(records, null, 2);
    }
  }
  const response = await fetch("/result", {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Sync-Measurement": "1" },
    body: JSON.stringify(records, null, 2),
  });
  status.textContent = response.ok ? "COMPLETE" : "RESULT POST FAILED";
  (globalThis as { __measureDone?: boolean }).__measureDone = true;
}

void main();
