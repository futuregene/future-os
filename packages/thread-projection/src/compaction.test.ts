import { describe, expect, it } from "vitest";
import { compactionCheckpoints } from "./compaction";
import type { CompactionDividerCarrier } from "./compaction";

describe("compactionCheckpoints", () => {
  it("returns an empty set when nothing renders a compaction divider", () => {
    expect(compactionCheckpoints([]).size).toBe(0);
    expect(compactionCheckpoints([{}]).size).toBe(0);
    expect(compactionCheckpoints([{ segments: [] }]).size).toBe(0);
    expect(compactionCheckpoints([{ segments: [{ kind: "text" }] }]).size).toBe(0);
  });

  it("collects the checkpoint of every committed divider, ignoring unnamed ones", () => {
    const carriers: CompactionDividerCarrier[] = [
      { segments: [{ checkpointId: "cp-1", kind: "compaction" }, { kind: "text" }] },
      { segments: [{ kind: "compaction" }] },
      { segments: [{ checkpointId: "cp-2", kind: "compaction" }] },
      { segments: [{ checkpointId: "cp-1", kind: "compaction" }] },
    ];
    expect([...compactionCheckpoints(carriers)].sort()).toEqual(["cp-1", "cp-2"]);
  });
});
