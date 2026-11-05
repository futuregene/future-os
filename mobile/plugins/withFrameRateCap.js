const { withMainActivity } = require("@expo/config-plugins");

// `android/` is a prebuild artifact (.gitignore), so the activity cannot be
// edited directly — the call is injected into the generated `MainActivity.kt`.
//
// Why 60 Hz: the chat UI's own content changes at most ~12 times a second (the
// sync engine coalesces live text at 80 ms), so a 120 Hz panel buys nothing
// here while it doubles composition and render work whenever the list scrolls
// or an animation runs. `View#setRequestedFrameRate` (API 35+) is a hint, not a
// cap — a window voting for a higher rate still wins — so it is worth keeping
// only if the panel actually follows:
//
//   adb shell dumpsys SurfaceFlinger | grep requestedFrameRate
//   adb shell dumpsys gfxinfo cn.futureos.mobile | grep "Total frames rendered"
//
// Reinstalling is not enough: an already-running process keeps its vote, so
// `am force-stop cn.futureos.mobile` (or a reboot) before measuring.
const MARKER = "capDisplayFrameRate";
const ANCHOR = "    super.onCreate(null)\n";
const CALL = "    capDisplayFrameRate(this)\n";
const HELPER = `
// Injected by mobile/plugins/withFrameRateCap.js — edit that plugin, not this
// generated file. Asked for in onCreate and posted so the view is attached when
// the vote is cast (an unattached view queues it until attach).
private fun capDisplayFrameRate(activity: android.app.Activity) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return
    val decor = activity.window.decorView
    decor.post { decor.setRequestedFrameRate(60f) }
}
`;

module.exports = function withFrameRateCap(config) {
  return withMainActivity(config, (cfg) => {
    if (cfg.modResults.language !== "kt") {
      throw new Error("withFrameRateCap expects the Kotlin MainActivity");
    }
    const contents = cfg.modResults.contents;
    if (contents.includes(MARKER)) return cfg;
    if (!contents.includes(ANCHOR)) {
      throw new Error(
        "withFrameRateCap: MainActivity no longer contains \"super.onCreate(null)\"; re-anchor the injection",
      );
    }
    cfg.modResults.contents =
      contents.replace(ANCHOR, ANCHOR + CALL).trimEnd() + "\n" + HELPER;
    return cfg;
  });
};
