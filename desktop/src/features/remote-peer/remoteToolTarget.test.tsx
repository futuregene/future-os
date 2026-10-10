// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { fetchRemoteToolTarget, forgetRemoteToolTargets } from "./remoteToolTarget";
import { useRemoteToolTarget } from "./useRemoteToolTarget";

/**
 * What a tool call actually did.
 *
 * A lean history page strips a shell call's arguments and keeps its identity, so
 * the target has to be fetched back. What is asserted is the *parse* — which
 * field carries the path, which the command — and that a transcript's cache
 * behaves: one request per call, a failure remembered, and nothing asked of a
 * host that cannot answer.
 */

const request = vi.fn<(desktopId: string, command: Record<string, unknown>, lane: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: Parameters<typeof request>) => request(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const CALL = {
  desktopId: "desktop_a",
  runId: "run_1",
  sessionId: "sess_1",
  toolCallId: "call_1",
};

beforeEach(() => {
  request.mockReset();
  forgetRemoteToolTargets("desktop_a", "sess_1");
});

it("reads a shell command out of the arguments", async () => {
  request.mockResolvedValue({ name: "shell", arguments: { command: "ls -la /tmp" } });

  await expect(fetchRemoteToolTarget(CALL)).resolves.toBe("ls -la /tmp");
  expect(request).toHaveBeenCalledWith(
    "desktop_a",
    {
      type: "get_tool_call_args",
      sessionId: "sess_1",
      runId: "run_1",
      toolCallId: "call_1",
    },
    "sess_1",
  );
});

/**
 * A file tool's path arrives under whichever field its schema uses, and the
 * shared projection package is what decides — not a second copy here, which
 * would drift into disagreeing about which field carries it.
 *
 * The tool names are the protocol's own and are **lowercase**: `shell`, `read`,
 * `edit`, `write`. The shared parser maps anything it does not recognise to
 * `shell`, which then looks for `command` — so a name outside that set yields no
 * target for a file tool rather than a wrong one. That is the parser's
 * behaviour, asserted below rather than guessed at.
 */
it("reads a path out of whichever field the tool used", async () => {
  request.mockResolvedValue({ name: "read", arguments: { file_path: "/tmp/a.txt" } });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "c1" })).resolves.toBe("/tmp/a.txt");

  request.mockResolvedValue({ name: "edit", arguments: { path: "/tmp/b.txt" } });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "c2" })).resolves.toBe("/tmp/b.txt");

  request.mockResolvedValue({ name: "write", arguments: { filePath: "/tmp/c.txt" } });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "c3" })).resolves.toBe("/tmp/c.txt");
});

/** Arguments that arrived as a JSON string are unwrapped, as the shared parser does. */
it("unwraps arguments that arrived as a string", async () => {
  request.mockResolvedValue({ name: "shell", arguments: JSON.stringify({ command: "echo hi" }) });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "s1" })).resolves.toBe("echo hi");
});

/** A name outside the protocol's set is not guessed at: it yields nothing. */
it("has no target for a tool whose name it does not know", async () => {
  // `Todo` is not one of the four kinds, so the parser reads it as `shell` and
  // finds no `command` — no target, rather than a path it did not have.
  request.mockResolvedValue({ name: "Todo", arguments: { todos: [], file_path: "/tmp/x" } });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "unknown" })).resolves.toBeNull();
});

it("has no target for a tool that names none", async () => {
  request.mockResolvedValue({ name: "todo", arguments: { todos: [] } });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "t1" })).resolves.toBeNull();

  request.mockResolvedValue({ name: "shell" });
  await expect(fetchRemoteToolTarget({ ...CALL, toolCallId: "t2" })).resolves.toBeNull();
});

/**
 * One request per call.
 *
 * A transcript re-renders on every streaming push; a row that refetched its
 * command each time would turn reading a conversation into a request storm.
 */
it("asks once for the same call, even when two rows ask at once", async () => {
  request.mockResolvedValue({ name: "shell", arguments: { command: "once" } });

  const [first, second] = await Promise.all([
    fetchRemoteToolTarget(CALL),
    fetchRemoteToolTarget(CALL),
  ]);

  expect(first).toBe("once");
  expect(second).toBe("once");
  expect(request).toHaveBeenCalledTimes(1);

  await fetchRemoteToolTarget(CALL);
  expect(request).toHaveBeenCalledTimes(1);
});

