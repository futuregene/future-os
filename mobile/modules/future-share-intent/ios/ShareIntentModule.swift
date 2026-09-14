import ExpoModulesCore

public final class ShareIntentModule: Module {
  private let queue = DispatchQueue(label: "cn.futureos.share-inbox")

  public func definition() -> ModuleDefinition {
    Name("FutureShareIntent")

    AsyncFunction("getPendingShare") { () -> [String: Any]? in
      guard let group = Bundle.main.object(forInfoDictionaryKey: "FutureShareAppGroup") as? String,
            let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else {
        return nil
      }
      let inbox = try ShareInbox(root: container.appendingPathComponent("FutureShareInbox"))
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
      guard let (payload, directory) = try inbox.take(into: cache) else { return nil }
      return [
        "text": payload.text,
        "tooLarge": payload.tooLarge,
        "files": payload.files.map { file in
          ["uri": directory.appendingPathComponent(file.path).absoluteString,
           "name": file.name, "mimeType": file.mimeType]
        }
      ]
    }.runOnQueue(queue)
  }
}
