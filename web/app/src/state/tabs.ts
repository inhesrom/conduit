import { createStore } from "solid-js/store";

/** Per-workspace shell tabs. The agent tab is implicit and always present;
 * only user-created shells are tracked here and persisted per-browser. The
 * daemon keys shells by tab_id, so these ids ("shell", "shell-2", …) are what
 * StartTerminal / TerminalOutput carry. */
export interface ShellTab {
  id: string;
  title: string;
}

const [shells, setShells] = createStore<Record<string, ShellTab[]>>({});

const keyFor = (wsId: string) => `conduit.tabs.${wsId}`;

function ensure(wsId: string): void {
  if (shells[wsId]) return;
  let initial: ShellTab[] = [];
  try {
    const raw = localStorage.getItem(keyFor(wsId));
    if (raw) initial = JSON.parse(raw) as ShellTab[];
  } catch {
    initial = [];
  }
  setShells(wsId, initial);
}

function persist(wsId: string): void {
  try {
    localStorage.setItem(keyFor(wsId), JSON.stringify(shells[wsId] ?? []));
  } catch {
    // best-effort
  }
}

export function shellTabs(wsId: string): ShellTab[] {
  ensure(wsId);
  return shells[wsId]!;
}

function nextShellId(existing: ShellTab[]): { id: string; n: number } {
  const ids = new Set(existing.map((t) => t.id));
  let n = 1;
  let id = "shell";
  while (ids.has(id)) {
    n += 1;
    id = `shell-${n}`;
  }
  return { id, n };
}

export function createShell(wsId: string): ShellTab {
  ensure(wsId);
  const { id, n } = nextShellId(shells[wsId]!);
  const tab: ShellTab = { id, title: `shell ${n}` };
  setShells(wsId, [...shells[wsId]!, tab]);
  persist(wsId);
  return tab;
}

export function removeShell(wsId: string, id: string): void {
  ensure(wsId);
  setShells(wsId, (list) => list.filter((t) => t.id !== id));
  persist(wsId);
}

export function renameShell(wsId: string, id: string, title: string): void {
  ensure(wsId);
  setShells(wsId, (t) => t.id === id, "title", title.trim() || id);
  persist(wsId);
}
