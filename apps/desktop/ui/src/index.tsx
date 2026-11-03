import { render } from "solid-js/web";
import "./styles/tokens.css";
import "./styles/app.css";
import App from "./App";
import { initTheme } from "./lib/theme";

initTheme();

render(() => <App />, document.getElementById("root")!);
