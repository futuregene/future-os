import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { CompactionOutcome } from "../../../remote/types";
import { emptyTimeline } from "../../../remote/timeline";
import { showToast } from "../utils";
import { useCompactContext, type CompactContextApi } from "../useCompactContext";

jest.mock("../utils", () => ({ showToast: jest.fn() }));
const toast = showToast as jest.MockedFunction<typeof showToast>;
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

type Remote = Parameters<typeof useCompactContext>[0];
function mount(overrides: Partial<Remote> = {}) {
  let remote = {
    selectedSessionId: "s1", compacting: false, busy: false, draft: false,
    credentials: { pairId: "pair" }, presence: { bridgeInstanceId: "bridge" },
    capabilities: new Set(["compaction_v1"]), timeline: emptyTimeline(),
    compactContext: jest.fn(async () => ({ sessionId: "s1", operationId: "cmp" })),
    awaitCompactionOutcome: jest.fn(async () => ({ status: "committed" as const })),
    ...overrides,
  } as Remote;
  const t = jest.fn((key: string) => key) as unknown as Parameters<typeof useCompactContext>[1];
  let api!: CompactContextApi;
  let tree!: ReactTestRenderer;
  function Harness() { api = useCompactContext(remote, t); return null; }
  act(() => { tree = create(createElement(Harness)); });
  return {
    get api() { return api; }, get remote() { return remote; }, t,
    update(patch: Partial<Remote>) { remote = { ...remote, ...patch }; act(() => tree.update(createElement(Harness))); },
    unmount() { act(() => tree.unmount()); },
  };
}

beforeEach(() => jest.clearAllMocks());

test("pending is immediate, survives ACK without started, and blocks same-tick double taps", async () => {
  const ack = deferred<{ sessionId: string; operationId: string }>();
  const end = deferred<CompactionOutcome>();
  const request = jest.fn(() => ack.promise);
  const wait = jest.fn(() => end.promise);
  const h = mount({ compactContext: request, awaitCompactionOutcome: wait });
  let first!: Promise<void>;
  act(() => { first = h.api.compact(); void h.api.compact(); });
  expect(h.api.pending).toBe(true);
  expect(request).toHaveBeenCalledTimes(1);
  await act(async () => ack.resolve({ sessionId: "s1", operationId: "cmp" }));
  expect(h.api.pending).toBe(true);
  expect(wait).toHaveBeenCalledWith("s1", "cmp", undefined, expect.any(AbortSignal));
  await act(async () => { await h.api.compact(); });
  expect(request).toHaveBeenCalledTimes(1);
  await act(async () => { end.resolve({ status: "committed" }); await first; });
  expect(h.api.pending).toBe(false);
  expect(toast).not.toHaveBeenCalled();
  h.unmount();
});

test.each([
  [{ status: "committed" }, null],
  [{ status: "failed", error: "summary failed" }, "chat.compactionRequestFailed"],
  [{ status: "unchanged", alreadyCompacted: false, reused: false }, "chat.compactionNotNeeded"],
  [{ status: "unchanged", alreadyCompacted: false, reused: true }, "chat.compactionNoNewContent"],
  [{ status: "timeout" }, "chat.compactionWaitTimedOut"],
  [{ status: "unobserved" }, null],
  [{ status: "cancelled" }, null],
] as [CompactionOutcome, string | null][])("settles %j and releases pending", async (outcome, label) => {
  const h = mount({ awaitCompactionOutcome: jest.fn(async () => outcome) });
  await act(async () => { await h.api.compact(); });
  expect(h.api.pending).toBe(false);
  if (label) expect(toast).toHaveBeenCalledWith(label);
  else expect(toast).not.toHaveBeenCalled();
  if (outcome.status === "failed") expect(h.t).toHaveBeenCalledWith(label, { message: "summary failed" });
  h.unmount();
});

test("rejected admission releases pending and does not wait for a fake operation", async () => {
  const h = mount({ compactContext: jest.fn(async () => { throw new Error("session busy"); }) });
  await act(async () => { await h.api.compact(); });
  expect(h.api.pending).toBe(false);
  expect(h.remote.awaitCompactionOutcome).not.toHaveBeenCalled();
  expect(h.t).toHaveBeenCalledWith("chat.compactionRequestFailed", { message: "session busy" });
  h.unmount();
});

test.each(["session", "desktop", "bridge"])("%s switch isolates the next request from a late ACK", async kind => {
  const oldAck = deferred<{ sessionId: string; operationId: string }>();
  const nextEnd = deferred<CompactionOutcome>();
  const request = jest.fn().mockImplementationOnce(() => oldAck.promise)
    .mockResolvedValue({ sessionId: kind === "session" ? "s2" : "s1", operationId: "next" });
  const wait = jest.fn((_session: string, _operation: string) => nextEnd.promise);
  const h = mount({ compactContext: request, awaitCompactionOutcome: wait });
  let first!: Promise<void>;
  act(() => { first = h.api.compact(); });
  const change = kind === "session" ? { selectedSessionId: "s2" }
    : kind === "desktop" ? { credentials: { ...h.remote.credentials!, pairId: "other" } }
      : { presence: { ...h.remote.presence!, bridgeInstanceId: "other" } };
  h.update(change);
  expect(h.api.pending).toBe(false);
  let second!: Promise<void>;
  act(() => { second = h.api.compact(); });
  await act(async () => { oldAck.resolve({ sessionId: "s1", operationId: "old" }); await first; });
  expect(h.api.pending).toBe(true);
  expect(wait).toHaveBeenCalledTimes(1);
  expect(wait.mock.calls[0]![1]).toBe("next");
  expect(toast).not.toHaveBeenCalled();
  await act(async () => { nextEnd.resolve({ status: "committed" }); await second; });
  h.unmount();
});

test("leaving after ACK cancels the waiter and suppresses late failure toasts", async () => {
  const end = deferred<CompactionOutcome>();
  const wait = jest.fn(() => end.promise);
  const h = mount({ awaitCompactionOutcome: wait });
  let done!: Promise<void>;
  await act(async () => { done = h.api.compact(); });
  const signal = (wait.mock.calls[0] as unknown as [string, string, undefined, AbortSignal])[3];
  h.unmount();
  expect(signal.aborted).toBe(true);
  await act(async () => { end.resolve({ status: "failed", error: "old" }); await done; });
  expect(toast).not.toHaveBeenCalled();
});

test.each([
  { compacting: true }, { busy: true }, { draft: true }, { selectedSessionId: "" },
  { capabilities: new Set<string>() }, { timeline: { ...emptyTimeline(), streaming: true } },
])("rejects unavailable context action %j", async overrides => {
  const h = mount(overrides);
  await act(async () => { await h.api.compact(); });
  expect(h.remote.compactContext).not.toHaveBeenCalled();
  expect(h.api.pending).toBe(false);
  h.unmount();
});
