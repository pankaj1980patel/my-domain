import { render } from "solid-js/web";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource-variable/inter"; // Inter, bundled (OFL: public/licenses/Inter-OFL.txt)
import "./styles/tokens.css";
import "./styles/app.css";
import App from "./App";
import QuickPanel from "./screens/QuickPanel";
import { followThemeChanges, initTheme } from "./lib/theme";

initTheme();

// The same bundle draws the menu-bar icon's quick panel in its own window
// (quick_panel.rs), told apart by the window's label.
const panel = getCurrentWindow().label === "quick-panel";
if (panel) {
  document.documentElement.classList.add("qp-window");
  followThemeChanges();
}

render(() => (panel ? <QuickPanel /> : <App />), document.getElementById("root")!);
