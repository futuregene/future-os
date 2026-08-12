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

describe("attachment helpers", () => {
  it("maps image MIME types to extensions, case-insensitively", async () => {
    const { imageExtensionFromMime } = await import("./attachments");
    expect(imageExtensionFromMime("image/PNG")).toBe("png");
    expect(imageExtensionFromMime("image/gif")).toBe("gif");
    expect(imageExtensionFromMime("text/plain")).toBeNull();
  });

  it("splits filenames into stem and extension", async () => {
    const { splitFileName } = await import("./attachments");
    expect(splitFileName("photo.png")).toEqual({ stem: "photo", ext: ".png" });
    expect(splitFileName(".gitignore")).toEqual({ stem: ".gitignore", ext: "" });
    expect(splitFileName("README")).toEqual({ stem: "README", ext: "" });
  });

  it("fileNameFromPath falls back to the whole path", async () => {
    const { fileNameFromPath } = await import("./attachments");
    expect(fileNameFromPath("/a/b/c.txt")).toBe("c.txt");
  });

  it("reports a read failure when inspection throws", async () => {
    inspectAttachment.mockRejectedValue(new Error("perm"));
    await expect(classifyAttachment("/tmp/x.png")).resolves.toMatchObject({ kind: null });
  });
});
