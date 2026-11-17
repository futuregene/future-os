import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { forgetRemoteLaneWarnings, warnRemoteLane } from "./remoteLaneWarnings";

/**
 * What the lane says about a run it stopped drawing.
 *
 * These lines are the only record of *why* a remote reply stopped growing, so
 * what matters is that each reason is said once, names the conversation it is
 * about, and carries the error rather than swallowing it.
 */

let warn: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  warn = vi.spyOn(console, "warn").mockImplementation(() => {});
  // Every conversation these tests warn about, because the memory is per
  // conversation and a test that inherited one would be asserting the previous
  // test's deduplication.
  for (const [desktop, session] of [
    ["desktop_a", "sess_1"],
    ["desktop_a", "sess_2"],
    ["desktop_b", "sess_1"],
    ["desktop_b", "sess_2"],
  ] as const) {
    forgetRemoteLaneWarnings(desktop, session);
  }
});

afterEach(() => {
  warn.mockRestore();
});

function lines(): string[] {
  return warn.mock.calls.map((call: unknown[]) => String(call[0]));
}

describe("warnRemoteLane", () => {
  it("names the conversation, the run and the reason", () => {
    warnRemoteLane("desktop_a", "sess_1", "the run's log could not be read", { runId: "run_7" });

    expect(lines()).toEqual([
      "[remote] the run's log could not be read for run run_7 — sess_1@desktop_a",
    ]);
  });

  it("passes the error through so its stack survives", () => {
    const failure = new Error("content_page_changed");
    warnRemoteLane("desktop_a", "sess_1", "the run's log could not be read", { runId: "run_7", error: failure });

    expect(lines()[0]).toContain("(content_page_changed)");
    expect(warn.mock.calls[0]![1]).toBe(failure);
  });

  /** A lane-wide event has no run, and the line says so by leaving it out. */
  it("leaves the run out of a lane-wide warning", () => {
    warnRemoteLane("desktop_a", "sess_1", "the event lane was reset");

    expect(lines()).toEqual(["[remote] the event lane was reset — sess_1@desktop_a"]);
  });

  /**
   * A link that reconnects a few times must not print a line per attempt: each
   * of these reasons is answered by the same fix, so the second one is noise.
   */
  it("says each thing once per run", () => {
    warnRemoteLane("desktop_a", "sess_1", "a gap in the run's feed could not be filled", { runId: "run_7" });
    warnRemoteLane("desktop_a", "sess_1", "a gap in the run's feed could not be filled", { runId: "run_7" });
    warnRemoteLane("desktop_a", "sess_1", "a gap in the run's feed could not be filled", { runId: "run_8" });

    expect(lines()).toHaveLength(2);
  });

  it("keeps one conversation's warnings out of another's", () => {
    warnRemoteLane("desktop_a", "sess_1", "the event lane was reset");
    warnRemoteLane("desktop_a", "sess_2", "the event lane was reset");
    warnRemoteLane("desktop_b", "sess_1", "the event lane was reset");

    expect(lines()).toHaveLength(3);
  });

  /** A conversation that is dropped takes its own warnings with it, and no other. */
  it("forgets one conversation's warnings when it is dropped", () => {
    warnRemoteLane("desktop_a", "sess_1", "the event lane was reset");
    warnRemoteLane("desktop_a", "sess_2", "the event lane was reset");

    forgetRemoteLaneWarnings("desktop_a", "sess_1");
    warnRemoteLane("desktop_a", "sess_1", "the event lane was reset");
    warnRemoteLane("desktop_a", "sess_2", "the event lane was reset");

    expect(lines()).toHaveLength(3);
  });
});
