import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed dev port; disable HMR websocket upgrades that
// conflict with the Tauri dev server protocol.
export default defineConfig({
  clearScreen: false,
  plugins: [react(), tailwindcss()],
  server: {
    port: 5180,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
});
