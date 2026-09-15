import ExpoModulesCore

public final class ShareIntentModule: Module {
  private var observer: NSObjectProtocol?

  public func definition() -> ModuleDefinition {
    Name("FutureShareIntent")
    Events("onPendingShare")

    OnCreate {
      self.observer = NotificationCenter.default.addObserver(
        forName: OpenFileReceiver.notification, object: nil, queue: .main
      ) { [weak self] _ in self?.sendEvent("onPendingShare", [:]) }
    }
    OnDestroy {
      if let observer = self.observer { NotificationCenter.default.removeObserver(observer) }
      self.observer = nil
    }

    AsyncFunction("getPendingShare") { () -> [String: Any]? in
      if OpenFileReceiver.failed {
        OpenFileReceiver.failed = false
        return ["text": "", "files": [], "tooLarge": false, "failed": true]
      }
      let cache = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("FutureSharedFiles")
      // Dismissed/import-failed shares are never retained indefinitely. Files
      // accepted as attachments also use the normal JS temporary-file cleanup.
      if let batches = try? FileManager.default.contentsOfDirectory(at: cache, includingPropertiesForKeys: [.creationDateKey]) {
        for batch in batches {
          let date = (try? batch.resourceValues(forKeys: [.creationDateKey]).creationDate) ?? .distantPast
          if Date().timeIntervalSince(date) > ShareInbox.lifetime { try? FileManager.default.removeItem(at: batch) }
        }
      }
      var received = try OpenFileReceiver.inbox().take(into: cache)
      if received == nil,
         let group = Bundle.main.object(forInfoDictionaryKey: "FutureShareAppGroup") as? String,
         let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) {
        let inbox = try ShareInbox(root: container.appendingPathComponent("FutureShareInbox"))
        received = try inbox.take(into: cache)
      }
      guard let (payload, directory) = received else { return nil }
      return [
        "text": payload.text,
        "tooLarge": payload.tooLarge,
        "files": payload.files.map { file in
          ["uri": directory.appendingPathComponent(file.path).absoluteString,
           "name": file.name, "mimeType": file.mimeType]
        }
      ]
    }.runOnQueue(OpenFileReceiver.queue)
  }
}
