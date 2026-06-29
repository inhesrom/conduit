import { createStore } from "solid-js/store";
import { fontById, fontCss } from "./fonts";

/** Client-side settings. Agent command resolution lives in the client (not the
 * daemon), exactly like the TUI: the daemon just runs whatever argv the client
 * sends in StartTerminal. Persisted per-browser in localStorage. */
export interface AgentProfile {
  name: string;
  command: string;
  yoloFlags: string[];
  continueFlags: string[];
}

export interface Settings {
  agents: AgentProfile[];
  defaultAgent: string;
  /** When on, agent launches include the profile's yolo flags. */
  yoloMode: boolean;
  attentionNotifications: boolean;
  /** xterm font size (px) for terminal panes. */
  termFontSize: number;
  /** Multiplier for all UI chrome font sizes, pushed to the --ui-scale CSS var. */
  uiScale: number;
  /** Rounded corners (default). Off sets data-corners="square" for hard edges. */
  roundedCorners: boolean;
  /** Git UI placement: a collapsible right sidebar (default) or the original
   * terminal-over-git bottom split. */
  gitLayout: "sidebar" | "bottom";
  /** Font ids (see state/fonts.ts). uiFont/diffFont drive the --font-ui /
   * --font-diff CSS vars; terminalFont is applied to xterm's fontFamily. */
  uiFont: string;
  terminalFont: string;
  diffFont: string;
}

// Mirrors the TUI defaults (crates/tui/src/app.rs).
const DEFAULTS: Settings = {
  agents: [
    { name: "claude", command: "claude", yoloFlags: ["--dangerously-skip-permissions"], continueFlags: ["-c"] },
    { name: "codex", command: "codex", yoloFlags: ["--full-auto"], continueFlags: [] },
  ],
  defaultAgent: "claude",
  yoloMode: false,
  attentionNotifications: true,
  termFontSize: 13,
  uiScale: 0.85,
  roundedCorners: true,
  gitLayout: "sidebar",
  uiFont: "pixelify-sans",
  terminalFont: "jetbrains-mono",
  diffFont: "jetbrains-mono",
};

const STORAGE_KEY = "conduit.settings";

function loadInitial(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Settings>) };
  } catch {
    // fall through to defaults
  }
  return structuredClone(DEFAULTS);
}

export const [settings, setSettings] = createStore<Settings>(loadInitial());

/** Push the UI chrome scale into the CSS var that app.css/theme.css multiply
 * their font sizes by. Applied on load (mirrors theme.ts's apply pattern) so
 * there's no flash, then re-applied whenever the setting changes. */
function applyUiScale(scale: number): void {
  document.documentElement.style.setProperty("--ui-scale", String(scale));
}
applyUiScale(settings.uiScale);

/** Rounded corners are the default (theme.css :root). Opting out sets
 * data-corners="square", which restores --radius:0 and the hard offset shadows.
 * Applied on load (no flash), then re-applied on change. */
function applyRounded(on: boolean): void {
  if (on) document.documentElement.removeAttribute("data-corners");
  else document.documentElement.setAttribute("data-corners", "square");
}
applyRounded(settings.roundedCorners);

/** Warm a font's woff2 so it's downloaded before anything measures against it.
 * No-op for system stacks (primary === null) and where the API is unavailable. */
function warmFont(id: string): void {
  const primary = fontById(id)?.primary;
  if (primary) document.fonts?.load(`16px "${primary}"`).catch(() => {});
}

/** Push the chosen UI and diff fonts into the CSS vars app.css/theme.css read.
 * The terminal font is applied separately via xterm's fontFamily (TerminalView).
 * Applied on load (no flash), then re-applied whenever a font setting changes. */
function applyFonts(): void {
  const root = document.documentElement.style;
  root.setProperty("--font-ui", fontCss(settings.uiFont));
  root.setProperty("--font-diff", fontCss(settings.diffFont));
  warmFont(settings.uiFont);
  warmFont(settings.diffFont);
}
applyFonts();

export function persistSettings(): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // ignore quota/availability errors — settings are best-effort
  }
}

export function updateSettings(patch: Partial<Settings>): void {
  setSettings(patch);
  if (patch.uiScale !== undefined) applyUiScale(patch.uiScale);
  if (patch.roundedCorners !== undefined) applyRounded(patch.roundedCorners);
  if (patch.uiFont !== undefined || patch.diffFont !== undefined) applyFonts();
  if (patch.terminalFont !== undefined) warmFont(patch.terminalFont);
  persistSettings();
}

export function updateAgent(name: string, patch: Partial<AgentProfile>): void {
  setSettings("agents", (a) => a.name === name, patch);
  persistSettings();
}

export function addAgent(profile: AgentProfile): void {
  if (settings.agents.some((a) => a.name === profile.name)) return;
  setSettings("agents", (a) => [...a, profile]);
  persistSettings();
}

export function removeAgent(name: string): void {
  if (settings.agents.length <= 1) return;
  setSettings("agents", (a) => a.filter((x) => x.name !== name));
  if (settings.defaultAgent === name) setSettings("defaultAgent", settings.agents[0]!.name);
  persistSettings();
}

function profileFor(choice?: string | null): AgentProfile | undefined {
  const name = choice && choice.trim() ? choice.trim() : settings.defaultAgent;
  return settings.agents.find((a) => a.name === name);
}

function tokens(s: string): string[] {
  return s.split(/\s+/).filter(Boolean);
}

/** Full launch argv for a workspace's chosen agent, including yolo flags when
 * yolo mode is on. An unrecognized choice is treated as a raw custom command. */
export function agentCmdFor(choice?: string | null): string[] {
  const name = choice && choice.trim() ? choice.trim() : settings.defaultAgent;
  const p = profileFor(choice);
  if (!p) return tokens(name);
  return [...tokens(p.command), ...(settings.yoloMode ? p.yoloFlags : [])];
}

/** Vanilla launch (no yolo flags) used for the fast-exit fallback restart. */
export function agentVanillaCmdFor(choice?: string | null): string[] {
  const name = choice && choice.trim() ? choice.trim() : settings.defaultAgent;
  const p = profileFor(choice);
  if (!p) return tokens(name).slice(0, 1);
  return tokens(p.command).slice(0, 1);
}
