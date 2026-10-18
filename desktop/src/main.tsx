import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import "./i18n";
// Applies the persisted light/dark choice before the first React paint.
import "./lib/theme";
import "./styles/globals.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
