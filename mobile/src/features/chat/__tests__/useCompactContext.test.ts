import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { CompactionOutcome } from "../../../remote/types";
import type { CompactContextApi } from "../useCompactContext";
import { showToast } from "../utils";
import { useCompactContext } from "../useCompactContext";

jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("../utils", () => ({ showToast: jest.fn() }));

const { useRemote } = jest.requireMock("../../../remote/RemoteContext") as { useRemote: jest.Mock };
const mockedToast = showToast as jest.MockedFunction<typeof showToast>;

interface HarnessOptions {
  compactContext?: jest.Mock;
  outcome?: CompactionOutcome;
  compacting?: boolean;
}

/** Mount the hook and keep the latest render's API, so the pending guard is
 *  exercised the way the composer uses it (through a fresh render). */
function mount(options: HarnessOptions = {}) {
  const compactContext = options.compactContext ?? jest.fn(async () => ({
    sessionId: "s1",
    operationId: "cmp-1",
  }));
  const awaitCompactionOutcome = jest.fn(
    async (_sessionId: string, _operationId: string) => options.outcome ?? { status: "committed" as const },
  );
  useRemote.mockReturnValue({
    compactContext,
    awaitCompactionOutcome,
    compacting: options.compacting ?? false,
  });
  let api: CompactContextApi | null = null;
  // The composer's translator: asserting on it proves the failure reason is
  // interpolated into the message the user reads, not just dropped.
  const t = jest.fn((key: string) => key);
  let renderer!: ReactTestRenderer;
  function Harness() {
    api = useCompactContext(useRemote(), t as unknown as Parameters<typeof useCompactContext>[1]);
    return null;
  }
  act(() => { renderer = create(createElement(Harness)); });
  return {
    get api(): CompactContextApi { return api!; },
    renderer,
    t,
    compactContext,
    awaitCompactionOutcome,
  };
}

describe("useCompactContext", () => {
  beforeEach(() => {
    mockedToast.mockClear();
    useRemote.mockReset();
  });

  it("stays silent when the checkpoint commits — the divider already reported it", async () => {
    const h = mount({ outcome: { status: "committed" } });
    await act(async () => { await h.api.compact(); });
    expect(h.compactContext).toHaveBeenCalledTimes(1);
    expect(h.awaitCompactionOutcome).toHaveBeenCalledWith("s1", "cmp-1");
    expect(mockedToast).not.toHaveBeenCalled();
    act(() => h.renderer.unmount());
  });

  it("explains a compaction that produced no new content", async () => {
    const fresh = mount({ outcome: { status: "unchanged", alreadyCompacted: false, reused: false } });
    await act(async () => { await fresh.api.compact(); });
    expect(mockedToast).toHaveBeenLastCalledWith("chat.compactionNotNeeded");
    act(() => fresh.renderer.unmount());
    mockedToast.mockClear();

    const reused = mount({ outcome: { status: "unchanged", alreadyCompacted: true, reused: true } });
    await act(async () => { await reused.api.compact(); });
    expect(mockedToast).toHaveBeenLastCalledWith("chat.compactionNoNewContent");
    act(() => reused.renderer.unmount());
  });

  it("surfaces the Agent's failure reason, a missing result and an unobserved end", async () => {
    const failed = mount({ outcome: { status: "failed", error: "summary failed" } });
    await act(async () => { await failed.api.compact(); });
    expect(failed.t).toHaveBeenCalledWith("chat.compactionRequestFailed", {
      message: "summary failed",
    });
    expect(mockedToast).toHaveBeenLastCalledWith("chat.compactionRequestFailed");
    act(() => failed.renderer.unmount());
    mockedToast.mockClear();

    const timedOut = mount({ outcome: { status: "timeout" } });
    await act(async () => { await timedOut.api.compact(); });
    expect(mockedToast).toHaveBeenLastCalledWith("chat.compactionWaitTimedOut");
    act(() => timedOut.renderer.unmount());
    mockedToast.mockClear();

    // The session stopped compacting but the terminal frame never arrived: the
    // transcript already shows the result, so inventing one would be a lie.
    const unobserved = mount({ outcome: { status: "unobserved" } });
    await act(async () => { await unobserved.api.compact(); });
    expect(mockedToast).not.toHaveBeenCalled();
    act(() => unobserved.renderer.unmount());
  });

  it("reports a rejected request instead of acknowledging it", async () => {
    const h = mount({
      compactContext: jest.fn(async () => {
        throw new Error("finish or stop the active run before manual compaction");
      }),
    });
    await act(async () => { await h.api.compact(); });
    expect(h.awaitCompactionOutcome).not.toHaveBeenCalled();
    expect(h.t).toHaveBeenCalledWith("chat.compactionRequestFailed", {
      message: "finish or stop the active run before manual compaction",
    });
    expect(mockedToast).toHaveBeenLastCalledWith("chat.compactionRequestFailed");
    act(() => h.renderer.unmount());
  });

  it("does not stack a second request while one is already in flight", async () => {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const compactContext = jest.fn(async () => {
      await gate;
      return { sessionId: "s1", operationId: "cmp-1" };
    });
    const h = mount({ compactContext });
    let first!: Promise<void>;
    act(() => { first = h.api.compact(); });
    await act(async () => { await h.api.compact(); });
    expect(compactContext).toHaveBeenCalledTimes(1);
    await act(async () => { release(); await first; });
    act(() => h.renderer.unmount());
  });

  it("never swallows a tap: a refused start says why", async () => {
    const h = mount({ compacting: true });
    await act(async () => { await h.api.compact(); });
    expect(h.compactContext).not.toHaveBeenCalled();
    expect(mockedToast).toHaveBeenLastCalledWith("chat.compacting");
    act(() => h.renderer.unmount());
  });

  it("retries once the authoritative state says the session is idle again", async () => {
    // A lost terminal frame must not leave the action permanently dead: the
    // gate is the session's compacting state, not this client's bookkeeping.
    const rejected = jest.fn(async () => {
      throw new Error("context compaction is already running");
    });
    const busy = mount({ compacting: true, compactContext: rejected });
    await act(async () => { await busy.api.compact(); });
    expect(rejected).not.toHaveBeenCalled();
    act(() => busy.renderer.unmount());
    mockedToast.mockClear();

    const idle = mount({ compacting: false });
    await act(async () => { await idle.api.compact(); });
    expect(idle.compactContext).toHaveBeenCalledTimes(1);
    act(() => idle.renderer.unmount());
  });
});
