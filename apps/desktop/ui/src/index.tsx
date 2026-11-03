import { render } from "solid-js/web";
import "@fontsource-variable/inter"; // Inter, bundled (OFL: public/licenses/Inter-OFL.txt)
import "./styles/tokens.css";
import "./styles/app.css";
import App from "./App";
import { initTheme } from "./lib/theme";

initTheme();

render(() => <App />, document.getElementById("root")!);
