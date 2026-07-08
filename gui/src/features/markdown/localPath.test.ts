import { describe, expect, it } from "vitest";
import { localFilePath } from "./localPath";

describe("localFilePath", () => {
  it("treats POSIX absolute paths as local", () => {
    expect(localFilePath("/Users/tao/Desktop/poem2.txt")).toBe("/Users/tao/Desktop/poem2.txt");
  });

  it("treats explicit relative paths as local and strips a leading ./", () => {
    expect(localFilePath("./poem2.txt")).toBe("poem2.txt");
    expect(localFilePath("./sub/dir/x.txt")).toBe("sub/dir/x.txt");
  });

  it("keeps ../ relative paths intact", () => {
    expect(localFilePath("../sibling/x.txt")).toBe("../sibling/x.txt");
  });

  it("treats Windows drive and UNC paths as local", () => {
    expect(localFilePath("C:/Users/tao/report.txt")).toBe("C:/Users/tao/report.txt");
    expect(localFilePath("C:\\Users\\tao\\report.txt")).toBe("C:\\Users\\tao\\report.txt");
    expect(localFilePath("\\\\server\\share\\file.txt")).toBe("\\\\server\\share\\file.txt");
  });

  it("decodes file:// URIs to their plain path", () => {
    expect(localFilePath("file:///Users/tao/a%20b.txt")).toBe("/Users/tao/a b.txt");
  });

  it("does not treat remote links as local", () => {
    expect(localFilePath("https://example.com/page")).toBeNull();
    expect(localFilePath("http://example.com")).toBeNull();
    expect(localFilePath("mailto:a@b.com")).toBeNull();
    expect(localFilePath("futureos://run/run_123")).toBeNull();
  });

  it("does not treat bare scheme-less tokens as local (ambiguous with domains)", () => {
    expect(localFilePath("example.com/page")).toBeNull();
    expect(localFilePath("poem2.txt")).toBeNull();
    expect(localFilePath("")).toBeNull();
  });
});
