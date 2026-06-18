/* @refresh reload */
import { render } from "solid-js/web";

import "@fontsource-variable/mona-sans";
import "@fontsource-variable/jetbrains-mono";
import "./theme.css";
import "./app.css";

import { App } from "./App";
import "./client";

render(() => <App />, document.getElementById("root")!);
