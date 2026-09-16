import ExpoModulesCore
import Foundation
import CryptoKit
import UniformTypeIdentifiers

public final class FileHandlerModule: Module {
  private let presenter = FilePresenter()

  public func definition() -> ModuleDefinition {
    Name("FutureFileHandler")

    AsyncFunction("hashFile") { (url: URL) -> String in
      guard url.isFileURL, FileSystemUtilities.isReadableFile(self.appContext, url) else {
        throw Exception(name: "UnreadableFile", description: "The selected file is not readable")
      }
      let file = try FileHandle(forReadingFrom: url)
      defer { try? file.close() }
      var hash = SHA256()
      var total = 0
      while let data = try file.read(upToCount: 64 * 1024), !data.isEmpty {
        total += data.count
        guard total <= 10 * 1024 * 1024 else {
          throw Exception(name: "FileTooLarge", description: "File exceeds the hash size limit")
        }
        hash.update(data: data)
      }
      return hash.finalize().map { String(format: "%02x", $0) }.joined()
    }.runOnQueue(.global(qos: .utility))

    AsyncFunction("openFile") { (url: URL, mimeType: String, promise: Promise) in
      guard FileSystemUtilities.isReadableFile(self.appContext, url), url.isFileURL else {
        throw Exception(name: "UnreadableFile", description: "The selected file is not readable")
      }
      guard let controller = self.appContext?.utilities?.currentViewController() else {
        throw Exception(name: "MissingController", description: "No view controller is available")
      }
      self.presenter.open(url, mimeType: mimeType, from: controller, promise: promise)
    }.runOnQueue(.main)

    AsyncFunction("saveFile") { (url: URL, promise: Promise) in
      guard FileSystemUtilities.isReadableFile(self.appContext, url), url.isFileURL else {
        throw Exception(name: "UnreadableFile", description: "The selected file is not readable")
      }
      guard let controller = self.appContext?.utilities?.currentViewController() else {
        throw Exception(name: "MissingController", description: "No view controller is available")
      }
      self.presenter.save(url, from: controller, promise: promise)
    }.runOnQueue(.main)
  }
}

// Retain UIKit's weak delegates and the document interaction controller until
// dismissal. Every result, including cancellation, settles the promise once.
private final class FilePresenter: NSObject, UIDocumentInteractionControllerDelegate, UIDocumentPickerDelegate {
  private var pending: Promise?
  private var document: UIDocumentInteractionController?

  private func begin(_ promise: Promise) -> Bool {
    guard pending == nil else {
      promise.reject("FileActionBusy", "Another file action is already open")
      return false
    }
    pending = promise
    return true
  }

  func open(_ url: URL, mimeType: String, from controller: UIViewController, promise: Promise) {
    guard begin(promise) else { return }
    let document = UIDocumentInteractionController(url: url)
    document.uti = UTType(mimeType: mimeType)?.identifier
    document.delegate = self
    self.document = document
    // Required on iPad; on iPhone UIKit presents the native Open In sheet.
    let bounds = controller.view.bounds
    let anchor = CGRect(x: bounds.midX, y: bounds.midY, width: 1, height: 1)
    if !document.presentOpenInMenu(from: anchor, in: controller.view, animated: true) {
      pending?.reject("NoFileHandler", "No installed application can open this file")
      pending = nil
      self.document = nil
    }
  }

  func save(_ url: URL, from controller: UIViewController, promise: Promise) {
    guard begin(promise) else { return }
    let picker = UIDocumentPickerViewController(forExporting: [url], asCopy: true)
    picker.delegate = self
    picker.shouldShowFileExtensions = true
    controller.present(picker, animated: true)
  }

  private func finish() {
    let promise = pending
    pending = nil
    document = nil
    promise?.resolve(nil)
  }

  func documentInteractionControllerDidDismissOpenInMenu(_ controller: UIDocumentInteractionController) { finish() }
  func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { finish() }
  func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) { finish() }
}
