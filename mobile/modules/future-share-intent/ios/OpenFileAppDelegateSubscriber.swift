import ExpoModulesCore
import UniformTypeIdentifiers

// A local inbox works in every host build, without Share Extension signing.
// Reads use this same serial queue, so a foreground read cannot overtake a copy.
enum OpenFileReceiver {
  static let queue = DispatchQueue(label: "cn.futureos.file-intake")
  static let notification = Notification.Name("FutureOpenFileReady")
  // Accessed only on queue. Surface provider/copy failures rather than silently
  // opening the app with no explanation. No source paths enter events or logs.
  static var failed = false

  static func inbox() throws -> ShareInbox {
    let support = try FileManager.default.url(for: .applicationSupportDirectory,
                                              in: .userDomainMask, appropriateFor: nil, create: true)
    return try ShareInbox(root: support.appendingPathComponent("FutureOpenFiles"))
  }

  static func receive(_ url: URL) -> Bool {
    guard url.isFileURL else { return false }
    // Acquire while the OS-delivered URL is valid, and retain access until the
    // coordinated background copy finishes (including failure paths).
    let scoped = url.startAccessingSecurityScopedResource()
    queue.async {
      defer {
        // With open-in-place disabled UIKit may first copy into Documents/Inbox.
        // Reclaim only that app-owned copy, never an external provider's original.
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let systemInbox = documents.appendingPathComponent("Inbox").resolvingSymlinksInPath()
        if url.deletingLastPathComponent().resolvingSymlinksInPath() == systemInbox {
          try? FileManager.default.removeItem(at: url)
        }
        if scoped { url.stopAccessingSecurityScopedResource() }
        DispatchQueue.main.async {
          NotificationCenter.default.post(name: notification, object: nil)
        }
      }
      var coordinationError: NSError?
      var importError: Error?
      NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &coordinationError) { source in
        do {
          let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
          try inbox().importFile(from: source, mimeType: mime)
        } catch { importError = error }
      }
      if coordinationError != nil || importError != nil { failed = true }
    }
    return true
  }
}

public final class OpenFileAppDelegateSubscriber: ExpoAppDelegateSubscriber {
  // UIKit delivers this for both cold and warm document opens. Do not also
  // import launchOptions[.url], which would duplicate a cold-start document.
  public func application(_ app: UIApplication, open url: URL,
                          options: [UIApplication.OpenURLOptionsKey: Any] = [:]) -> Bool {
    OpenFileReceiver.receive(url)
  }
}
