import { reconcile } from "solid-js/store";
import { type AppEvent, termKey } from "@conduit/shared";
import { navigate } from "../router";
import { closeAppModal } from "./modals";
import { setStore, store } from "./store";
import { pushToast } from "./toasts";

/** The single reducer from the event stream into the store. Snapshot events
 * (RepositoryList / WorkspaceList) are full replacements reconciled by id, so
 * only changed rows actually update — and reconnect self-heals for free. */
export function applyEvent(e: AppEvent): void {
  switch (e.type) {
    case "RepositoryList":
      setStore("repositories", reconcile(e.items, { key: "id" }));
      break;

    case "WorkspaceList":
      setStore("workspaces", reconcile(e.items, { key: "id" }));
      break;

    case "WorkspaceGitUpdated":
      setStore("gitByWs", e.id, e.git);
      // Keep the summary fields the board/sidebar render in sync with the
      // fresh git detail.
      setStore(
        "workspaces",
        (w) => w.id === e.id,
        (w) => ({
          ...w,
          branch: e.git.branch,
          ahead: e.git.ahead,
          behind: e.git.behind,
          dirty_files: e.git.changed.length,
        }),
      );
      break;

    case "WorkspaceAttentionChanged":
      setStore("workspaces", (w) => w.id === e.id, "attention", e.level);
      break;

    case "WorkspaceReviewChanged":
      setStore("workspaces", (w) => w.id === e.id, "ready_for_review", e.ready);
      break;

    case "TerminalStarted":
      setStore("terminals", termKey(e.id, e.kind, e.tab_id), { running: true, exitCode: null });
      break;

    case "TerminalExited":
      setStore("terminals", termKey(e.id, e.kind, e.tab_id), {
        running: false,
        exitCode: e.code,
      });
      break;

    case "WorkspaceDiffUpdated":
      setStore("diffByWs", e.id, { file: e.file, diff: e.diff });
      break;

    case "CommitFilesLoaded":
      // Merge under the workspace key so the (possibly first) nested entry is
      // created reliably; an empty array still marks the commit as loaded.
      setStore("commitFilesByWs", e.id, (prev) => ({ ...prev, [e.hash]: e.files }));
      break;

    case "GitActionResult":
      if (e.message) pushToast(e.success ? "ok" : "error", e.message);
      break;

    case "Error":
      pushToast("error", e.message);
      break;

    case "RepoBranches":
      setStore("repoBranches", e.repo_id, { local: e.local, remote: e.remote });
      break;

    case "WorktreeCreateProgress":
      setStore("createProgress", { repoId: e.repo_id, stage: e.stage });
      break;

    case "BranchDiffFilesLoaded":
      setStore("reviewByWs", e.id, { base: e.base, files: e.files });
      break;

    case "PullRequestLoaded":
      setStore("pullRequestsByWs", e.id, {
        status: "loaded",
        details: e.details,
        candidates: undefined,
        message: undefined,
        failureCount: 0,
        updatedAt: Date.now(),
      });
      break;

    case "PullRequestCandidatesLoaded": {
      const prev = store.pullRequestsByWs[e.id];
      setStore("pullRequestsByWs", e.id, {
        status: "candidates",
        details: prev?.details,
        candidates: e.candidates,
        message: "Choose an open pull request for this repository.",
        failureCount: 0,
        updatedAt: Date.now(),
      });
      break;
    }

    case "PullRequestNotFound": {
      const prev = store.pullRequestsByWs[e.id];
      setStore("pullRequestsByWs", e.id, {
        status: "none",
        details: prev?.details,
        candidates: undefined,
        message: e.message,
        failureCount: (prev?.failureCount ?? 0) + 1,
        updatedAt: Date.now(),
      });
      break;
    }

    case "PullRequestSetupRequired": {
      const prev = store.pullRequestsByWs[e.id];
      setStore("pullRequestsByWs", e.id, {
        status: "setup",
        details: prev?.details,
        candidates: undefined,
        message: e.message,
        failureCount: (prev?.failureCount ?? 0) + 1,
        updatedAt: Date.now(),
      });
      break;
    }

    case "PullRequestMutationResult": {
      if (e.message) pushToast(e.success ? "ok" : "error", e.message);
      if (!e.success) {
        const prev = store.pullRequestsByWs[e.id];
        setStore("pullRequestsByWs", e.id, {
          status: "error",
          details: prev?.details,
          candidates: prev?.candidates,
          message: e.message,
          failureCount: (prev?.failureCount ?? 0) + 1,
          updatedAt: Date.now(),
        });
      }
      break;
    }

    case "ShellResurrectionChanged": {
      const k = `${e.id}/${e.tab_id}`;
      if (e.command) setStore("resurrection", k, e.command);
      else setStore("resurrection", k, undefined!);
      break;
    }

    case "WorkspaceCreated": {
      setStore("createProgress", null);
      if (store.pendingCreatePrompt) {
        setStore("pendingPrompt", e.id, store.pendingCreatePrompt);
        setStore("pendingCreatePrompt", null);
      }
      if (store.pendingCreateResume) {
        setStore("pendingResume", e.id, true);
        setStore("pendingCreateResume", false);
      }
      closeAppModal();
      navigate({ name: "workspace", id: e.id });
      break;
    }

    default:
      // Terminal, diff, git-action, branch, and progress events are handled
      // by later milestones (workspace screen, git layer, creation).
      break;
  }
}
