import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri loads the dev server at a fixed port (tauri.conf.json devUrl) and
// prints its own output, so Vite keeps the port and leaves the screen alone.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5174,
    strictPort: true,
    watch: {
      // The Rust side rebuilds itself; watching it only wakes Vite for nothing.
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
  },
});
