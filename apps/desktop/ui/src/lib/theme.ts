// Themes: the five palettes from the brand kit (docs/design/theme.md). The
// colours live in styles/tokens.css under <html data-theme="...">; this file
// only picks one and remembers it (localStorage, like a cookie that never
// leaves the machine).
//
//   import { theme, setTheme, THEMES } from "../lib/theme";
//   setTheme("light");        // switch now and on every later launch
//   theme();                  // the current id (a Solid signal, reactive)

import { createSignal } from "solid-js";

export type ThemeId = "cosmic-blue" | "aurora" | "ocean" | "midnight" | "light";

export type ThemeInfo = {
  id: ThemeId;
  /** Swatches for a picker: background, surface, primary, accent. */
  swatch: [string, string, string, string];
  /** Browser/OS chrome colour (the <meta name="theme-color">). */
  chrome: string;
};

/** In the kit's order. Cosmic Blue is the brand default. */
export const THEMES: ThemeInfo[] = [
  { id: "cosmic-blue", swatch: ["#050b18", "#0b1630", "#1455d9", "#22c7f2"], chrome: "#050b18" },
  { id: "aurora", swatch: ["#090817", "#17152e", "#713bdb", "#42d9f5"], chrome: "#090817" },
  { id: "ocean", swatch: ["#061321", "#0d2439", "#0759a6", "#00b8d9"], chrome: "#061321" },
  { id: "midnight", swatch: ["#080b12", "#111827", "#2563eb", "#38bdf8"], chrome: "#080b12" },
  { id: "light", swatch: ["#f4f8ff", "#ffffff", "#1769e8", "#38bdf8"], chrome: "#f4f8ff" },
];

export const DEFAULT_THEME: ThemeId = "cosmic-blue";
const STORAGE_KEY = "md.theme";

function isTheme(v: unknown): v is ThemeId {
  return THEMES.some((t) => t.id === v);
}

function saved(): ThemeId {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return isTheme(v) ? v : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

const [current, setCurrent] = createSignal<ThemeId>(saved());

/** The active theme id. */
export const theme = current;

function apply(id: ThemeId) {
  const root = document.documentElement;
  root.dataset.theme = id;
  let meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (!meta) {
    meta = document.createElement("meta");
    meta.name = "theme-color";
    document.head.appendChild(meta);
  }
  meta.content = THEMES.find((t) => t.id === id)!.chrome;
}

/** Switch theme now and remember it for the next launch. */
export function setTheme(id: ThemeId) {
  if (!isTheme(id)) return;
  setCurrent(id);
  apply(id);
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    // Storage unavailable: the theme still applies for this session.
  }
}

/** Apply the saved theme. Call once before the first render (index.tsx). */
export function initTheme() {
  apply(current());
}
