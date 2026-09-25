package cn.future_os.filehandler

import android.content.ClipData
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import androidx.core.content.FileProvider
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.functions.Queues
import java.io.File
import java.security.MessageDigest

class FileHandlerModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("FutureFileHandler")

    AsyncFunction("hashFile") { fileUrl: String ->
      val context = appContext.reactContext ?: error("React context is unavailable")
      val uri = readableContentUri(fileUrl)
      val hash = MessageDigest.getInstance("SHA-256")
      context.contentResolver.openInputStream(uri).use { input ->
        requireNotNull(input) { "The selected file is not readable" }
        val buffer = ByteArray(64 * 1024)
        var total = 0L
        while (true) {
          val count = input.read(buffer)
          if (count < 0) break
          total += count
          require(total <= 10L * 1024 * 1024) { "File exceeds the hash size limit" }
          hash.update(buffer, 0, count)
        }
      }
      hash.digest().joinToString("") { "%02x".format(it.toInt() and 0xff) }
    }.runOnQueue(Queues.DEFAULT)

    AsyncFunction("findSupportedMimeType") { fileName: String, mimeTypes: List<String> ->
      mimeTypes.firstOrNull { canHandle(fileName, it) }
    }

    // Which apps would answer the album and photo-picker intents on this device.
    // Android's photo surface is OEM-specific: AndroidX's photo-picker contract
    // silently degrades to the document picker wherever no system photo picker
    // exists, and some phones hand the classic gallery intent to a file manager.
    // Resolving the candidates first lets JS open a real gallery, fall back to a
    // real photo picker, or report the album as unavailable — instead of showing
    // a file browser under an album label.
    AsyncFunction("resolveImagePickRoutes") {
      val context = appContext.reactContext ?: error("React context is unavailable")
      val packageManager = context.packageManager

      fun handlers(action: String, data: String?, type: String?): List<Map<String, String>> {
        val intent = Intent(action).apply {
          if (data != null) {
            setDataAndType(Uri.parse(data), type)
          } else {
            this.type = type
          }
        }
        // MATCH_DEFAULT_ONLY mirrors how the system resolves an implicit
        // startActivity, so the answer matches what launching would do.
        return packageManager
          .queryIntentActivities(intent, PackageManager.MATCH_DEFAULT_ONLY)
          .map { info ->
            mapOf(
              "package" to info.activityInfo.packageName,
              "activity" to info.activityInfo.name
            )
          }
      }

      mapOf<String, Any>(
        "sdkInt" to Build.VERSION.SDK_INT,
        // The album intent: MediaStore's image collection with the image type.
        "album" to handlers(Intent.ACTION_PICK, "content://media/external/images/media", "image/*"),
        // Android 13's system photo picker.
        "photoPicker" to handlers("android.provider.action.PICK_IMAGES", null, "image/*"),
        // The photo picker backport AOSP ships to Android 11/12 devices.
        "photoPickerFallback" to handlers(
          "androidx.activity.result.contract.action.PICK_IMAGES",
          null,
          "image/*"
        ),
        // Diagnostics only: the document picker an album must never open.
        "document" to handlers(Intent.ACTION_OPEN_DOCUMENT, null, "image/*")
      )
    }.runOnQueue(Queues.DEFAULT)

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
