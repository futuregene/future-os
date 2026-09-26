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
const success = { content: "final", complete: true, sessionId: "session-a" };
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
async function mount(options: {
  pendingPrompt?: {
    id: string;
    content: string;
    targetThreadId: string;
  } | null;
  onPromptConsumed?: (id: string) => void;
} = {}) {
  const view = renderHook(() => useAgentThreadState({
    thread: { id: "lifecycle-thread", agentSessionId: sessionId } as StoredThread,
    loadingStore: false,
    modelId: "p/m",
    thinkingLevel: "off",
    pendingPrompt: options.pendingPrompt ?? null,
    onPromptConsumed: options.onPromptConsumed ?? noop,
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
  it("consumes a new conversation prompt at acceptance, before its reply settles", async () => {
    const onPromptConsumed = vi.fn();
    const view = await mount({
      pendingPrompt: {
        id: "first-prompt",
        content: "first question",
        targetThreadId: "lifecycle-thread",
      },
      onPromptConsumed,
    });
    expect(agent.send).toHaveBeenCalledTimes(1);
    expect(onPromptConsumed).not.toHaveBeenCalled();

    await act(async () => agent.send.mock.calls[0]![0].onAccepted());
    expect(onPromptConsumed).toHaveBeenCalledExactlyOnceWith("first-prompt");
    expect(view.current.recentRun?.status).toBe("running");

    latest = { ...running, status: "completed", endedAt: time + 1000 };
    await act(async () => {
      reply.resolve(success);
      await reply.promise;
      await Promise.resolve();
    });
    expect(onPromptConsumed).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("does not submit a staged prompt again when remounted before acceptance", async () => {
    const preparation = deferred<{ attachments: []; temporarySources: [] }>();
    agent.prepare.mockReturnValue(preparation.promise);
    const options = {
      pendingPrompt: { id: "remount-prompt", content: "question", targetThreadId: "lifecycle-thread" },
      onPromptConsumed: vi.fn(),
    };
    const first = await mount(options);
    first.unmount();
    await mount(options);
    expect(agent.prepare).toHaveBeenCalledTimes(1);
    expect(storage.createRun).not.toHaveBeenCalled();
    await act(async () => preparation.resolve({ attachments: [], temporarySources: [] }));
    expect(storage.createRun).toHaveBeenCalledTimes(1);
    expect(agent.send).toHaveBeenCalledTimes(1);
    await act(async () => agent.send.mock.calls[0]![0].onAccepted());
    expect(options.onPromptConsumed).toHaveBeenCalledExactlyOnceWith("remount-prompt");
    latest = { ...running, status: "completed" };
    await act(async () => reply.resolve(success));
  });

  it("keeps a rejected first prompt available for retry on a later visit", async () => {
    agent.prepare.mockRejectedValueOnce(new Error("unreadable attachment"));
    const options = {
      pendingPrompt: { id: "rejected-prompt", content: "question", targetThreadId: "lifecycle-thread" },
      onPromptConsumed: vi.fn(),
    };
    const first = await mount(options);
    expect(options.onPromptConsumed).not.toHaveBeenCalled();
    expect(storage.createRun).not.toHaveBeenCalled();
    first.unmount();
    await mount(options);
    expect(agent.send).toHaveBeenCalledTimes(1);
    await act(async () => agent.send.mock.calls[0]![0].onAccepted());
    expect(options.onPromptConsumed).toHaveBeenCalledExactlyOnceWith("rejected-prompt");
    latest = { ...running, status: "completed" };
    await act(async () => reply.resolve(success));
  });

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

  it("leaves the send alone when the watchdog's run read fails", async () => {
    // error-path: the watchdog driver reads the run row to decide whether a hung
    // send can be abandoned. If that read fails there is no evidence the run
    // finished, so the send must be left exactly as it was - abandoning it on a
    // failed read would release the composer while the agent is still working.
    const view = await mount();
    const first = await begin(view);
    const before = view.current.recentRun;
    storage.getRun.mockRejectedValueOnce(new Error("run table locked"));

    await act(async () => {
      window.dispatchEvent(new Event("focus"));
    });

    // The read was attempted and its failure tolerated.
    expect(storage.getRun).toHaveBeenCalledWith(running.id);
    expect(view.current.recentRun).toEqual(before);
    view.unmount();
    reply.resolve(success);
    await first.done;
  });

  it("anchors the live timer on createdAt for a legacy run with no start time", async () => {
    // boundary: `recentRun?.startedAt ?? recentRun?.createdAt ?? null`. A row written
    // by an older build can carry `createdAt` but no `startedAt`, and the live
    // elapsed timer still needs an anchor - falling through to `createdAt` keeps the
    // timer running instead of starting it from nothing. The row must be the one the
    // pipeline itself caches, so `createRun` returns it.
    const legacy = { ...running, startedAt: null, createdAt: time - 5_000 } as unknown as StoredRun;
    storage.createRun.mockImplementationOnce(async () => {
      latest = legacy;
      return legacy;
    });
    const view = await mount();
    const first = await begin(view);

    // The run is active (not settled), so the anchor expression is evaluated with
    // its `createdAt` operand as the only timestamp available.
    expect(view.current.recentRun?.id).toBe(running.id);
    expect(view.current.recentRun?.startedAt).toBeNull();
    expect(view.current.recentRun?.createdAt).toBe(time - 5_000);
    view.unmount();
    reply.resolve(success);
    await first.done;
  });

  it("yields no anchor for a run with neither a start nor a creation time", async () => {
    // boundary: the third operand of the chain. A row with no usable timestamp must
    // still render (the bubble shows no elapsed figure rather than `NaN`).
    const bare = { ...running, startedAt: null, createdAt: null } as unknown as StoredRun;
    storage.createRun.mockImplementationOnce(async () => {
      latest = bare;
      return bare;
    });
    const view = await mount();
    const first = await begin(view);

    expect(view.current.recentRun?.id).toBe(running.id);
    expect(view.current.recentRun?.startedAt).toBeNull();
    expect(view.current.recentRun?.createdAt).toBeNull();
    view.unmount();
    reply.resolve(success);
    await first.done;
  });

  it("does not reconcile a hung send while the tab is hidden", async () => {
    // concurrency: `visibilitychange` fires both when a tab returns AND when it goes
    // away, and the guard is what tells them apart. Reconciling while hidden would
    // abandon a send the user cannot see: the composer would silently release the
    // lock and the run would look finished before the user came back.
    const view = await mount();
    const first = await begin(view);
    latest = { ...running, status: "completed", endedAt: time + 1000 } as StoredRun;
    storage.getRun.mockClear();

    await act(async () => {
      Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
      document.dispatchEvent(new Event("visibilitychange"));
    });

    // The tab is hidden, so the watchdog pass must not have read the run at all.
    expect(storage.getRun).not.toHaveBeenCalled();

    // Returning to the tab is the same event with the other value, and now the
    // terminal row is honoured.
    await act(async () => {
      Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(storage.getRun).toHaveBeenCalledWith(running.id);
    expect(view.current.recentRun?.status).toBe("completed");

    view.unmount();
    reply.resolve(success);
    await first.done;
  });

  it("waits for the thread a staged prompt was composed for", async () => {
    // concurrency: a fast thread switch during the (async) message load can make
    // `thread` the newly-opened conversation while the prompt still targets the
    // one just created. Delivering here would drop the first message (and its
    // attachments) into the wrong chat, so the effect must wait for the target to
    // be active - asserting nothing was sent on this view.
    const options = {
      pendingPrompt: { id: "other-thread-prompt", content: "question", targetThreadId: "some-other-thread" },
      onPromptConsumed: vi.fn(),
    };
    const view = await mount(options);
    await act(async () => {});

    expect(agent.send).not.toHaveBeenCalled();
    expect(storage.createRun).not.toHaveBeenCalled();
    expect(options.onPromptConsumed).not.toHaveBeenCalled();
    // The prompt is not consumed, so a later visit to its own thread can still
    // deliver it: the guard waits rather than discarding.
    view.unmount();
  });
});
