import { defineConfig } from "vite";

// Tauri expects a fixed dev port; disable HMR websocket upgrades that
// conflict with the Tauri dev server protocol.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5180,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
});
