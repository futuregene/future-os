import { describe, expect, it } from "vitest";
import { formatErrorType } from "./runDisplayFormatters";

describe("formatErrorType", () => {
  it("returns null for null or undefined", () => {
    expect(formatErrorType(null)).toBeNull();
    expect(formatErrorType(undefined)).toBeNull();
  });

  it("formats stream_disconnected with orange color", () => {
    const result = formatErrorType("stream_disconnected");
    expect(result).not.toBeNull();
    expect(result!.label).toBe("Stream disconnected");
    expect(result!.icon).toBe("🔌");
    expect(result!.color).toBe("text-orange-600");
  });

  it("formats command_failed with red color", () => {
    const result = formatErrorType("command_failed");
    expect(result).not.toBeNull();
    expect(result!.label).toBe("Command failed");
    expect(result!.icon).toBe("⚠️");
    expect(result!.color).toBe("text-red-600");
  });

  it("formats model_failed with purple color", () => {
    const result = formatErrorType("model_failed");
    expect(result).not.toBeNull();
    expect(result!.label).toBe("Model failed");
    expect(result!.icon).toBe("🤖");
    expect(result!.color).toBe("text-purple-600");
  });

  it("formats abort_requested with gray color", () => {
    const result = formatErrorType("abort_requested");
    expect(result).not.toBeNull();
    expect(result!.label).toBe("Aborted by user");
    expect(result!.icon).toBe("⏹️");
    expect(result!.color).toBe("text-gray-600");
  });

  it("formats timeout with yellow color", () => {
    const result = formatErrorType("timeout");
    expect(result).not.toBeNull();
    expect(result!.label).toBe("Timeout");
    expect(result!.icon).toBe("⏰");
    expect(result!.color).toBe("text-yellow-600");
  });

  it("formats unknown with gray color", () => {
    const result = formatErrorType("unknown");
    expect(result).not.toBeNull();
    expect(result!.label).toBe("Unknown error");
    expect(result!.icon).toBe("❓");
    expect(result!.color).toBe("text-gray-600");
  });
});
