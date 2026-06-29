/* @refresh reload */
import { render } from "solid-js/web";

import "@fontsource/silkscreen";
import "@fontsource/silkscreen/700.css";
import "@fontsource-variable/pixelify-sans";
import "@fontsource-variable/jetbrains-mono";
// Selectable fonts (Settings → Fonts). Bundled so they work regardless of the
// system's installed fonts; the registry in state/fonts.ts maps ids → families.
import "@fontsource-variable/inter";
import "@fontsource-variable/fira-code";
import "@fontsource-variable/cascadia-code";
import "@fontsource/ibm-plex-mono";
import "@fontsource/ibm-plex-mono/700.css";
import "./state/theme";
import "./theme.css";
import "./app.css";

import { App } from "./App";
import "./client";
import { settings } from "./state/settings";
import { fontById } from "./state/fonts";

// Warm the saved terminal font at boot so its woff2 is downloading well before
// the first terminal mounts. Without this, a cold-cache first open measures cell
// width against a fallback font and renders glyphs with broken spacing until the
// font lands (see terminal.ts attach()). The UI and diff fonts are warmed by
// applyFonts() in state/settings.ts. System fonts (primary null) need no warm.
const termPrimary = fontById(settings.terminalFont)?.primary;
if (termPrimary) document.fonts?.load(`15px "${termPrimary}"`).catch(() => {});

render(() => <App />, document.getElementById("root")!);
