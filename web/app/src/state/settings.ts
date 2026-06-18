import { createStore } from "solid-js/store";

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

export function persistSettings(): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // ignore quota/availability errors — settings are best-effort
  }
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
