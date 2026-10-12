import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import { windowRoute } from "./app/windowRoute";
import { PeerSettingsWindow } from "./features/remote-peer/PeerSettingsWindow";
import "./i18n";
import "./styles/globals.css";

// The same bundle serves both windows; the URL says which one this is (see
// `windowRoute`). A second HTML entry point would mean a second copy of the
// app's providers and styles to keep in step.
const route = windowRoute(window.location.search);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {route.kind === "peer-settings"
      ? <PeerSettingsWindow desktopId={route.desktopId} />
      : <App />}
  </React.StrictMode>,
);
