package cn.future_os.filehandler

import android.content.ClipData
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import androidx.core.content.FileProvider
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.functions.Queues
import java.io.File

class FileHandlerModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("FutureFileHandler")

    AsyncFunction("findSupportedMimeType") { fileName: String, mimeTypes: List<String> ->
      mimeTypes.firstOrNull { canHandle(fileName, it) }
    }

    // Sharing only needs a successful handoff, not an activity result (which
    // compatibility runtimes may never deliver). Do not keep ExpoSharing's
    // process-wide pending promise: it can permanently block later shares.
    AsyncFunction("shareFile") { fileUrl: String, mimeType: String, title: String ->
      val activity = appContext.throwingActivity
      val contentUri = readableContentUri(fileUrl)
      val send = Intent(Intent.ACTION_SEND).apply {
        type = mimeType
        putExtra(Intent.EXTRA_STREAM, contentUri)
        clipData = ClipData.newRawUri("", contentUri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      activity.startActivity(Intent.createChooser(send, title))
    }.runOnQueue(Queues.MAIN)

    AsyncFunction("openFile") { fileUrl: String, mimeType: String ->
      val context = appContext.reactContext ?: error("React context is unavailable")
      val contentUri = readableContentUri(fileUrl)
      val intent = Intent.createChooser(
        Intent(Intent.ACTION_VIEW).apply {
          setDataAndType(contentUri, mimeType)
          addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        },
        null
      ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      context.startActivity(intent)
    }
  }

  private fun readableContentUri(fileUrl: String): Uri {
    val context = appContext.reactContext ?: error("React context is unavailable")
    val sourceUri = Uri.parse(fileUrl)
    require(sourceUri.scheme == "file") { "Only local file URLs can be opened" }
    val file = File(requireNotNull(sourceUri.path) { "The file URL has no path" }).canonicalFile
    val allowedRoots = listOfNotNull(context.cacheDir, context.filesDir, context.getExternalFilesDir(null))
      .map { it.canonicalFile }
    require(allowedRoots.any { file.path == it.path || file.path.startsWith("${it.path}/") }) {
      "The file is outside app-owned storage"
    }
    require(file.isFile && file.canRead()) { "The selected file is not readable" }
    return FileProvider.getUriForFile(context, "${context.packageName}.FutureFileHandlerProvider", file)
  }

  private fun canHandle(fileName: String, mimeType: String): Boolean {
    val context = appContext.reactContext ?: return false
    val packageManager = context.packageManager
    val placeholder = Uri.Builder()
      .scheme("content")
      .authority("${context.packageName}.file-check")
      .appendPath(fileName)
      .build()
    val viewIntent = Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(placeholder, mimeType)
      addCategory(Intent.CATEGORY_DEFAULT)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    val flags = PackageManager.MATCH_DEFAULT_ONLY
    return packageManager.queryIntentActivities(viewIntent, flags).isNotEmpty()
  }
}
