import UIKit
import Social
import UniformTypeIdentifiers

// Deliberately uses the supported compose-extension lifecycle. Do not reach
// through the responder chain to UIApplication to force-launch the host app.
final class ShareViewController: SLComposeServiceViewController {
  private var task: Task<Void, Never>?
  private var saving = false
  private var chinese: Bool { Locale.preferredLanguages.first?.hasPrefix("zh") == true }
  private func message(_ english: String, _ chinese: String) -> String { self.chinese ? chinese : english }

  override func viewDidLoad() {
    super.viewDidLoad()
    title = "FutureOS"
    navigationController?.navigationBar.topItem?.rightBarButtonItem?.title = message("Save", "保存")
    placeholder = message("Add a note. Open FutureOS after saving to choose a conversation.", "可添加说明。保存后打开 FutureOS，选择会话并确认发送。")
  }

  override func isContentValid() -> Bool { !saving }

  override func didSelectPost() {
    guard !saving else { return }
    saving = true
    validateContent()
    let note = contentText ?? ""
    let items = extensionContext?.inputItems as? [NSExtensionItem] ?? []
    task = Task {
      do {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "FutureShareAppGroup") as? String,
              let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else {
          throw ShareInboxError.unavailable
        }
        let inbox = try ShareInbox(root: container.appendingPathComponent("FutureShareInbox"))
        let directory = try inbox.begin()
        defer { try? FileManager.default.removeItem(at: directory) }
        var payload = InboxPayload()
        appendText(note, to: &payload)
        var total = 0
        var providersSeen = 0
        for item in items {
          let providers = item.attachments ?? []
          // Text-only senders may provide no NSItemProvider at all.
          if providers.isEmpty, let text = item.attributedContentText?.string, text != note {
            appendText(text, to: &payload)
          }
          for provider in providers {
            try Task.checkCancellation()
            providersSeen += 1
            guard providersSeen <= ShareInbox.maxFiles else { payload.tooLarge = true; continue }
            do {
              if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
                 !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                if let url = try await loadText(provider, type: UTType.url.identifier) {
                  appendText(url, to: &payload)
                }
              } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier),
                        !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier),
                        !provider.hasItemConformingToTypeIdentifier(UTType.image.identifier),
                        URL(fileURLWithPath: provider.suggestedName ?? "").pathExtension.isEmpty {
                if let text = try await loadText(provider, type: UTType.plainText.identifier), text != note {
                  appendText(text, to: &payload)
                }
              } else {
                let (file, bytes) = try await loadFile(provider, directory: directory, remaining: ShareInbox.maxTotalBytes - total)
                payload.files.append(file)
                total += bytes
              }
            } catch ShareInboxError.tooLarge {
              payload.tooLarge = true
            }
          }
        }
        try Task.checkCancellation()
        try inbox.commit(payload, directory: directory)
        let alert = UIAlertController(title: message("Saved to FutureOS", "已保存到 FutureOS"),
          message: message("Open FutureOS to choose a conversation and review before sending. Nothing has been uploaded.", "请打开 FutureOS 选择会话，检查内容后发送。当前未上传任何内容。"), preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: message("Done", "完成"), style: .default) { [weak self] _ in
          self?.extensionContext?.completeRequest(returningItems: nil)
        })
        present(alert, animated: true)
      } catch is CancellationError {
        // Cancellation never publishes a partially copied share.
      } catch {
        saving = false
        validateContent()
        let text = (error as? ShareInboxError) == .full
          ? message("The share inbox is full. Open FutureOS to import pending shares, then try again.", "分享收件箱已满，请先打开 FutureOS 导入待处理内容，再重试。")
          : message("Could not save this share. Check that the file is available and try again.", "无法保存此分享，请检查文件是否可用后重试。")
        let alert = UIAlertController(title: "FutureOS", message: text, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: message("OK", "确定"), style: .default))
        present(alert, animated: true)
      }
    }
  }

  override func didSelectCancel() {
    task?.cancel()
    super.didSelectCancel()
  }

  private func appendText(_ text: String, to payload: inout InboxPayload) {
    guard !text.isEmpty, text != payload.text else { return }
    let combined = payload.text.isEmpty ? text : payload.text + "\n\n" + text
    if combined.utf8.count > ShareInbox.maxTextBytes { payload.tooLarge = true }
    // UTF-8 prefix can split a scalar; decode then drop the replacement scalar
    // by truncating at the last valid boundary instead.
    var bytes = Array(combined.utf8.prefix(ShareInbox.maxTextBytes))
    while String(bytes: bytes, encoding: .utf8) == nil { bytes.removeLast() }
    payload.text = String(decoding: bytes, as: UTF8.self)
  }

  private func loadText(_ provider: NSItemProvider, type: String) async throws -> String? {
    try await withCheckedThrowingContinuation { continuation in
      provider.loadItem(forTypeIdentifier: type, options: nil) { item, error in
        if let error { continuation.resume(throwing: error) }
        else if let url = item as? URL { continuation.resume(returning: url.absoluteString) }
        else { continuation.resume(returning: item as? String) }
      }
    }
  }

  private func loadFile(_ provider: NSItemProvider, directory: URL, remaining: Int) async throws -> (InboxFile, Int) {
    let fileType = provider.registeredTypeIdentifiers.first {
      $0 != UTType.fileURL.identifier && $0 != UTType.url.identifier && (UTType($0)?.conforms(to: .data) ?? false)
    }
    let type = fileType ?? UTType.data.identifier
    return try await withCheckedThrowingContinuation { continuation in
      // Copy within this callback: provider URLs stop being valid after it returns.
      // No UIImage decoding or unbounded Data representation in the extension.
      let consume: (URL?, Error?) -> Void = { url, error in
        do {
          if let error { throw error }
          guard let url else { throw ShareInboxError.invalidFile }
          let scoped = url.startAccessingSecurityScopedResource()
          defer { if scoped { url.stopAccessingSecurityScopedResource() } }
          let suggested = provider.suggestedName ?? url.lastPathComponent
          var name = URL(fileURLWithPath: suggested).lastPathComponent
          if name.isEmpty || name == "." || name == ".." { name = "attachment" }
          if URL(fileURLWithPath: name).pathExtension.isEmpty,
             let ext = UTType(type)?.preferredFilenameExtension { name += "." + ext }
          let ext = URL(fileURLWithPath: name).pathExtension
          let filename = UUID().uuidString + (ext.isEmpty ? "" : "." + ext)
          let size = try ShareInbox.copyFile(from: url, to: directory.appendingPathComponent(filename), remaining: remaining)
          let mime = UTType(type)?.preferredMIMEType
            ?? UTType(filenameExtension: ext)?.preferredMIMEType ?? "application/octet-stream"
          continuation.resume(returning: (InboxFile(path: filename, name: name, mimeType: mime), size))
        } catch { continuation.resume(throwing: error) }
      }
      if fileType == nil, provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
        provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, error in
          consume(item as? URL, error)
        }
      } else {
        provider.loadFileRepresentation(forTypeIdentifier: type, completionHandler: consume)
      }
    }
  }
}
