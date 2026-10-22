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
 *                          (`shot/prefs.ts` pins these before the app is
 *                          imported — see `desktop/shot/index.html`)
 *   ?theme=dark&palette=nord  pin the appearance and the color theme (both are
 *                          preferences read before the first paint, so a capture
 *                          of "Gruvbox in a light window" is one URL rather than
 *                          a click sequence)
 *   ?settings=key:value    seed the fake backend's app settings before boot
 *   ?streaming=id,id       report these threads as running (rail's ring)
 *   ?unread=id,id          mark these threads' finished runs unread (rail's dot)
 *   ?pinned=id,id          pin these threads for this capture (the Vibe panel's
 *                          membership — the fixture ships one pinned row)
 *   ?remoteSlowRead=1      hold a host's reads back, so the state a reader is in
 *                          while one is in flight reaches a capture (the model
 *                          control before the host answers)
 *
 * `window.__shot` carries the states a query parameter cannot express, because
 * they only exist *after* a screen is mounted: pushing a pending approval onto
 * the open remote conversation (and onto this machine's own), one live session
 * event for a row already in the list, and the rest of a run the reader opened
 * part-way through.
 */
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "../src/app/App";
import { patchSettings, seedHarnessState } from "./mock/backend";
import { remoteApproval } from "./mock/data";
import { emit } from "./mock/event";
import { startMidFlightRun } from "./mock/remoteStream";
import "../src/i18n";
import "../src/styles/globals.css";

const params = new URLSearchParams(location.search);

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
  update: params.get("update"),
  pinned: list(params.get("pinned")),
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
 *
 * The run a host was already part-way through is the same shape of state: its
 * beginning is the page the client reads when it opens the conversation, and its
 * rest is pushed — so the step that continues it has to come after that open.
 */
(window as unknown as { __shot: unknown }).__shot = {
  /**
   * Continue the run `rs_forest`'s transcript is already part-way through.
   *
   * The conversation has to be open already: this is the push lane, and a reply
   * pushed before the client subscribed would be exactly what the fixture is not
   * testing. Call it after the conversation's own read has landed.
   */
  remoteMidFlight: () => startMidFlightRun("rs_forest"),
  /**
   * One live event for a conversation the host's catalogue already holds.
   *
   * A row's *place* in the merged list is the host's to state — its catalogue
   * carries the host's own `last_message_at`, and only the host's `state.activity`
   * packet moves a row — so an event of its own must not reorder the list (see
   * `useRemotePeers`). `?remoteStream=1` only pushes into the conversation a
   * scenario opens, whose row is already the newest of its group; this emits one
   * for a row further down, which is where a client that ranked on its own clock
   * would be visible.
   */
  remoteSessionEvent: (sessionId: string, desktopId = "desktop_studio") => emit("remote-peer-event", {
    desktopId,
    kind: "event",
    payload: { sessionId, type: "text_chunk", data: JSON.stringify({ text: "…" }) },
  }),
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
