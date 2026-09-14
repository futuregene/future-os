import Foundation
import Darwin

// Shared by the extension and host app. No network, credentials, or React runtime
// in the extension. Only committed directories are visible to the reader.
struct InboxFile: Codable {
  let path: String
  let name: String
  let mimeType: String
}

struct InboxPayload: Codable {
  var text = ""
  var files: [InboxFile] = []
  var tooLarge = false
}

enum ShareInboxError: Error, Equatable {
  case unavailable, full, tooLarge, invalidFile, empty
}

final class ShareInbox {
  static let maxFileBytes = 10 * 1024 * 1024
  static let maxTotalBytes = 20 * 1024 * 1024
  static let maxFiles = 10
  static let maxTextBytes = 256 * 1024
  static let maxPending = 10
  static let lifetime: TimeInterval = 7 * 24 * 3600
  let root: URL
  private let fm = FileManager.default

  init(root: URL) throws {
    self.root = root
    try fm.createDirectory(at: root, withIntermediateDirectories: true)
    var directory = root
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try directory.setResourceValues(values)
  }

  func begin() throws -> URL {
    try locked {
      try prune()
      // Include in-flight extensions: simultaneous shares cannot fill the disk.
      guard try entries().count < Self.maxPending else { throw ShareInboxError.full }
      let directory = root.appendingPathComponent(UUID().uuidString + ".staging", isDirectory: true)
      try fm.createDirectory(at: directory, withIntermediateDirectories: false)
      return directory
    }
  }

  func commit(_ payload: InboxPayload, directory: URL) throws {
    guard !payload.text.isEmpty || !payload.files.isEmpty || payload.tooLarge else {
      throw ShareInboxError.empty
    }
    guard payload.files.count <= Self.maxFiles, payload.text.utf8.count <= Self.maxTextBytes else {
      throw ShareInboxError.tooLarge
    }
    try JSONEncoder().encode(payload).write(to: directory.appendingPathComponent("payload.json"), options: .atomic)
    try locked {
      try fm.moveItem(at: directory, to: directory.deletingPathExtension().appendingPathExtension("ready"))
    }
  }

  // Called only after pairing. Move to the app's cache before removing the queue
  // entry; a failed move leaves the share available for the next foreground read.
  func take(into cache: URL) throws -> (InboxPayload, URL)? {
    try locked {
      try prune()
      let ready = try entries().filter { $0.pathExtension == "ready" }.sorted {
        let left = (try? $0.resourceValues(forKeys: [.creationDateKey]).creationDate) ?? .distantPast
        let right = (try? $1.resourceValues(forKeys: [.creationDateKey]).creationDate) ?? .distantPast
        return left < right
      }
      for directory in ready {
        let payload: InboxPayload
        do {
          let manifest = directory.appendingPathComponent("payload.json")
          guard (try manifest.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? Int.max) <= Self.maxTextBytes * 8 else {
            throw ShareInboxError.invalidFile
          }
          payload = try JSONDecoder().decode(InboxPayload.self, from: Data(contentsOf: manifest))
          guard payload.text.utf8.count <= Self.maxTextBytes, payload.files.count <= Self.maxFiles else {
            throw ShareInboxError.invalidFile
          }
          var total = 0
          for file in payload.files {
            // Never allow a manifest to escape the private batch directory.
            guard !file.path.isEmpty, file.path != ".", file.path != "..",
                  !file.path.contains("/"), !file.path.contains("\\") else {
              throw ShareInboxError.invalidFile
            }
            let values = try directory.appendingPathComponent(file.path).resourceValues(
              forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
            let size = values.fileSize ?? Int.max
            guard values.isRegularFile == true, values.isSymbolicLink != true,
                  size <= Self.maxFileBytes, size <= Self.maxTotalBytes - total else {
              throw ShareInboxError.invalidFile
            }
            total += size
          }
        } catch {
          // Bad entries must not poison the queue.
          try fm.removeItem(at: directory)
          continue
        }
        try fm.createDirectory(at: cache, withIntermediateDirectories: true)
        let destination = cache.appendingPathComponent(directory.deletingPathExtension().lastPathComponent)
        // IO failure leaves the queue entry intact for the next foreground read.
        try fm.moveItem(at: directory, to: destination)
        return (payload, destination)
      }
      return nil
    }
  }

  private func entries() throws -> [URL] {
    try fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.creationDateKey])
      .filter { ["ready", "staging"].contains($0.pathExtension) }
  }

  private func prune() throws {
    for url in try entries() {
      let date = (try? url.resourceValues(forKeys: [.creationDateKey]).creationDate) ?? .distantPast
      let age = url.pathExtension == "staging" ? 24 * 3600.0 : Self.lifetime
      if Date().timeIntervalSince(date) > age { try fm.removeItem(at: url) }
    }
  }

  private func locked<T>(_ action: () throws -> T) throws -> T {
    let descriptor = Darwin.open(root.appendingPathComponent(".lock").path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
    guard descriptor >= 0 else { throw ShareInboxError.unavailable }
    defer { Darwin.close(descriptor) }
    while flock(descriptor, LOCK_EX) != 0 {
      if errno != EINTR { throw ShareInboxError.unavailable }
    }
    defer { flock(descriptor, LOCK_UN) }
    return try action()
  }

  // Bounded streaming copy, not Data(contentsOf:) on untrusted provider files.
  // The extra byte detects providers whose advertised length was incorrect.
  static func copyFile(from source: URL, to destination: URL, remaining: Int) throws -> Int {
    let values = try source.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
    guard source.isFileURL, values.isRegularFile == true, values.isSymbolicLink != true else {
      throw ShareInboxError.invalidFile
    }
    let limit = min(maxFileBytes, remaining)
    guard limit >= 0, (values.fileSize ?? 0) <= limit else { throw ShareInboxError.tooLarge }
    guard let input = InputStream(url: source), let output = OutputStream(url: destination, append: false) else {
      throw ShareInboxError.invalidFile
    }
    input.open()
    output.open()
    var completed = false
    defer {
      input.close()
      output.close()
      if !completed { try? FileManager.default.removeItem(at: destination) }
    }
    var buffer = [UInt8](repeating: 0, count: 32 * 1024)
    var total = 0
    while true {
      let count = input.read(&buffer, maxLength: min(buffer.count, limit - total + 1))
      guard count >= 0 else { throw input.streamError ?? ShareInboxError.invalidFile }
      if count == 0 { break }
      guard count <= limit - total else { throw ShareInboxError.tooLarge }
      try buffer.withUnsafeBufferPointer { bytes in
        var written = 0
        while written < count {
          let result = output.write(bytes.baseAddress! + written, maxLength: count - written)
          guard result > 0 else { throw output.streamError ?? ShareInboxError.invalidFile }
          written += result
        }
      }
      total += count
    }
    completed = true
    return total
  }
}
