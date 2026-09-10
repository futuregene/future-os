package cn.future_os.shareintent

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import expo.modules.core.interfaces.ReactActivityLifecycleListener

/**
 * Captures share intents for as long as the activity lives.
 *
 * `onCreate` covers the cold start (the share intent launches the app);
 * `onNewIntent` covers a warm start, where `MainActivity` is `singleTask` and
 * `Activity.getIntent()` keeps returning the original intent.
 */
class ShareIntentLifecycleListener : ReactActivityLifecycleListener {
  override fun onCreate(activity: Activity, savedInstanceState: Bundle?) {
    ShareIntentStore.capture(activity.intent)
  }

  override fun onNewIntent(intent: Intent?): Boolean {
    ShareIntentStore.capture(intent)
    // Never claim the intent: React Native's own deep-link handling still needs it.
    return false
  }
}
