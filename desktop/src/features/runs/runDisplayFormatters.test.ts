import { describe, expect, it } from "vitest";
import { formatRunStatus, runTone, shortId, toolStatusLabel } from "./runDisplayFormatters";

describe("formatRunStatus", () => {
  it("labels every status", () => {
    expect(formatRunStatus("completed")).toBe("completed");
    expect(formatRunStatus("failed")).toBe("failed");
    expect(formatRunStatus("running")).toBe("running");
    expect(formatRunStatus("waiting_approval")).toBe("approval");
    expect(formatRunStatus("cancelled")).toBe("cancelled");
    expect(formatRunStatus("queued")).toBe("queued");
  });
});

describe("toolStatusLabel", () => {
  it("labels known statuses and falls back for unknown/empty", () => {
    expect(toolStatusLabel("completed")).toBe("Completed");
    expect(toolStatusLabel("failed")).toBe("Failed");
    expect(toolStatusLabel("cancelled")).toBe("Cancelled");
    expect(toolStatusLabel("running")).toBe("Running");
    expect(toolStatusLabel("pending")).toBe("pending");
    expect(toolStatusLabel("")).toBe("Unknown");
  });
});

describe("runTone", () => {
  it("maps statuses to tones", () => {
    expect(runTone("completed")).toBe("success");
    expect(runTone("failed")).toBe("danger");
    expect(runTone("cancelled")).toBe("danger");
    expect(runTone("waiting_approval")).toBe("warning");
    expect(runTone("running")).toBe("accent");
    expect(runTone("queued")).toBe("neutral");
  });
});

describe("shortId", () => {
  it("keeps the first two underscore segments", () => {
    expect(shortId("run_abc_def_ghi")).toBe("run_abc");
  });
});
