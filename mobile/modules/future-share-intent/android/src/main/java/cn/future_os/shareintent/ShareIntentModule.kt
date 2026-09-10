package cn.future_os.shareintent

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

/**
 * Hands the pending share payload to JavaScript exactly once.
 *
 * Returns `null` when nothing was shared, so the caller can poll this on every
 * foreground without having to know whether the app was started by a share.
 */
class ShareIntentModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("FutureShareIntent")

    AsyncFunction("getPendingShare") {
      val context = appContext.reactContext ?: return@AsyncFunction null
      val content = ShareIntentStore.take(context) ?: return@AsyncFunction null
      mapOf(
        "text" to content.text,
        "tooLarge" to content.tooLarge,
        "files" to content.files.map { file ->
          mapOf(
            "uri" to file.uri,
            "name" to file.name,
            "mimeType" to file.mimeType,
          )
        },
      )
    }
  }
}
