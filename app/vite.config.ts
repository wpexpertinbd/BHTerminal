import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri drives this dev server; the port is pinned so tauri.conf.json's
// devUrl can match it.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5275, strictPort: true },
  build: { target: "es2022", emptyOutDir: true },
});
