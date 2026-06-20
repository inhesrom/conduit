import { createSignal } from "solid-js";
import { initSessions } from "./sessions";

export type AuthState = "checking" | "needed" | "ready";

const [authState, setAuthState] = createSignal<AuthState>("checking");
export { authState };

let started = false;
function startOnce(): void {
  if (started) return;
  started = true;
  // Auth cleared — load the session list and attach (the WS connects once a
  // session is selected).
  void initSessions();
}

/** Decide whether to show the login screen, then connect when cleared. */
export async function checkSession(): Promise<void> {
  try {
    const res = await fetch("/api/session");
    const j = (await res.json()) as { auth_required: boolean; authenticated: boolean };
    if (!j.auth_required || j.authenticated) {
      setAuthState("ready");
      startOnce();
    } else {
      setAuthState("needed");
    }
  } catch {
    // No session endpoint reachable (e.g. a bare dev server) — just connect.
    setAuthState("ready");
    startOnce();
  }
}

export async function login(password: string): Promise<boolean> {
  const res = await fetch("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (!res.ok) return false;
  setAuthState("ready");
  startOnce();
  return true;
}
