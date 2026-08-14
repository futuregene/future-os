import { basename, localFilePath } from "../localPath";

describe("localFilePath", () => {
  test("treats POSIX absolute paths as local", () => {
    expect(localFilePath("/Users/tao/Desktop/poem2.txt")).toBe("/Users/tao/Desktop/poem2.txt");
  });

  test("treats explicit relative paths as local and strips a leading ./", () => {
    expect(localFilePath("./poem2.txt")).toBe("poem2.txt");
    expect(localFilePath("./sub/dir/x.txt")).toBe("sub/dir/x.txt");
  });

  test("keeps ../ relative paths intact", () => {
    expect(localFilePath("../sibling/x.txt")).toBe("../sibling/x.txt");
  });

  test("treats Windows drive and UNC paths as local", () => {
    expect(localFilePath("C:/Users/tao/report.txt")).toBe("C:/Users/tao/report.txt");
    expect(localFilePath("C:\\Users\\tao\\report.txt")).toBe("C:\\Users\\tao\\report.txt");
    expect(localFilePath("\\\\server\\share\\file.txt")).toBe("\\\\server\\share\\file.txt");
  });

  test("decodes file:// URIs to their plain path", () => {
    expect(localFilePath("file:///Users/tao/a%20b.txt")).toBe("/Users/tao/a b.txt");
  });

  test("does not treat remote links as local", () => {
    expect(localFilePath("https://example.com/page")).toBeNull();
    expect(localFilePath("http://example.com")).toBeNull();
    expect(localFilePath("mailto:a@b.com")).toBeNull();
    expect(localFilePath("futureos://run/run_123")).toBeNull();
  });

  test("treats a bare relative path with a separator as local (non-domain first segment)", () => {
    expect(localFilePath("docs/readme.md")).toBe("docs/readme.md");
    expect(localFilePath("src/main.rs")).toBe("src/main.rs");
    expect(localFilePath("assets/img/logo.png")).toBe("assets/img/logo.png");
  });

  test("treats a bare single-token filename with a known extension as local", () => {
    expect(localFilePath("长诗.md")).toBe("长诗.md");
    expect(localFilePath("poem2.txt")).toBe("poem2.txt");
    expect(localFilePath("config.json")).toBe("config.json");
    expect(localFilePath("main.rs")).toBe("main.rs");
  });

  test("still leaves bare domains and non-file tokens to the browser", () => {
    expect(localFilePath("example.com/page")).toBeNull();
    expect(localFilePath("github.com/user/repo")).toBeNull();
    expect(localFilePath("example.com")).toBeNull();
    expect(localFilePath("google.co.uk")).toBeNull();
    expect(localFilePath("README")).toBeNull();
    expect(localFilePath("some.unknownext")).toBeNull();
    expect(localFilePath("")).toBeNull();
  });

  test("returns null when the file:// URI cannot be decoded", () => {
    expect(localFilePath("file:///%E0%A4%A")).toBeNull();
  });
});

describe("basename", () => {
  test("extracts the last segment across POSIX and Windows separators", () => {
    expect(basename("/Users/tao/Desktop/report.md")).toBe("report.md");
    expect(basename("C:\\Users\\tao\\report.md")).toBe("report.md");
    expect(basename("docs/readme.md")).toBe("readme.md");
    expect(basename("report.md")).toBe("report.md");
  });
});
