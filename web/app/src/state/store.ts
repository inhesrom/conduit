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
  /** The diff currently loaded for a workspace's diff pane. */
  diffByWs: Record<string, { file: string; diff: string }>;
  /** File lists for expanded commits, keyed by workspace then commit hash. */
  commitFilesByWs: Record<string, Record<string, string[]>>;
  /** Branch lists per repo for the create-workspace picker. */
  repoBranches: Record<string, { local: string[]; remote: string[] }>;
  /** Live stage text while a worktree is being created. */
  createProgress: { repoId: string; stage: string } | null;
  /** Initial agent prompts awaiting delivery, keyed by new workspace id. */
  pendingPrompt: Record<string, string>;
  /** Prompt staged at create time; attaches to the next WorkspaceCreated
   * (creations are serial from the UI). */
  pendingCreatePrompt: string | null;
}

export const [store, setStore] = createStore<AppState>({
  conn: "connecting",
  repositories: [],
  workspaces: [],
  gitByWs: {},
  terminals: {},
  diffByWs: {},
  commitFilesByWs: {},
  repoBranches: {},
  createProgress: null,
  pendingPrompt: {},
  pendingCreatePrompt: null,
});
