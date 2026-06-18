import { createStore } from "solid-js/store";
import type {
  ConnStatus,
  GitState,
  RepositorySummary,
  WorkspaceSummary,
} from "@conduit/shared";

export interface TerminalState {
  running: boolean;
  exitCode: number | null;
}

export interface AppState {
  conn: ConnStatus;
  repositories: RepositorySummary[];
  workspaces: WorkspaceSummary[];
  /** Full git detail per workspace, populated by WorkspaceGitUpdated. The
   * board/sidebar read the summary fields; the workspace screen reads this. */
  gitByWs: Record<string, GitState>;
  /** Liveness per terminal, keyed by termKey(id, kind, tabId). */
  terminals: Record<string, TerminalState>;
}

export const [store, setStore] = createStore<AppState>({
  conn: "connecting",
  repositories: [],
  workspaces: [],
  gitByWs: {},
  terminals: {},
});
