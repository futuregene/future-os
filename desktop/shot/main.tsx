/**
 * Screenshot harness entry point for the desktop app.
 *
 * Mounts the real application (`src/app/App`) with the Tauri boundary mocked.
 * Started by `make screenshots-desktop`; see `docs/guide/screenshots.md`.
 *
 * URL parameters drive states a plain page load cannot reach:
 *   ?press=meta+f          dispatch this shortcut at the real window listener
 *                          (the browser tool's key table has no letter keys)
 *   ?lang=en               render the English UI instead of Chinese
 *   ?settings=key:value    seed the fake backend's app settings before boot
 */
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "../src/app/App";
import { patchSettings } from "./mock/backend";
import "../src/i18n";
import "../src/styles/globals.css";

const params = new URLSearchParams(location.search);

// The UI language is a user preference in the real app; pin it per capture.
localStorage.setItem("future.language", params.get("lang") === "en" ? "en" : "zh");

// `?settings=communityEdition:true,bellOnComplete:false`
const settingsPatch = params.get("settings");
if (settingsPatch) {
  const patch: Record<string, unknown> = {};
  for (const pair of settingsPatch.split(",")) {
    const [key, value] = pair.split(":");
    if (key && value)
      patch[key] = value === "true" ? true : value === "false" ? false : value;
  }
  patchSettings(patch);
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// Wait for the initial data load before firing shortcuts at the app.
for (const spec of params.getAll("press")) {
  setTimeout(() => {
    const parts = spec.split("+");
    const key = parts.pop() ?? "";
    const modifiers = parts.map(part => part.toLowerCase());
    window.dispatchEvent(new KeyboardEvent("keydown", {
      key,
      bubbles: true,
      cancelable: true,
      ctrlKey: modifiers.includes("ctrl") || modifiers.includes("control"),
      metaKey: modifiers.includes("meta") || modifiers.includes("cmd"),
      shiftKey: modifiers.includes("shift"),
      altKey: modifiers.includes("alt"),
    }));
  }, 2500);
}
