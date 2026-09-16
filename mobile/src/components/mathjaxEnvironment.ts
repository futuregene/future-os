// MathJax's environment probe runs before the lite (non-DOM) adaptor is
// registered. React Native exposes window.navigator but, unlike a browser,
// omits these metadata strings. Supply only missing strings; never create a
// document, spoof a browser, or replace existing host values.
const navigator = globalThis.navigator;
if (navigator) {
  for (const key of ["appVersion", "userAgent"] as const) {
    if (typeof navigator[key] !== "string") {
      Object.defineProperty(navigator, key, { value: "ReactNative", configurable: true });
    }
  }
}

export {};
