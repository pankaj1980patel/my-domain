import { render } from "solid-js/web";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "@fontsource-variable/inter"; // Inter, bundled (OFL: public/licenses/Inter-OFL.txt)
import "./styles/tokens.css";
import "./styles/app.css";
import App from "./App";
import QuickPanel from "./screens/QuickPanel";
import KvmOverlay from "./screens/KvmOverlay";
import { followThemeChanges, initTheme } from "./lib/theme";

initTheme();

// The same bundle draws the menu-bar icon's quick panel in its own window
// (quick_panel.rs), told apart by the window's label.
const label = getCurrentWindow().label;
const panel = label === "quick-panel";
// ... and the bar shown while another computer controls this one (kvm.rs).
const kvmBar = label === "kvm-overlay";
if (kvmBar) {
  document.documentElement.classList.add("kvm-window");
  followThemeChanges();
}
if (panel) {
  document.documentElement.classList.add("qp-window");
  followThemeChanges();
}

render(() => (kvmBar ? <KvmOverlay /> : panel ? <QuickPanel /> : <App />), document.getElementById("root")!);
