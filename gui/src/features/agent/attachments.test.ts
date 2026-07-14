import { beforeEach, describe, expect, it, vi } from "vitest";
import { classifyAttachment, READ_SOURCE_MAX_BYTES } from "./attachments";

const inspectAttachment = vi.fn();
const validateImageAttachment = vi.fn();

vi.mock("../../integrations/storage/files", () => ({
  inspectAttachment: (...args: unknown[]) => inspectAttachment(...args),
  validateImageAttachment: (...args: unknown[]) => validateImageAttachment(...args),
}));

describe("classifyAttachment", () => {
  beforeEach(() => {
    inspectAttachment.mockReset();
    validateImageAttachment.mockReset();
    validateImageAttachment.mockResolvedValue(undefined);
  });

  it("rejects images over the byte limit", async () => {
    inspectAttachment.mockResolvedValue({ isBinary: true, isDir: false, size: READ_SOURCE_MAX_BYTES + 1 });

    await expect(classifyAttachment("/tmp/large.png")).resolves.toMatchObject({
      kind: null,
      reason: expect.stringContaining("25.0 MiB"),
    });
  });

  it("keeps non-image files unlimited", async () => {
    inspectAttachment.mockResolvedValue({ isBinary: true, isDir: false, size: READ_SOURCE_MAX_BYTES + 1 });

    await expect(classifyAttachment("/tmp/archive.zip")).resolves.toEqual({ kind: "file" });
  });

  it("rejects directories", async () => {
    inspectAttachment.mockResolvedValue({ isBinary: false, isDir: true, size: 0 });

    await expect(classifyAttachment("/tmp/folder")).resolves.toMatchObject({ kind: null });
  });

  it("rejects an image that cannot be decoded", async () => {
    inspectAttachment.mockResolvedValue({ isBinary: true, isDir: false, size: 1024 });
    validateImageAttachment.mockRejectedValue(new Error("bad image"));

    await expect(classifyAttachment("/tmp/broken.png")).resolves.toMatchObject({
      kind: null,
      reason: expect.stringContaining("broken.png"),
    });
  });
});
