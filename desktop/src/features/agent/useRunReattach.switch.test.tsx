// @vitest-environment jsdom
import type { MutableRefObject } from "react";
import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearThreadMessageSnapshots } from "./threadMessageCache";
import { resetRunProjection } from "./threadRunProjection";
import { useRunReattach } from "./useRunReattach";
import { useThreadMessages } from "./useThreadMessages";

const storage = vi.hoisted(() => ({
  getLatestRun: vi.fn(),
  getRun: vi.fn(),
  getSessionEntriesPage: vi.fn(),
  listRuns: vi.fn(),
  listRunEventsSince: vi.fn(),
  listRunEvents: vi.fn(),
  listRunEventsBulk: vi.fn(),
  storedTimeToIso: (value: number) => new Date(value).toISOString(),
}));
const listeners = vi.hoisted(() => new Map<string, Set<(event: unknown) => void>>());
vi.mock("../../integrations/storage/threadStore", () => storage);
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: unknown) => void) => {
    const handlers = listeners.get(name) ?? new Set();
    handlers.add(handler);
    listeners.set(name, handlers);
    return () => handlers.delete(handler);
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const runId = "switch-run-a";
const time = Date.parse("2026-09-12T00:00:00Z");
const run = { id: runId, threadId: "A", status: "running", startedAt: time, createdAt: time, updatedAt: time } as StoredRun;
let activeRun: StoredRun | null;
let events: StoredRunEvent[];
let current: ReturnType<typeof useThreadMessages>;
let sending: MutableRefObject<boolean>;
let root: ReturnType<typeof createRoot>;

function textEvent(sequence: number, text: string): StoredRunEvent {
  return { id: `e${sequence}`, runId, sequence, eventType: "text_chunk", payload: JSON.stringify({ text }), createdAt: time };
}

function Harness({ threadId }: { threadId: string }) {
  const messages = useThreadMessages({ threadId, agentSessionId: `session-${threadId}` });
  const sendingRef = useRef(false);
  const active = messages.recentRun?.status === "running" ? messages.recentRun : null;
  useRunReattach({
    threadId,
    activeRunId: active?.id ?? null,
    activeRunStartedAt: active?.startedAt ?? null,
    loadingThread: messages.loadingThread,
    sendingRef,
    setMessages: messages.setMessages,
    refreshRecentRun: messages.refreshRecentRun,
    reloadMessagesQuiet: messages.reloadMessagesQuiet,
  });
  current = messages;
  sending = sendingRef;
  return null;
}

async function switchTo(threadId: string) {
  await act(async () => root.render(<Harness key={threadId} threadId={threadId} />));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(200);
  });
}

async function startAAndSwitchAway() {
  await switchTo("A");
  act(() => {
    sending.current = true;
    activeRun = run;
    current.setRecentRun(run);
    current.setMessages(messages => [...messages, {
      id: "pending-a",
      role: "assistant",
      runId,
      status: "streaming",
      authorKey: "author.researchCopilot",
      content: "before switch",
      createdAt: new Date(time).toISOString(),
    }]);
  });
  await switchTo("B");
  expect(current.messages.every(message => message.runId !== runId)).toBe(true);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  listeners.clear();
  clearThreadMessageSnapshots();
  resetRunProjection(runId);
  activeRun = null;
  events = [textEvent(0, "before switch")];
  storage.getLatestRun.mockImplementation(async (threadId: string) => threadId === "A" ? activeRun : null);
  storage.getRun.mockImplementation(async () => activeRun);
  storage.listRuns.mockImplementation(async (threadId: string) => threadId === "A" && activeRun ? [activeRun] : []);
  storage.listRunEventsSince.mockImplementation(async (_runId: string, since: number) => events.filter(event => event.sequence > since));
  storage.listRunEvents.mockImplementation(async () => events);
  storage.listRunEventsBulk.mockResolvedValue([]);
  storage.getSessionEntriesPage.mockImplementation(async (threadId: string) => ({
    entries: [
      {
        id: `user-${threadId}`,
        kind: "user",
        role: "user",
        runId: threadId === "A" ? runId : undefined,
        run: threadId === "A" && activeRun ? { status: activeRun.status, error: activeRun.errorMessage } : undefined,
        createdAtMs: time,
        blocks: [{ kind: "text", text: `prompt ${threadId}` }],
      },
      ...(threadId === "A" && activeRun?.status === "completed"
        ? [{
            id: "canonical-answer",
            kind: "assistant",
            role: "assistant",
            runId,
            createdAtMs: time + 1000,
            blocks: [{ kind: "text", text: "completed while away" }],
          }]
        : []),
    ],
    nextOffset: 0,
    hasMore: false,
  }));
  root = createRoot(document.createElement("div"));
});

afterEach(() => {
  act(() => root.unmount());
  vi.useRealTimers();
  clearThreadMessageSnapshots();
  resetRunProjection(runId);
});

describe("switching A -> B -> A", () => {
  it("takes over the cached optimistic bubble and keeps applying live events", async () => {
    await startAAndSwitchAway();
    events.push(textEvent(1, " + while away"));
    await switchTo("A");
    expect(current.messages.filter(message => message.role === "assistant")).toMatchObject([
      { id: "pending-a", content: "before switch + while away", status: "streaming" },
    ]);
    events.push(textEvent(2, " + after return"));
    await act(async () => {
      for (const handler of listeners.get("thread-runtime-updated") ?? []) {
        handler({ payload: { updates: [{ threadId: "A", runId, revision: 1, status: "running", resetProjection: false }] } });
      }
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(current.messages.filter(message => message.role === "assistant")).toMatchObject([
      { id: "pending-a", content: "before switch + while away + after return", status: "streaming" },
    ]);
    await switchTo("B");
    expect(current.messages.every(message => message.runId !== runId)).toBe(true);
    events.push(textEvent(3, " + second return"));
    await switchTo("A");
    expect(current.messages.filter(message => message.role === "assistant")).toMatchObject([
      { id: "pending-a", content: "before switch + while away + after return + second return", status: "streaming" },
    ]);
  });

  it.each(["completed", "failed", "cancelled"] as const)("settles the cached streaming bubble when A became %s while B was open", async (status) => {
    await startAAndSwitchAway();
    activeRun = { ...run, status, endedAt: time + 1000, errorMessage: status === "failed" ? "synthetic failure" : null };
    await switchTo("A");
    expect(current.recentRun?.status).toBe(status);
    expect(current.messages.filter(message => message.role === "assistant")).toHaveLength(1);
    expect(current.messages.find(message => message.role === "assistant")?.id).toBe("pending-a");
    if (status === "completed") {
      expect(current.messages.find(message => message.role === "assistant")).toMatchObject({ content: "completed while away", status: "complete" });
    }
    expect(current.messages.some(message => message.status === "streaming")).toBe(false);
  });
});
