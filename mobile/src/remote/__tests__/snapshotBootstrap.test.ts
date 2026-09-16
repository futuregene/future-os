import type { RemoteClient } from "../client";
import { fetchEventsSince } from "../replay";
import { SyncEngine } from "../syncEngine";
import { timelineFromEntries } from "../timeline";
import type { RemoteCommand, StreamEvent } from "../types";

const event = (type: string, idx: number, data: unknown): StreamEvent => ({ type, idx, runId: "r", data: JSON.stringify(data) });
const attachment = { name: "file.pdf", path: "file.pdf", kind: "file" as const };
const journal = [
  event("agent_start", 0, { started_at_ms: 1000 }),
  event("user_message", 1, { entry_id: "u", run_id: "r", text: "question", attachments: [attachment] }),
  event("thinking_start", 2, {}), event("thinking_delta", 3, { text: "reason" }),
  event("thinking_delta", 4, { text: " more" }), event("thinking_end", 5, {}),
  event("text_chunk", 6, { text: "prefix" }), event("text_chunk", 7, { text: " body" }),
  event("tool_start", 8, { tool_name: "shell", tool_call_id: "t", tool_args: { command: "echo ok" } }),
  event("tool_end", 9, { tool_name: "shell", tool_call_id: "t", text: "ok", exit_code: 0 }),
  event("text_chunk", 10, { text: " tail" }),
  event("agent_end", 11, { duration_ms: 100, usage: { output_tokens: 42 } }),
];
const snapshotEvents = [journal[0], journal[1], journal[2],
  event("thinking_delta", 4, { text: "reason more" }), journal[5],
  event("text_chunk", 7, { text: "prefix body" }), journal[8]];
const history = () => timelineFromEntries([
  { id: "u", runId: "r", kind: "user", role: "user", createdAtMs: 1,
    blocks: [{ kind: "text", text: "question" }], metadata: { attachments: [attachment] } },
  { id: "partial", runId: "r", kind: "assistant", role: "assistant", createdAtMs: 2,
    blocks: [{ kind: "text", text: "partial history preview" }] },
]);

beforeEach(() => jest.useFakeTimers());
afterEach(() => jest.useRealTimers());

test("snapshot plus suffix matches raw replay, without duplicate user/assistant rows or live terminal stats", async () => {
  const engines: SyncEngine[] = [];
  async function open(snapshot: boolean) {
    let engine: SyncEngine;
    const requestRetry = jest.fn(async (command: RemoteCommand) => {
      if (snapshot && command.preferSnapshot) {
        // These live suffix events overlap the following catch-up RPC. They
        // must apply exactly once after the projection is committed.
        for (const e of journal.slice(9)) engine.event("s", e);
        return { data: { runSnapshot: true, events: [], watermark: 8,
          projection: { runId: "r", cursor: 8, events: snapshotEvents } } };
      }
      return { data: { events: journal.filter(e => e.idx! > command.sinceIdx!), watermark: 11 } };
    });
    const client = { requestRetry } as unknown as RemoteClient;
    const onTiming = jest.fn();
    engine = new SyncEngine({
      onTiming,
      requestGetState: async () => ({ activeRun: { runId: "r" } }), requestHistory: async () => history(),
      fetchReplay: async (session, run, since, current) => {
        const result = await fetchEventsSince(client, session, run, since, current);
        return { ...result, events: result.events ?? [] };
      },
    });
    engines.push(engine);
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    return { engine, requestRetry, onTiming };
  }
  try {
    const raw = await open(false);
    const restored = await open(true);
    expect(restored.engine.cursorFor("s").get("r")).toEqual({ highWater: 11, prefixComplete: true });
    expect(restored.engine.timelineFor("s")?.items).toEqual(raw.engine.timelineFor("s")?.items);
    expect(restored.engine.timelineFor("s")?.items.filter(i => i.kind === "message" && i.role === "user")).toHaveLength(1);
    expect(restored.engine.streamingFor("s")).toBe(false);
    expect(restored.requestRetry).toHaveBeenCalledTimes(2);
    expect(restored.requestRetry.mock.calls[1]?.[0].sinceIdx).toBe(8);
    expect(restored.onTiming.mock.calls.at(-1)?.[0].replayPlan.mode).toBe("snapshot");
    expect(raw.onTiming.mock.calls.at(-1)?.[0].replayPlan.mode).toBe("full");
  } finally { engines.forEach(e => e.clear()); }
});

test("an error in the snapshot suffix settles the run even before agent_end arrives", async () => {
  let calls = 0;
  const client = { requestRetry: async () => (++calls === 1 ? {
    data: { runSnapshot: true, watermark: 0, events: [], projection: { runId: "r", cursor: 0, events: [journal[0]] } },
  } : { data: { events: [event("error", 1, { error: "failed" })], watermark: 1 } }) } as unknown as RemoteClient;
  const engine = new SyncEngine({
    requestGetState: async () => ({ activeRun: { runId: "r" } }), requestHistory: async () => history(),
    fetchReplay: async (session, run, since, current) => {
      const result = await fetchEventsSince(client, session, run, since, current);
      return { ...result, events: result.events ?? [] };
    },
  });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    expect(engine.cursorFor("s").get("r")?.highWater).toBe(1);
    expect(engine.streamingFor("s")).toBe(false);
  } finally { engine.clear(); }
});

test("a failed snapshot suffix does not commit its cursor or overwrite the readable preview", async () => {
  let calls = 0;
  const client = { requestRetry: async () => {
    if (++calls === 1) return { data: { runSnapshot: true, watermark: 8, events: [],
      projection: { runId: "r", cursor: 8, events: snapshotEvents } } };
    throw new Error("offline");
  } } as unknown as RemoteClient;
  const failure = jest.fn();
  const engine = new SyncEngine({
    requestGetState: async () => ({ activeRun: { runId: "r" } }), requestHistory: async () => history(),
    fetchReplay: async (session, run, since, current) => {
      const result = await fetchEventsSince(client, session, run, since, current);
      return { ...result, events: result.events ?? [] };
    }, onFailure: failure,
  });
  try {
    await engine.open("s");
    await jest.advanceTimersByTimeAsync(0);
    expect(failure).toHaveBeenCalledTimes(1);
    expect(engine.cursorFor("s").get("r")).toBeUndefined();
    expect(engine.timelineFor("s")?.items.some(i => i.kind === "message" && i.text === "partial history preview")).toBe(true);
  } finally { engine.clear(); }
});
