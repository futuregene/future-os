package cn.future_os.nativeui

import android.app.AlertDialog
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
import expo.modules.kotlin.Promise
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class NativeUiModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("FutureNativeUi")

    Function("isPhotoPickerAvailable") {
      val context = appContext.reactContext ?: error("React context is unavailable")
      PickVisualMedia.isPhotoPickerAvailable(context)
    }

    AsyncFunction("showActionSheet") { title: String?, options: List<String>, promise: Promise ->
      require(options.isNotEmpty()) { "Action sheet options cannot be empty" }
      val activity = appContext.currentActivity ?: error("Current activity is unavailable")
      activity.runOnUiThread {
        var selectedIndex: Int? = null
        var settled = false
        fun resolveAfterDismiss() {
          if (settled) return
          settled = true
          promise.resolve(selectedIndex)
        }

        val builder = AlertDialog.Builder(activity)
        if (!title.isNullOrBlank()) builder.setTitle(title)
        val dialog = builder
          .setItems(options.toTypedArray()) { _, index -> selectedIndex = index }
          .create()
        dialog.setOnDismissListener { resolveAfterDismiss() }
        dialog.show()
      }
    }
  }
}
