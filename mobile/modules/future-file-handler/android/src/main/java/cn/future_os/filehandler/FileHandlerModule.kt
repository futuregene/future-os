package cn.future_os.filehandler

import android.content.ClipData
import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.functions.Queues
import java.io.File
import java.security.MessageDigest

private val IMAGE_FILE_EXTENSIONS = setOf(
  "jpg", "jpeg", "png", "gif", "webp", "bmp", "heic", "heif"
)

// Where photos live on an Android phone. The album grid falls back to walking
// these when MediaStore has nothing for the app (some compatibility runtimes
// map shared storage without exposing the media database).
private val ALBUM_SCAN_DIRECTORIES = listOf(
  "DCIM", "Pictures", "Download", "Screenshots", "Movies", "Documents", "Android/media"
)

private const val ALBUM_SCAN_MAX_DIRECTORIES = 4_000

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
        // The legacy "give me an image" intent. Galleries register this too, and
        // some devices only register it.
        "imageContent" to handlers(Intent.ACTION_GET_CONTENT, null, "image/*"),
        // Android 13's system photo picker.
        "photoPicker" to handlers("android.provider.action.PICK_IMAGES", null, "image/*"),
        // The photo picker backport AOSP ships to Android 11/12 devices.
        "photoPickerFallback" to handlers(
          "androidx.activity.result.contract.action.PICK_IMAGES",
          null,
          "image/*"
        ),
        // The Play-services build of that backport.
        "photoPickerPlayServices" to handlers("com.google.android.gms.provider.action.PICK_IMAGES", null, "image/*"),
        // Diagnostics only: the document picker an album must never open.
        "document" to handlers(Intent.ACTION_OPEN_DOCUMENT, null, "image/*")
      )
    }.runOnQueue(Queues.DEFAULT)

    // The album grid for devices that cannot present a system picker at all —
    // no gallery app and no photo picker, as inside an Android compatibility
    // container. Reading the library stays the caller's decision: JS asks for
    // the media permission first, so this only reports what is actually
    // readable. The attachment pipeline then treats each choice like any other
    // picked image.
    AsyncFunction("listAlbumImages") { limit: Int ->
      val context = appContext.reactContext ?: error("React context is unavailable")
      require(limit > 0) { "The album limit must be positive" }
      val indexed = runCatching { mediaStoreImages(context, limit) }.getOrDefault(emptyList())
      if (indexed.isNotEmpty()) {
        indexed
      } else {
        scanAlbumDirectories(limit)
      }
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

  /** Images the media database knows about, newest first. */
  private fun mediaStoreImages(context: Context, limit: Int): List<Map<String, Any>> {
    val collection = MediaStore.Images.Media.EXTERNAL_CONTENT_URI
    val projection = arrayOf(
      MediaStore.Images.Media._ID,
      MediaStore.Images.Media.DISPLAY_NAME,
      MediaStore.Images.Media.MIME_TYPE,
      MediaStore.Images.Media.SIZE,
      MediaStore.Images.Media.DATE_MODIFIED
    )
    val sortOrder = "${MediaStore.Images.Media.DATE_MODIFIED} DESC"
    val images = mutableListOf<Map<String, Any>>()
    val cursor = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val args = Bundle().apply {
        putStringArray(ContentResolver.QUERY_ARG_SORT_COLUMNS, arrayOf(MediaStore.Images.Media.DATE_MODIFIED))
        putInt(ContentResolver.QUERY_ARG_SORT_DIRECTION, ContentResolver.QUERY_SORT_DIRECTION_DESCENDING)
        putInt(ContentResolver.QUERY_ARG_LIMIT, limit)
      }
      context.contentResolver.query(collection, projection, args, null)
    } else {
      context.contentResolver.query(collection, projection, null, null, sortOrder)
    }
    cursor?.use { rows ->
      val idColumn = rows.getColumnIndexOrThrow(MediaStore.Images.Media._ID)
      val nameColumn = rows.getColumnIndex(MediaStore.Images.Media.DISPLAY_NAME)
      val mimeColumn = rows.getColumnIndex(MediaStore.Images.Media.MIME_TYPE)
      val sizeColumn = rows.getColumnIndex(MediaStore.Images.Media.SIZE)
      val modifiedColumn = rows.getColumnIndex(MediaStore.Images.Media.DATE_MODIFIED)
      while (rows.moveToNext() && images.size < limit) {
        val id = rows.getLong(idColumn)
        images += albumImage(
          uri = ContentUris.withAppendedId(collection, id).toString(),
          name = nameColumn.takeIf { it >= 0 }?.let { rows.getString(it) } ?: "image-$id",
          mimeType = mimeColumn.takeIf { it >= 0 }?.let { rows.getString(it) },
          size = sizeColumn.takeIf { it >= 0 }?.let { rows.getLong(it) } ?: 0L,
          modified = (modifiedColumn.takeIf { it >= 0 }?.let { rows.getLong(it) } ?: 0L) * 1_000L
        )
      }
    }
    return images
  }

  /** Files under the usual photo directories, newest first. */
  private fun scanAlbumDirectories(limit: Int): List<Map<String, Any>> {
    val storage = Environment.getExternalStorageDirectory() ?: return emptyList()
    val found = mutableListOf<Map<String, Any>>()
    var visited = 0

    fun scan(directory: File, depth: Int) {
      if (found.size >= limit || visited >= ALBUM_SCAN_MAX_DIRECTORIES || depth > 4) return
      val children = directory.listFiles() ?: return
      for (child in children) {
        if (found.size >= limit || visited >= ALBUM_SCAN_MAX_DIRECTORIES) return
        visited += 1
        if (child.isDirectory) {
          // Hidden directories hold caches and thumbnails, not the user's photos.
          if (!child.name.startsWith(".")) scan(child, depth + 1)
        } else if (isImageFile(child.name)) {
          found += albumImage(
            uri = Uri.fromFile(child).toString(),
            name = child.name,
            mimeType = MimeTypeMap.getSingleton()
              .getMimeTypeFromExtension(child.extension.lowercase()),
            size = child.length(),
            modified = child.lastModified()
          )
        }
      }
    }

    for (name in ALBUM_SCAN_DIRECTORIES) {
      val directory = File(storage, name)
      if (directory.isDirectory) scan(directory, 0)
      if (found.size >= limit) break
    }
    return found.sortedByDescending { it["modified"] as Long }
  }

  private fun albumImage(
    uri: String,
    name: String,
    mimeType: String?,
    size: Long,
    modified: Long
  ): Map<String, Any> = mapOf(
    "uri" to uri,
    "name" to name,
    "mimeType" to (mimeType ?: "image/jpeg"),
    "size" to size,
    "modified" to modified
  )

  private fun isImageFile(name: String): Boolean {
    val extension = name.substringAfterLast('.', "").lowercase()
    return extension.isNotEmpty() && extension in IMAGE_FILE_EXTENSIONS
  }
}
