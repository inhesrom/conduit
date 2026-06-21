import { createSignal } from "solid-js";
import { client } from "../client";
import { resetStore } from "./store";

/** The running conduit sessions the web server can attach to, and which one
 * we're currently driving. (Distinct from the auth "session" in session.ts.) */
const [sessions, setSessions] = createSignal<string[]>([]);
const [currentSession, setCurrentSession] = createSignal<string | null>(null);
const [pinned, setPinned] = createSignal(false);
const [loaded, setLoaded] = createSignal(false);
/** True when served by the native desktop window — it always shows the picker
 * on startup (no single-session auto-attach) and can create sessions. */
const [desktop, setDesktop] = createSignal(false);
export { currentSession, desktop, loaded, pinned, sessions };

function wsBase(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return import.meta.env.VITE_CONDUIT_WS_URL ?? `${proto}://${location.host}/ws`;
}

export async function refreshSessions(): Promise<void> {
  try {
    const res = await fetch("/api/sessions");
    const j = (await res.json()) as { sessions: string[]; pinned: boolean; desktop?: boolean };
    setSessions(j.sessions);
    setPinned(j.pinned);
    setDesktop(j.desktop ?? false);
  } catch {
    setSessions([]);
  } finally {
    setLoaded(true);
  }
}

/** Called once auth is cleared: load sessions and auto-attach when there's
 * exactly one (or a pinned one). The desktop window always shows the picker so
 * the user explicitly picks (or creates) a session first. */
export async function initSessions(): Promise<void> {
  await refreshSessions();
  const list = sessions();
  if (pinned() && list[0]) selectSession(list[0]);
  else if (!desktop() && list.length === 1) selectSession(list[0]!);
}

/** Create (or restart) a named session daemon, then attach to it. Desktop-only
 * (the server rejects it elsewhere). Returns an error message on failure. */
export async function createSession(name: string): Promise<string | null> {
  const trimmed = name.trim();
  if (!trimmed) return "session name is required";
  try {
    const res = await fetch("/api/sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: trimmed }),
    });
    if (!res.ok) return (await res.text()) || `failed (${res.status})`;
    const j = (await res.json()) as { ok: boolean; name?: string; error?: string };
    if (!j.ok || !j.name) return j.error ?? "failed to create session";
    await refreshSessions();
    selectSession(j.name);
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : "failed to create session";
  }
}

export function selectSession(name: string): void {
  if (currentSession() === name) return;
  resetStore();
  setCurrentSession(name);
  client.connectTo(`${wsBase()}?session=${encodeURIComponent(name)}`);
}
