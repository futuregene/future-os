/**
 * Screenshot harness entry point for the desktop app.
 *
 * Mounts the real application (`src/app/App`) with the Tauri boundary mocked.
 * Started by `scripts/screenshots/capture.py serve-desktop`; see
 * `docs/guide/screenshots.zh-CN.md` (or the .md next to it).
 *
 * URL parameters drive states a plain page load cannot reach:
 *   ?press=meta+f          dispatch this shortcut at the real window listener
 *                          (the browser tool's key table has no letter keys)
 *   ?lang=en               render the English UI instead of Chinese
 *   ?settings=key:value    seed the fake backend's app settings before boot
 *   ?streaming=id,id       report these threads as running (rail's ring)
 *   ?unread=id,id          mark these threads' finished runs unread (rail's dot)
 *
 * `window.__shot` carries the two states a query parameter cannot express,
 * because they only exist *after* a screen is mounted: pushing a pending
 * approval onto the open remote conversation, and onto this machine's own.
 */
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "../src/app/App";
import { patchSettings, seedHarnessState } from "./mock/backend";
import { remoteApproval } from "./mock/data";
import { emit } from "./mock/event";
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

// Rail states: `?streaming=th_longformer&unread=th_data,th_fig`. Comma-separated
// lists, empty entries dropped, so a scenario can name one state without the
// other.
const list = (value: string | null): string[] => (value ?? "").split(",").filter(Boolean);
seedHarnessState({
  streaming: list(params.get("streaming")),
  unread: list(params.get("unread")),
});

/**
 * States that only exist once a screen is mounted.
 *
 * A pending approval is *pushed* to the client, not read from it: a local one
 * arrives through the store, a remote one through the peer event stream. Both
 * are therefore triggered by a scenario step rather than a URL parameter, so the
 * capture controls when the card appears instead of racing a timer.
 */
(window as unknown as { __shot: unknown }).__shot = {
  /** A pending approval on the host, for the open remote conversation. */
  remoteApproval: () => emit("remote-peer-event", {
    desktopId: "desktop_studio",
    kind: "event",
    payload: {
      sessionId: remoteApproval.sessionId,
      type: "approval_request",
      data: JSON.stringify({
        approval_request_id: remoteApproval.id,
        tool_name: remoteApproval.toolName,
        title: remoteApproval.title,
        summary: remoteApproval.summary,
        risk_level: remoteApproval.riskLevel,
      }),
    },
  }),
};

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
