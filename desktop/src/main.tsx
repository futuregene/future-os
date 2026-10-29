import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
// Keeps a wheel gesture inside the scroll container it started in.
import { installScrollGestureLatch } from "./lib/scrollGestureLatch";
import "./i18n";
// Applies the persisted light/dark choice before the first React paint.
import "./lib/theme";
// Applies the persisted text size before the first React paint.
import "./lib/fontSize";
import "./styles/globals.css";

installScrollGestureLatch();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
