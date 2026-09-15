package cn.future_os.shareintent

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Intent
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowContentResolver

@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE, sdk = [35])
class ShareIntentStoreTest {
  private val context get() = RuntimeEnvironment.getApplication()

  @Before fun reset() {
    ShareIntentStore.take(context)
    val provider = DocumentProvider()
    provider.attachInfo(context, android.content.pm.ProviderInfo().apply { authority = "open-test" })
    ShadowContentResolver.registerProviderInternal("open-test", provider)
  }

  @After fun cleanup() {
    ShareIntentStore.onPending = null
    ShareIntentStore.take(context)
    File(context.cacheDir, "share").deleteRecursively()
    File(context.cacheDir, "provider-source").delete()
  }

  @Test fun openInCopiesDataUriWithMetadataAndConsumesItOnce() {
    for (name in listOf("报告.doc", "报告.docx", "报告.pdf", "报告.pptx", "笔记.md", "图.jpeg", "图.png")) {
      val uri = Uri.parse("content://open-test/$name")
      var notified = false
      ShareIntentStore.onPending = { notified = ShareIntentStore.hasPending() }
      ShareIntentStore.capture(Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/octet-stream"))
      assertTrue(notified)
      val received = ShareIntentStore.take(context)!!
      assertEquals("", received.text)
      assertFalse(received.tooLarge)
      val file = received.files.single()
      assertEquals(name, file.name)
      assertEquals("application/pdf", file.mimeType) // Resolver overrides the sender's fallback MIME.
      assertEquals("document bytes", File(Uri.parse(file.uri).path!!).readText())
      assertNull(ShareIntentStore.take(context))
    }
  }

  @Test fun unrelatedViewIntentsCannotReplaceAPendingDocument() {
    ShareIntentStore.capture(Intent(Intent.ACTION_VIEW, Uri.parse("content://open-test/report.pdf")))
    for (uri in listOf("https://example.com/a.pdf", "futureos://pair/test", "file:///private/secret.pdf")) {
      ShareIntentStore.capture(Intent(Intent.ACTION_VIEW, Uri.parse(uri)))
    }
    ShareIntentStore.capture(Intent(Intent.ACTION_MAIN))
    ShareIntentStore.capture(Intent(Intent.ACTION_VIEW))
    assertEquals("report.pdf", ShareIntentStore.take(context)!!.files.single().name)
  }

  @Test fun openInDoesNotInterpretTextOrExtraStreamsAsItsDocument() {
    ShareIntentStore.capture(Intent(Intent.ACTION_VIEW)
      .putExtra(Intent.EXTRA_TEXT, "not a file")
      .putExtra(Intent.EXTRA_STREAM, Uri.parse("content://open-test/wrong.pdf")))
    assertFalse(ShareIntentStore.hasPending())
  }

  @Test fun unreadableDocumentIsReportedInsteadOfSilentlyIgnored() {
    ShareIntentStore.capture(Intent(Intent.ACTION_VIEW, Uri.parse("content://open-test/unreadable.pdf")))
    val received = ShareIntentStore.take(context)!!
    assertTrue(received.failed)
    assertTrue(received.files.isEmpty())
    assertFalse(received.tooLarge)
  }

  @Test fun existingSendAndSendMultipleStillWork() {
    ShareIntentStore.capture(Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_TEXT, "shared text"))
    assertEquals("shared text", ShareIntentStore.take(context)!!.text)
    ShareIntentStore.capture(Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(
      Intent.EXTRA_STREAM, arrayListOf(Uri.parse("content://open-test/a.pdf"), Uri.parse("content://open-test/b.pdf"))))
    assertEquals(listOf("a.pdf", "b.pdf"), ShareIntentStore.take(context)!!.files.map { it.name })
  }

  class DocumentProvider : ContentProvider() {
    override fun onCreate() = true
    override fun getType(uri: Uri) = "application/pdf"
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?,
                       selectionArgs: Array<out String>?, sortOrder: String?) =
      MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME)).apply { addRow(arrayOf(uri.lastPathSegment)) }
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
      if (uri.lastPathSegment == "unreadable.pdf") throw java.io.FileNotFoundException()
      val file = File(RuntimeEnvironment.getApplication().cacheDir, "provider-source")
      file.writeText("document bytes")
      return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?) = 0
  }
}
