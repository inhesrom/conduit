import { createSignal } from "solid-js";

/** Terminal/git split: the terminal's height fraction, plus whether the git
 * pane is collapsed. Global (not per workspace) and persisted. */
const KEY_PCT = "conduit.ws.termPct";
const KEY_COLLAPSED = "conduit.ws.gitCollapsed";

function clamp(p: number): number {
  return Math.min(0.85, Math.max(0.2, p));
}

function readNum(key: string, def: number): number {
  try {
    const v = parseFloat(localStorage.getItem(key) ?? "");
    return Number.isFinite(v) ? clamp(v) : def;
  } catch {
    return def;
  }
}

const [termPct, setTermPctSig] = createSignal(readNum(KEY_PCT, 0.68));
const [gitCollapsed, setGitCollapsedSig] = createSignal(
  (() => {
    try {
      return localStorage.getItem(KEY_COLLAPSED) === "1";
    } catch {
      return false;
    }
  })(),
);
export { gitCollapsed, termPct };

export function setTermPct(p: number): void {
  const c = clamp(p);
  setTermPctSig(c);
  try {
    localStorage.setItem(KEY_PCT, String(c));
  } catch {
    // ignore
  }
}

export function toggleGitCollapsed(): void {
  const next = !gitCollapsed();
  setGitCollapsedSig(next);
  try {
    localStorage.setItem(KEY_COLLAPSED, next ? "1" : "0");
  } catch {
    // ignore
  }
}
