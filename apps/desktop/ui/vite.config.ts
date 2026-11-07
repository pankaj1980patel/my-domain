import { fileURLToPath } from "node:url";
import { defineConfig, searchForWorkspaceRoot, type Plugin } from "vite";
import solid from "vite-plugin-solid";

// The Help topics live in the repo's docs/help (shared with the Android
// app) and are bundled with import.meta.glob (src/help/topics.ts). The dev
// server only serves files under the UI's folder unless told otherwise.
const helpDocs = fileURLToPath(new URL("../../../docs/help", import.meta.url));

// Screenshot mode (`vite build --mode mock`, `vite --mode mock`): the page
// starts at src/dev/mockEntry.ts, which answers the app's commands with
// sample data (src/dev/mockTauri.ts) when the URL has ?mock=1. Only this
// mode swaps the entry, so the normal build (the one Tauri bundles) never
// contains the mock or its samples. scripts/screenshots-desktop.sh uses it.
const mockEntry: Plugin = {
  name: "md-mock-entry",
  // "pre": before Vite reads the page's entry script from it.
  transformIndexHtml: { order: "pre", handler: (html) => html.replace("/src/index.tsx", "/src/dev/mockEntry.ts") },
};

// Tauri serves this in dev (devUrl) and bundles dist/ in release.
export default defineConfig(({ mode }) => ({
  plugins: mode === "mock" ? [solid(), mockEntry] : [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    fs: { allow: [searchForWorkspaceRoot(process.cwd()), helpDocs] },
  },
  // One bundle loaded from disk by the app, not over a network: the
  // 500 kB web-page warning doesn't apply (the Hindi strings took it past).
  build: {
    target: "es2021",
    outDir: mode === "mock" ? "dist-mock" : "dist",
    emptyOutDir: true,
    chunkSizeWarningLimit: 1024,
  },
}));
