import Foundation

// No XCTest/UIKit dependency: also runs with macOS Command Line Tools.
@main
struct ShareInboxTests {
  static func expect(_ condition: @autoclosure () -> Bool, _ message: String) {
    precondition(condition(), message)
  }

  static func expectFailure(_ action: () throws -> Void) {
    do { try action(); preconditionFailure("Expected failure") } catch { }
  }

  static func main() throws {
    let fm = FileManager.default
    let root = fm.temporaryDirectory.appendingPathComponent("future-share-test-" + UUID().uuidString)
    try fm.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? fm.removeItem(at: root) }
    let inbox = try ShareInbox(root: root.appendingPathComponent("inbox"))
    let cache = root.appendingPathComponent("cache")
    let source = root.appendingPathComponent("source.txt")
    try Data("hello".utf8).write(to: source)
    let staging = try inbox.begin()
    expect(tryValue { try inbox.take(into: cache) == nil }, "Staging shares must not be visible")
    let copied = try ShareInbox.copyFile(from: source, to: staging.appendingPathComponent("file.txt"), remaining: 5)
    expect(copied == 5, "Exact byte ceiling must be accepted")
    try inbox.commit(InboxPayload(text: "中文", files: [InboxFile(path: "file.txt", name: "报告.txt", mimeType: "text/plain")]), directory: staging)
    let received = try inbox.take(into: cache)!
    expect(received.0.text == "中文" && received.0.files[0].name == "报告.txt", "Unicode metadata must round-trip")
    expect(tryValue { try Data(contentsOf: received.1.appendingPathComponent("file.txt")) == Data("hello".utf8) }, "File bytes must survive handoff")
    expect(tryValue { try inbox.take(into: cache) == nil }, "A share is consumed only once")

    let partial = root.appendingPathComponent("partial")
    expectFailure { _ = try ShareInbox.copyFile(from: source, to: partial, remaining: 4) }
    expect(!fm.fileExists(atPath: partial.path), "Oversized copies must leave no partial file")
    let large = root.appendingPathComponent("large")
    try Data(repeating: 0, count: ShareInbox.maxFileBytes + 1).write(to: large)
    expectFailure { _ = try ShareInbox.copyFile(from: large, to: partial, remaining: ShareInbox.maxTotalBytes) }
    let symlink = root.appendingPathComponent("symlink")
    try fm.createSymbolicLink(at: symlink, withDestinationURL: source)
    expectFailure { _ = try ShareInbox.copyFile(from: symlink, to: partial, remaining: 10) }
    expectFailure { _ = try ShareInbox.copyFile(from: root, to: partial, remaining: 10) }

    let traversal = try inbox.begin()
    try inbox.commit(InboxPayload(files: [InboxFile(path: "../source.txt", name: "evil", mimeType: "text/plain")]), directory: traversal)
    let broken = try inbox.begin()
    try Data("not JSON".utf8).write(to: broken.appendingPathComponent("payload.json"))
    try fm.moveItem(at: broken, to: broken.deletingPathExtension().appendingPathExtension("ready"))
    let good = try inbox.begin()
    try inbox.commit(InboxPayload(text: "after poison"), directory: good)
    expect(tryValue { try inbox.take(into: cache)?.0.text == "after poison" }, "Invalid manifests cannot block good shares")
    expect(tryValue { try Data(contentsOf: source) == Data("hello".utf8) }, "Traversal cannot alter files outside the inbox")
    expect(tryValue { try inbox.take(into: cache) == nil }, "Invalid entries must be discarded")

    let retry = try inbox.begin()
    try inbox.commit(InboxPayload(text: "retry"), directory: retry)
    expectFailure { _ = try inbox.take(into: source) }
    expect(tryValue { try inbox.take(into: cache)?.0.text == "retry" }, "Cache IO failure must retain input")

    let empty = try inbox.begin()
    expectFailure { try inbox.commit(InboxPayload(), directory: empty) }
    try fm.removeItem(at: empty)
    let overText = try inbox.begin()
    expectFailure { try inbox.commit(InboxPayload(text: String(repeating: "x", count: ShareInbox.maxTextBytes + 1)), directory: overText) }
    try fm.removeItem(at: overText)
    // An oversize-only payload still reports the problem to the host app.
    let dropped = try inbox.begin()
    try inbox.commit(InboxPayload(tooLarge: true), directory: dropped)
    expect(tryValue { try inbox.take(into: cache)?.0.tooLarge == true }, "Oversize information must survive")

    // Open In requires no App Group and leaves the original file unchanged.
    for name in ["报告.doc", "报告.docx", "报告.pdf", "幻灯片.pptx", "笔记.md", "图.jpeg", "图.png"] {
      let document = root.appendingPathComponent(name)
      try Data("opened document".utf8).write(to: document)
      try inbox.importFile(from: document, mimeType: "application/octet-stream")
      let opened = try inbox.take(into: cache)!
      expect(opened.0.files.count == 1 && opened.0.files[0].name == name, "Open In must preserve Unicode filename/extension")
      expect(opened.0.files[0].mimeType == "application/octet-stream", "Open In preserves supplied MIME")
      expect(tryValue { try Data(contentsOf: opened.1.appendingPathComponent(opened.0.files[0].path)) == Data(contentsOf: document) }, "Open In copies source bytes")
      expect(tryValue { try inbox.take(into: cache) == nil }, "Open In consumed exactly once")
    }
    try inbox.importFile(from: large, mimeType: "application/pdf")
    expect(tryValue { try inbox.take(into: cache)?.0.tooLarge == true }, "Open In reports oversized files")
    expectFailure { try inbox.importFile(from: symlink, mimeType: "text/plain") }
    expectFailure { try inbox.importFile(from: root, mimeType: "text/plain") }
    expectFailure { try inbox.importFile(from: root.appendingPathComponent("missing"), mimeType: "text/plain") }
    let leftovers = try fm.contentsOfDirectory(at: inbox.root, includingPropertiesForKeys: nil)
    expect(!leftovers.contains { $0.pathExtension == "staging" }, "Failed opens must clean staging directories")

    for _ in 0..<ShareInbox.maxPending { _ = try inbox.begin() }
    expectFailure { _ = try inbox.begin() }
    expect(tryValue { try inbox.take(into: cache) == nil }, "In-flight batches do not appear as ready")
    print("ShareInbox: atomic handoff, retry, quotas, Unicode, malformed input and path safety passed")
  }

  private static func tryValue(_ action: () throws -> Bool) -> Bool { (try? action()) ?? false }
}
