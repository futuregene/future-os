package cn.future_os.shareintent

import android.content.Context
import expo.modules.core.interfaces.Package
import expo.modules.core.interfaces.ReactActivityLifecycleListener

/**
 * Registered automatically: the Expo autolinking plugin scans local modules for
 * classes importing `expo.modules.core.interfaces.Package`, so no generated
 * package list has to be edited.
 */
class ShareIntentPackage : Package {
  override fun createReactActivityLifecycleListeners(
    activityContext: Context?
  ): List<ReactActivityLifecycleListener> = listOf(ShareIntentLifecycleListener())
}
