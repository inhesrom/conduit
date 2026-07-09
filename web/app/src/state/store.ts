import { createStore } from "solid-js/store";
import type {
  ChangedFile,
  ConnStatus,
  GitState,
  PullRequestDetails,
  PullRequestSummary,
  RepositorySummary,
  SavedCommand,
  WorkspaceSummary,
} from "@conduit/shared";

export interface TerminalState {
  running: boolean;
  exitCode: number | null;
}

export type PullRequestLoadStatus =
  | "idle"
  | "loading"
  | "loaded"
  | "candidates"
  | "none"
  | "setup"
  | "error";

export interface PullRequestState {
  status: PullRequestLoadStatus;
  details?: PullRequestDetails;
  candidates?: PullRequestSummary[];
  message?: string;
  failureCount: number;
  updatedAt?: number;
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
  /** Prompts awaiting delivery to a shell tab — a Diff Question's secondary
   * agent — keyed by `${wsId}/${tabId}`. Session-only, like pendingPrompt. */
  pendingTabPrompt: Record<string, string>;
  /** Prompt staged at create time; attaches to the next WorkspaceCreated
   * (creations are serial from the UI). */
  pendingCreatePrompt: string | null;
  /** Branch-vs-base file lists for review mode, per workspace. */
  reviewByWs: Record<string, { base: string; files: ChangedFile[] }>;
  /** GitHub pull request viewer data, per workspace. */
  pullRequestsByWs: Record<string, PullRequestState>;
  /** Pending shell-resurrection commands, keyed by `${wsId}/${tabId}`. */
  resurrection: Record<string, SavedCommand>;
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
  pendingTabPrompt: {},
  pendingCreatePrompt: null,
  reviewByWs: {},
  pullRequestsByWs: {},
  resurrection: {},
});

/** Clear all session-scoped state when switching sessions. Connection status
 * is left to the client's status callback. */
export function resetStore(): void {
  setStore({
    repositories: [],
    workspaces: [],
    gitByWs: {},
    terminals: {},
    diffByWs: {},
    commitFilesByWs: {},
    repoBranches: {},
    createProgress: null,
    pendingPrompt: {},
    pendingTabPrompt: {},
    pendingCreatePrompt: null,
    reviewByWs: {},
    pullRequestsByWs: {},
    resurrection: {},
  });
}
