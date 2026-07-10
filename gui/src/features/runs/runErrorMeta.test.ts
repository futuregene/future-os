import { Bot, CircleHelp, CircleStop, Clock, TriangleAlert, Unplug } from "lucide-react";
import { describe, expect, it } from "vitest";
import { errorTypeMeta } from "./runErrorMeta";

describe("errorTypeMeta", () => {
  it("returns null for null or undefined", () => {
    expect(errorTypeMeta(null)).toBeNull();
    expect(errorTypeMeta(undefined)).toBeNull();
  });

  it("maps stream_disconnected to its icon, category color, and label key", () => {
    expect(errorTypeMeta("stream_disconnected")).toEqual({ Icon: Unplug, color: "text-orange-600", labelKey: "errorType.streamDisconnected" });
  });

  it("maps command_failed", () => {
    expect(errorTypeMeta("command_failed")).toEqual({ Icon: TriangleAlert, color: "text-red-600", labelKey: "errorType.commandFailed" });
  });

  it("maps model_failed", () => {
    expect(errorTypeMeta("model_failed")).toEqual({ Icon: Bot, color: "text-purple-600", labelKey: "errorType.modelFailed" });
  });

  it("maps abort_requested", () => {
    expect(errorTypeMeta("abort_requested")).toEqual({ Icon: CircleStop, color: "text-gray-600", labelKey: "errorType.abortRequested" });
  });

  it("maps timeout", () => {
    expect(errorTypeMeta("timeout")).toEqual({ Icon: Clock, color: "text-yellow-600", labelKey: "errorType.timeout" });
  });

  it("maps unknown", () => {
    expect(errorTypeMeta("unknown")).toEqual({ Icon: CircleHelp, color: "text-gray-600", labelKey: "errorType.unknown" });
  });
});
