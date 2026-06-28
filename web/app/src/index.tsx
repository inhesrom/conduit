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

// Warm the terminal font at boot so its woff2 is downloading well before the
// first terminal mounts. Without this, a cold-cache first open measures cell
// width against a fallback font and renders glyphs with broken spacing until
// the font lands (see terminal.ts attach()).
document.fonts?.load('15px "JetBrains Mono Variable"').catch(() => {});

render(() => <App />, document.getElementById("root")!);
