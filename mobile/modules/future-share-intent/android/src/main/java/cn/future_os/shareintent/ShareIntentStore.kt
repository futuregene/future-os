package cn.future_os.shareintent

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import java.io.File
import java.util.UUID

/** One file carried by a share intent, copied into app-owned storage. */
data class SharedFile(val uri: String, val name: String, val mimeType: String)

/** The parsed payload of a share intent, or null when there was nothing to share. */
data class SharedContent(
  val text: String,
  val files: List<SharedFile>,
  /** At least one shared file exceeded the per-file, batch-byte or item-count limit. */
  val tooLarge: Boolean,
  val failed: Boolean = false,
)

/**
 * Holds the payload of the share intent that launched (or resumed) the app until
 * JavaScript asks for it.
 *
 * Two receipts matter, and both are handled by [ShareIntentStore.capture]: a cold
 * start, where the share intent *is* the launch intent, and a warm start, where
 * `MainActivity` is `singleTask` and the framework calls `onNewIntent` — in that
 * case `Activity.getIntent()` still returns the *original* intent, so the new one
 * has to be captured by the lifecycle listener rather than read from the activity.
 *
 * Only the most recent share is kept: sharing something twice before the app
 * reads the first payload should not queue two conversations. Nothing is stored
 * for an unrelated intent (the launcher intent, a `futureos://` deep link).
 * ACTION_VIEW documents use the same private-copy/attachment path as shares.
 */
object ShareIntentStore {
  /** Per-file ceiling. The composer validates against its own (smaller) limit. */
  const val MAX_BYTES = ShareFileCopier.MAX_FILE_BYTES

  /** Shared copies older than this are reclaimed when the next share arrives. */
  private const val RETAIN_MILLIS = 24L * 60 * 60 * 1000

  private val lock = Any()
  private var pending: PendingShare? = null

  @Volatile
  var onPending: (() -> Unit)? = null

  private class PendingShare(val text: String, val uris: List<Uri>, val mimeType: String?)

  /** Record a share intent. Safe to call with any intent, including null. */
  fun capture(intent: Intent?) {
    val action = intent?.action ?: return
    val opening = action == Intent.ACTION_VIEW
    if (!opening && action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return
    // Open In carries its document in data, not EXTRA_STREAM. Only accept
    // provider-granted content URIs; never treat deep links or private paths as files.
    val uris = if (opening) listOfNotNull(intent.data?.takeIf { it.scheme == "content" })
      else streamUris(intent)
    val text = if (opening) "" else intent.getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString().orEmpty()
    if (text.isBlank() && uris.isEmpty()) return
    synchronized(lock) {
      pending = PendingShare(text, uris, intent.type)
    }
    onPending?.invoke()
  }

  /**
   * Take the pending payload, copying its files into app-owned cache storage.
   *
   * The copy happens here rather than at capture time so the (possibly large)
   * read never blocks the activity's lifecycle callback, and it happens at all
   * because the content URIs are only readable through the grant carried by the
   * receiving intent — a path that can be handed to JavaScript cannot assume it.
   */
  fun take(context: Context): SharedContent? {
    val share = synchronized(lock) {
      val current = pending
      pending = null
      current
    } ?: return null

    prune(context)
    var tooLarge = share.uris.size > ShareFileCopier.MAX_FILES
    var failed = false
    val budget = ShareFileCopier.Budget()
    val files = mutableListOf<SharedFile>()
    share.uris.take(ShareFileCopier.MAX_FILES).forEachIndexed { index, uri ->
      var target: File? = null
      var retained = false
      try {
        budget.beginFile()
        val name = displayName(context, uri) ?: "shared-${index + 1}${extensionFor(share.mimeType)}"
        target = File(shareDirectory(context), "${UUID.randomUUID()}-$name")
        val input = context.contentResolver.openInputStream(uri)
          ?: throw java.io.IOException("Document provider returned no stream")
        ShareFileCopier.copy(input, target, budget)
        files.add(
          SharedFile(
            uri = Uri.fromFile(target).toString(),
            name = name,
            mimeType = context.contentResolver.getType(uri)?.takeIf { !it.contains('*') }
              ?: share.mimeType?.takeIf { it.isNotBlank() && !it.contains('*') }
              ?: "application/octet-stream",
          )
        )
        retained = true
      } catch (_: ShareFileCopier.LimitExceededException) {
        tooLarge = true
      } catch (_: Exception) {
        // A single unreadable item must not discard the rest of the share.
        failed = true
      } finally {
        // Also covers metadata/URI failures after copying has succeeded.
        if (!retained) target?.delete()
      }
    }
    if (share.text.isBlank() && files.isEmpty() && !tooLarge && !failed) return null
    return SharedContent(text = share.text, files = files, tooLarge = tooLarge, failed = failed)
  }

  /** Whether a share is waiting, without consuming it (used by tests). */
  fun hasPending(): Boolean = synchronized(lock) { pending != null }

  @Suppress("DEPRECATION")
  private fun streamUris(intent: Intent): List<Uri> {
    // Some senders put a list on ACTION_SEND, so both shapes are accepted
    // regardless of the declared action.
    val stream = intent.extras?.get(Intent.EXTRA_STREAM)
    val uris = when (stream) {
      is Uri -> listOf(stream)
      is ArrayList<*> -> stream.filterIsInstance<Uri>()
      else -> emptyList()
    }
    // Gallery/file apps may grant their attachments through ClipData only.
    val clip = intent.clipData
    val clipped = if (clip == null) emptyList() else
      (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
    return (uris + clipped).distinct()
  }

  private fun displayName(context: Context, uri: Uri): String? {
    if (uri.scheme == "file") return uri.lastPathSegment?.substringAfterLast('/')
    val name = try {
      context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { cursor -> if (cursor.moveToFirst()) cursor.getString(0) else null }
    } catch (_: Exception) {
      null
    }
    return name?.substringAfterLast('/')?.substringAfterLast('\\')?.takeIf { it.isNotBlank() }
  }

  private fun extensionFor(mimeType: String?): String = when {
    mimeType == null -> ""
    mimeType == "image/jpeg" -> ".jpg"
    mimeType == "image/png" -> ".png"
    mimeType == "image/gif" -> ".gif"
    mimeType == "image/webp" -> ".webp"
    mimeType.startsWith("image/") -> ".${mimeType.removePrefix("image/")}"
    else -> ""
  }

  private fun shareDirectory(context: Context): File =
    File(context.cacheDir, "share").apply { mkdirs() }

  /** Reclaim copies from previous shares; the cache dir is otherwise unbounded. */
  private fun prune(context: Context) {
    val cutoff = System.currentTimeMillis() - RETAIN_MILLIS
    shareDirectory(context).listFiles()?.forEach { file ->
      if (file.isFile && file.lastModified() < cutoff) file.delete()
    }
  }
}
