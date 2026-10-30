/**
 * The language the scripted data is written in: the one the app is drawn in.
 *
 * A capture pins the interface language with `--lang` (a browser locale
 * override, so `navigator.language` is what expo-localization reads), and every
 * fixture reads the same signal — so a frame cannot end up with an English
 * chrome over a Chinese transcript, and a fixture's own labels (a machine's
 * name, a session title) are in the frame's language too. The TUI does the same
 * thing through `FUTURE_DEMO_LANG`, which its capture passes alongside the
 * interface pin.
 */
export const shotLanguage: "zh" | "en" = typeof navigator !== "undefined" &&
  (navigator.languages?.[0] ?? navigator.language ?? "").toLowerCase().startsWith("zh")
  ? "zh"
  : "en";
