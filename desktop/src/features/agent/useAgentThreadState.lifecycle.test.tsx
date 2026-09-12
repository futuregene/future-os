// @vitest-environment jsdom
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { clearThreadMessageSnapshots } from "./threadMessageCache";
import { useAgentThreadState } from "./useAgentThreadState";

const storage = vi.hoisted(() => ({
  getLatestRun: vi.fn(),
  getRun: vi.fn(),
  createRun: vi.fn(),
  listRuns: vi.fn(),
  getSessionEntriesPage: vi.fn(),
  listRunEvents: vi.fn(),
  listRunEventsSince: vi.fn(),
}));
const agent = vi.hoisted(() => ({ send: vi.fn(), prepare: vi.fn() }));
vi.mock("../../integrations/storage/threadStore", async original => ({
  ...await original<typeof import("../../integrations/storage/threadStore")>(),
  ...storage,
  listRunEventsBulk: vi.fn(async () => []),
}));
vi.mock("../../integrations/agent/agentClient", () => ({ sendPromptToFutureAgent: agent.send }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("./threadAttachments", () => ({
  persistImageAttachments: agent.prepare,
  finalizeTemporaryAttachmentSources: vi.fn(async () => {}),
}));
vi.mock("./buildReferencePrompt", () => ({ buildReferenceContext: vi.fn(async () => "") }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
function noop() {}
const time = Date.parse("2026-09-12T00:00:00Z");
const running = { id: "lifecycle-run", threadId: "lifecycle-thread", status: "running", createdAt: time, updatedAt: time, startedAt: time } as StoredRun;
const success = { content: "final", complete: true, sessionId: "session-a", sessionRecreated: false };
let latest: StoredRun | null;
let sessionId: string | null;
let reply: ReturnType<typeof deferred<typeof success>>;
let hook: ReturnType<typeof renderHook<ReturnType<typeof useAgentThreadState>>> | undefined;

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  clearThreadMessageSnapshots();
  latest = null;
  sessionId = "session-a";
  reply = deferred();
  storage.getLatestRun.mockImplementation(async () => latest);
  storage.getRun.mockImplementation(async () => latest);
  storage.listRuns.mockImplementation(async () => latest ? [latest] : []);
  storage.listRunEvents.mockResolvedValue([]);
  storage.listRunEventsSince.mockResolvedValue([]);
  storage.getSessionEntriesPage.mockResolvedValue({ entries: [], nextOffset: 0, hasMore: false });
  agent.prepare.mockResolvedValue({ attachments: [], temporarySources: [] });
  storage.createRun.mockImplementation(async () => {
    latest = running;
    return running;
  });
  agent.send.mockImplementation(() => reply.promise);
});
afterEach(() => {
  hook?.unmount();
  hook = undefined;
  vi.useRealTimers();
  clearThreadMessageSnapshots();
});
async function mount() {
  const view = renderHook(() => useAgentThreadState({
    thread: { id: "lifecycle-thread", agentSessionId: sessionId } as StoredThread,
    loadingStore: false,
    modelId: "p/m",
    thinkingLevel: "off",
    pendingPrompt: null,
    onPromptConsumed: noop,
    onThreadActivity: noop,
  }));
  hook = view;
  await act(async () => {});
  return view;
}
async function begin(view: NonNullable<typeof hook>) {
  let done!: Promise<void>;
  await act(async () => {
    done = view.current.handleSend({ content: "question", attachments: [] });
  });
  return { done };
}

describe("local send lifecycle", () => {
  it("does not release a preparing send using the previous run's terminal status", async () => {
    latest = { ...running, id: "previous-run", status: "completed" };
    const view = await mount();
    const preparation = deferred<{ attachments: []; temporarySources: [] }>();
    agent.prepare.mockReturnValue(preparation.promise);
    const first = await begin(view);
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    const second = await begin(view);
    const calls = agent.prepare.mock.calls.length;
    view.unmount();
    preparation.resolve({ attachments: [], temporarySources: [] });
    reply.resolve(success);
    await Promise.all([first.done, second.done]);
    expect(calls).toBe(1);
  });

  it("allows the first-binding send to refresh its terminal run and send again", async () => {
    sessionId = null;
    const view = await mount();
    const first = await begin(view);
    sessionId = "session-a";
    await act(async () => view.rerender());
    latest = { ...running, status: "completed", endedAt: time + 1000 };
    await act(async () => {
      reply.resolve(success);
      await first.done;
    });
    expect(view.current.messages.slice(-1)[0]?.status).toBe("complete");
    expect(view.current.recentRun?.status).toBe("completed");
    reply = deferred();
    const next = await begin(view);
    expect(agent.send).toHaveBeenCalledTimes(2);
    latest = { ...running, status: "completed" };
    await act(async () => {
      reply.resolve(success);
      await next.done;
    });
  });

  it("releases the active run after cancellation outside this view's abort handler", async () => {
    const view = await mount();
    const first = await begin(view);
    latest = { ...running, status: "cancelled", endedAt: time + 1000 };
    await act(async () => {
      reply.resolve({ ...success, content: "partial" });
      await first.done;
    });
    expect(view.current.messages.slice(-1)[0]?.stopped).toBe(true);
    expect(view.current.recentRun?.status).toBe("cancelled");
  });

  it("ignores an old watchdog read after a newer send owns the view", async () => {
    const view = await mount();
    const first = await begin(view);
    const oldRead = deferred<StoredRun | null>();
    storage.getRun.mockReturnValueOnce(oldRead.promise);
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    latest = { ...running, status: "completed", endedAt: time + 1000 };
    await act(async () => {
      reply.resolve(success);
      await first.done;
    });
    reply = deferred();
    const next = await begin(view);
    await act(async () => {
      oldRead.resolve({ ...running, status: "completed" });
    });
    expect(view.current.recentRun?.status).toBe("running");
    const blocked = await begin(view);
    expect(agent.send).toHaveBeenCalledTimes(2);
    latest = { ...running, status: "completed" };
    await act(async () => {
      reply.resolve(success);
      await next.done;
      await blocked.done;
    });
  });

  it("still recovers the matching hung send immediately on focus", async () => {
    const view = await mount();
    const first = await begin(view);
    latest = { ...running, status: "completed", endedAt: time + 1000 };
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(view.current.recentRun?.status).toBe("completed");
    expect(storage.getRun).toHaveBeenCalledWith(running.id);
    view.unmount();
    reply.resolve(success);
    await first.done;
  });
});