/** Different calls are different requests: the cache is keyed by all four ids. */
it("keys the cache by the whole address", async () => {
  request.mockResolvedValue({ name: "shell", arguments: { command: "one" } });
  await fetchRemoteToolTarget(CALL);

  request.mockResolvedValue({ name: "shell", arguments: { command: "two" } });
  await fetchRemoteToolTarget({ ...CALL, toolCallId: "call_2" });
  await fetchRemoteToolTarget({ ...CALL, runId: "run_2" });
  await fetchRemoteToolTarget({ ...CALL, sessionId: "sess_2" });

  expect(request).toHaveBeenCalledTimes(4);
});

/**
 * A failed read is remembered as "no target".
 *
 * Retrying on every render would be worse than showing less: the row falls back
 * to its tool name either way, and a conversation full of failing requests is a
 * conversation the user cannot read.
 */
it("remembers a failure rather than retrying it", async () => {
  request.mockRejectedValue(new Error("peer_not_connected"));

  await expect(fetchRemoteToolTarget(CALL)).resolves.toBeNull();
  await expect(fetchRemoteToolTarget(CALL)).resolves.toBeNull();
  expect(request).toHaveBeenCalledTimes(1);
});

// ── the hook ────────────────────────────────────────────────────────────────

async function mountHook(props: Partial<Parameters<typeof useRemoteToolTarget>[0]> = {}) {
  let input = {
    desktopId: "desktop_a",
    enabled: true,
    runId: "run_1",
    sessionId: "sess_1",
    toolCallId: "call_1",
    ...props,
  };
  let saw: string | null = null;
  function Probe() {
    saw = useRemoteToolTarget(input);
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const settle = async () => {
    await act(async () => {
      for (let i = 0; i < 6; i += 1) await Promise.resolve();
    });
  };
  await act(async () => root.render(<Probe />));
  await settle();
  return {
    get current() {
      return saw;
    },
    /** Re-render with different props, as a recycled row would. */
    rerender: async (next: Partial<Parameters<typeof useRemoteToolTarget>[0]>) => {
      input = { ...input, ...next };
      await act(async () => root.render(<Probe />));
      await settle();
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("resolves the target for a renderable tool row", async () => {
  request.mockResolvedValue({ name: "shell", arguments: { command: "pwd" } });
  const hook = await mountHook();
  expect(hook.current).toBe("pwd");
  await hook.unmount();
});

/**
 * A row with no call id or no run asks nothing.
 *
 * The host cannot answer either, and a transcript full of requests it must
 * refuse is worse than one that shows less.
 */
it("asks nothing when the row cannot be addressed", async () => {
  for (const missing of [{ toolCallId: "" }, { runId: "" }, { enabled: false }]) {
    const hook = await mountHook(missing);
    expect(hook.current).toBeNull();
    await hook.unmount();
  }
  expect(request).not.toHaveBeenCalled();
});

it("asks nothing without a host or a session", async () => {
  const noHost = await mountHook({ desktopId: "" });
  expect(noHost.current).toBeNull();
  await noHost.unmount();

  const noSession = await mountHook({ sessionId: "" });
  expect(noSession.current).toBeNull();
  await noSession.unmount();
  expect(request).not.toHaveBeenCalled();
});

/**
 * A row whose call id changes must not take the previous call's answer.
 *
 * Rows are recycled: the same component instance can be handed a different
 * tool call while the old read is still in flight. Without the cancellation the
 * late answer lands on the new row, which then shows another call's command —
 * a wrong answer, which is worse than a missing one.
 */
it("does not let a previous call's late answer land on the new one", async () => {
  const pending = new Map<string, (value: unknown) => void>();
  request.mockImplementation((_desktop, command) => new Promise((resolve) => {
    pending.set(String((command as { toolCallId: string }).toolCallId), resolve);
  }));

  const hook = await mountHook();
  expect(hook.current).toBeNull();

  // The row is handed another call while the first read is still out.
  await hook.rerender({ toolCallId: "call_2" });

  await act(async () => {
    pending.get("call_2")?.({ name: "shell", arguments: { command: "the new call" } });
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  expect(hook.current).toBe("the new call");

  // The first call's answer arrives after the row moved on.
  await act(async () => {
    pending.get("call_1")?.({ name: "shell", arguments: { command: "the old call" } });
  });
  await act(async () => {
    for (let i = 0; i < 4; i += 1) await Promise.resolve();
  });
  expect(hook.current).toBe("the new call");

  await hook.unmount();
});
