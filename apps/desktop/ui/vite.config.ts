import { fileURLToPath } from "node:url";
import { defineConfig, searchForWorkspaceRoot } from "vite";
import solid from "vite-plugin-solid";

// The Help topics live in the repo's docs/help (shared with the Android
// app) and are bundled with import.meta.glob (src/help/topics.ts). The dev
// server only serves files under the UI's folder unless told otherwise.
const helpDocs = fileURLToPath(new URL("../../../docs/help", import.meta.url));

// Tauri serves this in dev (devUrl) and bundles dist/ in release.
export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    fs: { allow: [searchForWorkspaceRoot(process.cwd()), helpDocs] },
  },
  // One bundle loaded from disk by the app, not over a network: the
  // 500 kB web-page warning doesn't apply (the Hindi strings took it past).
  build: { target: "es2021", outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 1024 },
});
