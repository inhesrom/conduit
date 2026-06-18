/* @refresh reload */
import { render } from "solid-js/web";

import "@fontsource/silkscreen";
import "@fontsource/silkscreen/700.css";
import "@fontsource-variable/pixelify-sans";
import "@fontsource-variable/jetbrains-mono";
import "./state/theme";
import "./theme.css";
import "./app.css";

import { App } from "./App";
import "./client";

render(() => <App />, document.getElementById("root")!);
