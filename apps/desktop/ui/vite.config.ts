import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Tauri serves this in dev (devUrl) and bundles dist/ in release.
export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2021", outDir: "dist", emptyOutDir: true },
});
