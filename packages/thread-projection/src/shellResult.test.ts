import { describe, expect, it } from "vitest";
import type { ShellResult, ShellStatus } from "./shellResult";
import type { SessionEntry, RunEvent } from "./events";
import { buildAssistantRunProjection } from "./liveApply";
import { entriesToMessages } from "./projection";
import { shellResult } from "./shellResult";

function facts(code: number | null, status: ShellStatus = "exited"): ShellResult {
  return {
    command: "action", cwd: "/repo", duration_ms: 1, status, exit_code: code, is_soft_fail: false,
    attempts: status === "not_started" ? [] : [{ status, exit_code: code, duration_ms: 1, output: "[exit: 0]", output_truncated: false, escalated: false }],
    approval: null, note: null, is_error: status !== "exited" || code !== 0,
  };
}
const event = (eventType: string, payload: unknown, sequence: number): RunEvent => ({
  eventType, payload: JSON.stringify(payload), sequence, id: `e${sequence}`, runId: "r", createdAt: sequence,
});
function history(result?: ShellResult): SessionEntry[] {
  const base = { createdAtMs: 1, runId: "r" };
  return [
    { ...base, id: "u", kind: "user", role: "user", blocks: [{ kind: "text", text: "do it" }] },
    { ...base, id: "a", kind: "assistant", role: "assistant", blocks: [{ kind: "tool_call", name: "shell", toolCallId: "t" }] },
    { ...base, id: "out", kind: "tool", role: "tool", blocks: [{ kind: "tool_result", toolCallId: "t", isError: result?.is_error ?? false }], metadata: result ? { shell_result: result } : undefined },
  ];
}
function replayActivities(entries: SessionEntry[]) {
  const message = entriesToMessages(entries).find(message => message.role === "assistant");
  // History uses ordered segments; activityItems is the legacy/live fallback.
  return message?.segments?.flatMap(segment => segment.kind === "activity" ? [segment.item] : []) ?? [];
}

describe("single-command facts across live and history projections", () => {
  it.each([
    facts(7), facts(0), facts(null, "timed_out"), facts(null, "cancelled"),
    facts(null, "launch_failed"), facts(null, "not_started"),
  ])("retains host facts and ignores printed status", (result) => {
    const live = buildAssistantRunProjection([
      event("tool_start", { tool_id: "t", tool_name: "shell" }, 1),
      event("tool_end", { tool_id: "t", exit_code: 0, text: "[exit: 0]", shell_result: result }, 2),
    ]);
    const replayItem = replayActivities(history(result))[0];
    expect(live.activityItems[0]?.shellResult).toEqual(result);
    expect(live.activityItems[0]?.status).toBe(result.is_error ? "failed" : "completed");
    expect(replayItem?.shellResult).toEqual(result);
    expect(replayItem?.status).toBe(live.activityItems[0]?.status);
  });

  it.each([[7, 0], [0, 2]])("keeps action and verification calls independent (%i, %i)", (first, second) => {
    const live = buildAssistantRunProjection([
      event("tool_start", { tool_id: "t", tool_name: "shell" }, 1),
      event("tool_end", { tool_id: "t", shell_result: facts(first!) }, 2),
      event("tool_start", { tool_id: "t2", tool_name: "shell" }, 3),
      event("tool_end", { tool_id: "t2", shell_result: facts(second!) }, 4),
    ]);
    expect(live.activityItems).toHaveLength(2);
    expect(live.activityItems.map(item => item.shellResult?.exit_code)).toEqual([first, second]);
  });

  it("retains original and approved retry facts in live and replay", () => {
    const result = facts(0);
    result.approval = "approved";
    result.attempts = [facts(7).attempts[0]!, { ...result.attempts[0]!, escalated: true }];
    const live = buildAssistantRunProjection([event("tool_end", { tool_id: "t", tool_name: "shell", shell_result: result }, 1)]);
    const replay = replayActivities(history(result));
    expect(live.activityItems[0]?.shellResult?.attempts.map(attempt => attempt.exit_code)).toEqual([7, 0]);
    expect(replay[0]?.shellResult).toEqual(result);
    expect(live.activityItems[0]?.status).toBe("completed");
  });

  it("keeps completed command grouping compact without losing child facts in live or history", () => {
    const first = facts(0);
    const second = { ...facts(0), command: "verify" };
    const live = buildAssistantRunProjection([
      event("tool_start", { tool_id: "t", tool_name: "shell" }, 1),
      event("tool_end", { tool_id: "t", shell_result: first }, 2),
      event("tool_start", { tool_id: "t2", tool_name: "shell" }, 3),
      event("tool_end", { tool_id: "t2", shell_result: second }, 4),
    ]);
    const entries = history(first);
    entries[1]!.blocks.push({ kind: "tool_call", name: "shell", toolCallId: "t2" });
    entries.push({
      id: "out2", createdAtMs: 2, runId: "r", kind: "tool", role: "tool",
      blocks: [{ kind: "tool_result", toolCallId: "t2", isError: false }],
      metadata: { shell_result: second },
    });
    const replay = replayActivities(entries);
    for (const items of [live.activityItems, replay]) {
      expect(items).toHaveLength(1);
      expect(items?.[0]?.count).toBe(2);
      expect(items?.[0]?.children?.map(child => child.shellResult)).toEqual([first, second]);
    }
  });

  it("does not invent attempt history for old records", () => {
    const replay = replayActivities(history());
    expect(replay).toHaveLength(1);
    expect(replay[0]?.shellResult).toBeUndefined();
  });

  it("uses the host verdict for normal query returns and retains nonzero", () => {
    const result = facts(1);
    result.is_error = false;
    result.is_soft_fail = true;
    const live = buildAssistantRunProjection([event("tool_end", { tool_id: "t", tool_name: "shell", shell_result: result }, 1)]);
    expect(live.activityItems[0]?.status).toBe("completed");
    expect(live.activityItems[0]?.shellResult?.exit_code).toBe(1);
  });

  it("rejects malformed envelopes without parsing their output", () => {
    expect(shellResult({ text: "[exit: 0]" })).toBeUndefined();
    expect(shellResult({ ...facts(0), status: "success" })).toBeUndefined();
    expect(shellResult({ ...facts(0), duration_ms: -1 })).toBeUndefined();
    expect(shellResult({ ...facts(0), attempts: [facts(0).attempts[0], facts(0).attempts[0], facts(0).attempts[0]] })).toBeUndefined();
    expect(shellResult({ steps: [facts(0)], is_error: false })).toBeUndefined();
  });
});
