// @vitest-environment jsdom
import type { AgentMessage } from "@future-os/thread-projection";
import { act } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Fetching the targets a transcript is missing.
 *
 * The host's history page is byte-budgeted and strips a shell call's arguments,
 * so those rows would say "Ran a command" and nothing more. Fetched once per
 * call, and never per render: a transcript re-renders on every streaming push.
 */
const request = vi.fn();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => request(...args),
}));

const { useRemoteActivityTargets } = await import("./remoteActivityTargets");
const { forgetRemoteToolTargets } = await import("./remoteToolTarget");

beforeEach(() => {
  request.mockReset();
  // Fetched targets are cached per (machine, session, run, call) for the life of
  // the window, so one test's answer would otherwise be the next one's — and the
  // fetch it asserts would never happen.
  for (const session of ["sess_1", "sess_2"])
    forgetRemoteToolTargets("desktop_a", session);
});

function message(id: string, toolCallId: string): AgentMessage {
  return {
    authorKey: "author.researchCopilot",
    content: "",
    createdAt: "2026-01-01T00:00:00.000Z",
    id: `m_${id}`,
    role: "assistant",
    segments: [{
      id: `seg_${id}`,
      item: { id: toolCallId, kind: "shell", runId: "r1", status: "completed", toolCallId },
      kind: "activity",
    }],
    status: "complete",
  } as unknown as AgentMessage;
}

it("fetches a missing target once and reports it", async () => {
  request.mockResolvedValue({ arguments: { command: "Rscript x.R" }, name: "shell" });
  const hook = renderHook(() => useRemoteActivityTargets("desktop_a", "sess_1", [message("a", "c1")]));
  await flushAsync();

  expect(request).toHaveBeenCalledTimes(1);
  expect(hook.current.get("c1")).toBe("Rscript x.R");
  hook.unmount();
});

/** A re-render with the same transcript does not ask again. */
it("does not re-ask on every render", async () => {
  request.mockResolvedValue({ arguments: { command: "ls" }, name: "shell" });
  const hook = renderHook(() => useRemoteActivityTargets("desktop_a", "sess_1", [message("a", "c1")]));
  await flushAsync();
  const calls = request.mock.calls.length;

  await act(async () => hook.rerender());
  await act(async () => hook.rerender());
  expect(request.mock.calls.length).toBe(calls);
  hook.unmount();
});

/** A host that reports nothing is not asked again either. */
it("remembers a call the host could not describe", async () => {
  request.mockResolvedValue({});
  const hook = renderHook(() => useRemoteActivityTargets("desktop_a", "sess_1", [message("a", "c1")]));
  await flushAsync();
  expect(request).toHaveBeenCalledTimes(1);

  await act(async () => hook.rerender());
  expect(request).toHaveBeenCalledTimes(1);
  expect(hook.current.get("c1")).toBeUndefined();
  hook.unmount();
});

/** Nothing to fetch means nothing asked. */
it("asks nothing when the transcript is complete", async () => {
  const complete = message("a", "c1");
  complete.segments = [{
    id: "seg",
    item: { id: "c1", kind: "shell", runId: "r1", status: "completed", target: "ls -la", toolCallId: "c1" },
    kind: "activity",
  }];
  const hook = renderHook(() => useRemoteActivityTargets("desktop_a", "sess_1", [complete]));
  await flushAsync();
  expect(request).not.toHaveBeenCalled();
  hook.unmount();
});

/** Without a conversation there is nothing to address. */
it("asks nothing without a conversation", async () => {
  const hook = renderHook(() => useRemoteActivityTargets(null, null, [message("a", "c1")]));
  await flushAsync();
  expect(request).not.toHaveBeenCalled();
  hook.unmount();
});

/** Switching conversations drops both what was found and what was asked for. */
it("forgets everything when the conversation changes", async () => {
  request.mockResolvedValue({ arguments: { command: "ls" }, name: "shell" });
  let session = "sess_1";
  const hook = renderHook(() => useRemoteActivityTargets("desktop_a", session, [message("a", "c1")]));
  await flushAsync();
  expect(hook.current.get("c1")).toBe("ls");

  request.mockResolvedValue({ arguments: { command: "pwd" }, name: "shell" });
  session = "sess_2";
  await act(async () => hook.rerender());
  await flushAsync();
  expect(request).toHaveBeenCalledTimes(2);
  expect(hook.current.get("c1")).toBe("pwd");
  hook.unmount();
});

/** A slow answer for a conversation the reader has left is dropped. */
it("drops an answer that arrives after the conversation changed", async () => {
  // One resolver per request, so the *first* conversation's answer can be
  // delivered while the second one is still in flight.
  const resolvers: Array<(value: unknown) => void> = [];
  request.mockImplementation(() => new Promise((done) => {
    resolvers.push(done);
  }));
  let session = "sess_1";
  const hook = renderHook(() => useRemoteActivityTargets("desktop_a", session, [message("a", "c1")]));
  await flushAsync();
  expect(resolvers).toHaveLength(1);

  session = "sess_2";
  await act(async () => hook.rerender());
  await act(async () => resolvers[0]!({ arguments: { command: "ls" }, name: "shell" }));
  await flushAsync();

  // The reply belonged to the conversation that was open when it was asked for.
  expect(hook.current.get("c1")).toBeUndefined();
  hook.unmount();
});
