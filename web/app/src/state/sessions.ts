import { createSignal } from "solid-js";
import { client } from "../client";
import { resetStore } from "./store";

/** A conduit session the web server knows about, with its current liveness.
 * `running: false` means stale (dead daemon socket) and resurrectable. */
export type SessionInfo = { name: string; running: boolean };

/** The conduit sessions the web server can attach to, and which one we're
 * currently driving. (Distinct from the auth "session" in session.ts.) */
const [sessions, setSessions] = createSignal<SessionInfo[]>([]);
const [currentSession, setCurrentSession] = createSignal<string | null>(null);
const [pinned, setPinned] = createSignal(false);
const [loaded, setLoaded] = createSignal(false);
/** True when served by the native desktop window — it always shows the chooser
 * on startup (no single-session auto-attach) and can create/delete sessions. */
const [desktop, setDesktop] = createSignal(false);
export { currentSession, desktop, loaded, pinned, sessions };

function wsBase(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return import.meta.env.VITE_CONDUIT_WS_URL ?? `${proto}://${location.host}/ws`;
}

export async function refreshSessions(): Promise<void> {
  try {
    const res = await fetch("/api/sessions");
    const j = (await res.json()) as { sessions: SessionInfo[]; pinned: boolean; desktop?: boolean };
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
 * a pinned server. Unpinned web and desktop surfaces always show the chooser so
 * the user explicitly chooses a session first. */
export async function initSessions(): Promise<void> {
  await refreshSessions();
  const list = sessions();
  if (pinned() && list[0]) void selectSession(list[0].name);
}

/** Ensure a registered session daemon is running (resurrecting a stale one), via
 * `POST /api/sessions`. Returns an error message on failure, else null. The
 * server only lets the shared web build hit names already in the registry. */
async function ensureSession(name: string): Promise<string | null> {
  try {
    const res = await fetch("/api/sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name }),
    });
    const text = await res.text();
    const j = parseSessionResponse(text);
    if (!res.ok) return j?.error ?? (text || `failed (${res.status})`);
    if (!j) return "failed to start session";
    if (!j.ok || !j.name) return j.error ?? "failed to start session";
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : "failed to start session";
  }
}

function parseSessionResponse(text: string): { ok: boolean; name?: string; error?: string } | null {
  try {
    return JSON.parse(text) as { ok: boolean; name?: string; error?: string };
  } catch {
    return null;
  }
}

/** Create (or restart) a named session daemon, then attach to it. Desktop-only
 * for new names (the server rejects them on the shared web build). Returns an
 * error message on failure. */
export async function createSession(name: string): Promise<string | null> {
  const trimmed = name.trim();
  if (!trimmed) return "session name is required";
  if (!/^[A-Za-z0-9_-]+$/.test(trimmed)) return "use only letters, digits, '-' and '_'";
  const err = await ensureSession(trimmed);
  if (err) return err;
  await refreshSessions();
  return selectSession(trimmed);
}

/** Delete a session entirely (stop its daemon, drop it from the registry, and
 * remove its persisted Conduit state), via `DELETE /api/sessions/:name`.
 * Desktop-only — the shared web build rejects it. The attached session is
 * refused in-app. Returns an error message on failure, else null. */
export async function deleteSession(name: string): Promise<string | null> {
  if (currentSession() === name) return "cannot delete the attached session";
  try {
    const res = await fetch(`/api/sessions/${encodeURIComponent(name)}`, { method: "DELETE" });
    if (!res.ok) return (await res.text()) || `failed (${res.status})`;
  } catch (e) {
    return e instanceof Error ? e.message : "failed to delete session";
  }
  await refreshSessions();
  return null;
}

/** Attach to a session, resurrecting its daemon first if needed. Running
 * sessions connect directly; stale sessions are revived through the server.
 * Returns an error message on failure, else null. A pinned session is managed
 * by the server, so we attach without ensuring. */
export async function selectSession(name: string): Promise<string | null> {
  if (currentSession() === name) return null;
  const known = sessions().find((s) => s.name === name);
  if (!pinned() && !known?.running) {
    const err = await ensureSession(name);
    if (err) return err;
    await refreshSessions();
  }
  resetStore();
  setCurrentSession(name);
  client.connectTo(`${wsBase()}?session=${encodeURIComponent(name)}`);
  return null;
}
