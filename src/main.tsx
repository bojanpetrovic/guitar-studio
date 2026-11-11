import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "@xyflow/react/dist/style.css";
import "./styles.css";
import App from "./App";
import { initTheme } from "./themes";
import { IS_PLUGIN } from "./transport";

initTheme();
// Inside a DAW: no traffic lights to make room for, no window to make see-through.
if (IS_PLUGIN) document.documentElement.classList.add("plugin");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
