import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // host: the phone remote loads the UI from here during `tauri dev` (see src-tauri/src/remote.rs).
  server: { port: 1420, strictPort: true, host: true, watch: { ignored: ["**/src-tauri/**", "**/target/**"] } },
  build: { target: "safari16" },
});
