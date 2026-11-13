import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// During development the API runs separately (cargo run -- serve); proxy to it.
const api = process.env.GANTRY_API ?? "http://127.0.0.1:8600";

export default defineConfig({
  plugins: [react()],
  server: { port: 5173, proxy: { "/api": api, "/mcp": api, "/manual": api } },
  build: { outDir: "dist", sourcemap: false, target: ["chrome111", "edge111", "safari16.4"], chunkSizeWarningLimit: 1500 },
  test: { environment: "jsdom", setupFiles: ["src/test-setup.ts"], include: ["src/**/*.test.{ts,tsx}"], css: false },
});
