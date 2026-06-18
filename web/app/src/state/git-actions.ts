import { client } from "../client";

/** Thin wrappers over the git Commands so panels don't repeat client.send.
 * Results come back asynchronously as GitActionResult (→ toast) and a fresh
 * WorkspaceGitUpdated. */
export const git = {
  refresh: (id: string) => client.send({ RefreshGit: { id } }),

  stageFile: (id: string, file: string) => client.send({ GitStageFile: { id, file } }),
  unstageFile: (id: string, file: string) => client.send({ GitUnstageFile: { id, file } }),
  stageAll: (id: string) => client.send({ GitStageAll: { id } }),
  unstageAll: (id: string) => client.send({ GitUnstageAll: { id } }),
  commit: (id: string, message: string) => client.send({ GitCommit: { id, message } }),
  discardFile: (id: string, file: string) => client.send({ GitDiscardFile: { id, file } }),
  discardAll: (id: string) => client.send({ GitDiscardAll: { id } }),

  stash: (id: string, message: string | null) => client.send({ GitStash: { id, message } }),
  stashAll: (id: string) => client.send({ GitStashAll: { id } }),
  stashPop: (id: string) => client.send({ GitStashPullPop: { id } }),

  push: (id: string) => client.send({ GitPush: { id } }),
  pull: (id: string) => client.send({ GitPull: { id } }),
  fetch: (id: string) => client.send({ GitFetch: { id } }),

  checkoutBranch: (id: string, branch: string) => client.send({ GitCheckoutBranch: { id, branch } }),
  checkoutRemote: (id: string, remoteBranch: string, localName: string) =>
    client.send({ GitCheckoutRemoteBranch: { id, remote_branch: remoteBranch, local_name: localName } }),
  createBranch: (id: string, branch: string) => client.send({ GitCreateBranch: { id, branch } }),
  deleteLocal: (id: string, branch: string) => client.send({ GitDeleteLocalBranch: { id, branch } }),
  deleteRemote: (id: string, remote: string, branch: string) =>
    client.send({ GitDeleteRemoteBranch: { id, remote, branch } }),

  loadDiff: (id: string, file: string) => client.send({ LoadDiff: { id, file } }),
  loadCommitFiles: (id: string, hash: string) => client.send({ LoadCommitFiles: { id, hash } }),
  loadCommitFileDiff: (id: string, hash: string, file: string) =>
    client.send({ LoadCommitFileDiff: { id, hash, file } }),
  loadCommitDiff: (id: string, hash: string) => client.send({ LoadCommitDiff: { id, hash } }),

  // Review
  loadBranchDiff: (id: string) => client.send({ LoadBranchDiff: { id } }),
  loadBranchFileDiff: (id: string, file: string) => client.send({ LoadBranchFileDiff: { id, file } }),
  setReadyForReview: (id: string, ready: boolean) => client.send({ SetReadyForReview: { id, ready } }),
  openPullRequest: (id: string) => client.send({ OpenPullRequest: { id } }),

  runCommand: (id: string, command: string) => client.send({ RunWorkspaceCommand: { id, command } }),
};
