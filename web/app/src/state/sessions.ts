import { createSignal } from "solid-js";
import { client } from "../client";
import { resetStore } from "./store";

/** The running conduit sessions the web server can attach to, and which one
 * we're currently driving. (Distinct from the auth "session" in session.ts.) */
const [sessions, setSessions] = createSignal<string[]>([]);
const [currentSession, setCurrentSession] = createSignal<string | null>(null);
const [pinned, setPinned] = createSignal(false);
const [loaded, setLoaded] = createSignal(false);
export { currentSession, loaded, pinned, sessions };

function wsBase(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return import.meta.env.VITE_CONDUIT_WS_URL ?? `${proto}://${location.host}/ws`;
}

export async function refreshSessions(): Promise<void> {
  try {
    const res = await fetch("/api/sessions");
    const j = (await res.json()) as { sessions: string[]; pinned: boolean };
    setSessions(j.sessions);
    setPinned(j.pinned);
  } catch {
    setSessions([]);
  } finally {
    setLoaded(true);
  }
}

/** Called once auth is cleared: load sessions and auto-attach when there's
 * exactly one (or a pinned one). */
export async function initSessions(): Promise<void> {
  await refreshSessions();
  const list = sessions();
  if (pinned() && list[0]) selectSession(list[0]);
  else if (list.length === 1) selectSession(list[0]!);
}

export function selectSession(name: string): void {
  if (currentSession() === name) return;
  resetStore();
  setCurrentSession(name);
  client.connectTo(`${wsBase()}?session=${encodeURIComponent(name)}`);
}
