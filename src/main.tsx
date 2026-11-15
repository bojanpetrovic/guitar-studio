import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import "@xyflow/react/dist/style.css";
import "./styles.css";
import App from "./App";
import { initTheme } from "./themes";
import { IS_PLUGIN } from "./transport";
import { IS_WINDOWS } from "./platform";

initTheme();
// Inside a DAW: no traffic lights to make room for, no window to make see-through.
if (IS_PLUGIN) document.documentElement.classList.add("plugin");
// Windows: a native title bar above the window, no traffic lights inside it.
else if (IS_WINDOWS) document.documentElement.classList.add("windows");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
