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
 * exactly one (or a pinned one). The desktop window always shows the picker so
 * the user explicitly picks (or creates) a session first. */
export async function initSessions(): Promise<void> {
  await refreshSessions();
  const list = sessions();
  if (pinned() && list[0]) void selectSession(list[0].name);
  else if (!desktop() && list.length === 1) void selectSession(list[0]!.name);
}

/** Ensure a session daemon is running (resurrecting a stale one), via
 * `POST /api/sessions`. Returns an error message on failure, else null. The
 * server only lets the shared web build hit names already in the registry. */
async function ensureSession(name: string): Promise<string | null> {
  try {
    const res = await fetch("/api/sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name }),
    });
    if (!res.ok) return (await res.text()) || `failed (${res.status})`;
    const j = (await res.json()) as { ok: boolean; name?: string; error?: string };
    if (!j.ok || !j.name) return j.error ?? "failed to start session";
    return null;
  } catch (e) {
    return e instanceof Error ? e.message : "failed to start session";
  }
}

/** Create (or restart) a named session daemon, then attach to it. Desktop-only
 * for new names (the server rejects them on the shared web build). Returns an
 * error message on failure. */
export async function createSession(name: string): Promise<string | null> {
  const trimmed = name.trim();
  if (!trimmed) return "session name is required";
  const err = await ensureSession(trimmed);
  if (err) return err;
  await refreshSessions();
  return selectSession(trimmed);
}

/** Delete a session entirely (stop its daemon, drop it from the registry, and
 * remove its persisted workspaces/repositories), via `DELETE /api/sessions/:name`.
 * Desktop-only — the shared web build rejects it. If we delete the session
 * we're currently attached to, detach back to the picker. Returns an error
 * message on failure, else null. */
export async function deleteSession(name: string): Promise<string | null> {
  try {
    const res = await fetch(`/api/sessions/${encodeURIComponent(name)}`, { method: "DELETE" });
    if (!res.ok) return (await res.text()) || `failed (${res.status})`;
  } catch (e) {
    return e instanceof Error ? e.message : "failed to delete session";
  }
  if (currentSession() === name) {
    client.close();
    resetStore();
    setCurrentSession(null);
  }
  await refreshSessions();
  return null;
}

/** Attach to a session, resurrecting its daemon first if needed — mirrors the
 * TUI, which calls `ensure_session_running` on every attach (a live daemon
 * returns immediately; a stale one is respawned). Returns an error message on
 * failure, else null. A pinned session is managed by the server (an embedded
 * in-process core has no daemon to resurrect), so we attach without ensuring. */
export async function selectSession(name: string): Promise<string | null> {
  if (currentSession() === name) return null;
  if (!pinned()) {
    const err = await ensureSession(name);
    if (err) return err;
  }
  resetStore();
  setCurrentSession(name);
  client.connectTo(`${wsBase()}?session=${encodeURIComponent(name)}`);
  return null;
}
